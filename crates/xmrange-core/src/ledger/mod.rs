//! The Ledger (spec §4): SQLite encrypted with SQLCipher (doctrine §3). A raw 256-bit key from
//! the OS RNG (no passphrase KDF); WAL for crash safety; incremental auto-vacuum so a migration
//! that drops data gives the space back; temp storage in memory so no plaintext temp file is ever
//! written. It keeps trials only: the 1 Hz samples left in schema v3 (ruling P5-O1).
use crate::secrets::SecretError;
use rusqlite::{Connection, ErrorCode, OpenFlags};
use sha3::{Digest, Sha3_256};
use std::fmt;
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

mod open;
mod payouts;
mod schema;
mod trials;
mod writer;
pub use open::{open_default, open_with, quarantine, AppKeyFile, KeySource, Opened};
pub use payouts::PayoutTotals;
pub use schema::SCHEMA_VERSION;
/// The SQLCipher major format every Ledger is written in.
pub const CIPHER_COMPATIBILITY: u32 = 4;
/// How long a statement waits on another connection's write lock (the app and a CLI `record`
/// can share one Ledger) before it reports `LedgerError::Busy`.
pub const BUSY_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
pub use trials::{curve_view, trial_label, trial_views, CurvePoint, CurveView, TrialConfig, TrialRecord, TrialView, TrialsView};
pub use writer::{AskError, LedgerHandle, LedgerWriter, CLOSE_WAIT, QUEUE_DEPTH};

/// The Ledger's raw 256-bit key. Debug never shows it, and this value's own bytes are zeroed on
/// drop. That wipe is best-effort (F15): hex copies of the key also live in the key file's own
/// bytes, in the `PRAGMA key` string handed to SQLCipher, and in whatever read the key in.
pub struct LedgerKey([u8; 32]);

impl LedgerKey {
    pub fn generate() -> Self {
        let mut b = [0u8; 32];
        getrandom::getrandom(&mut b).expect("OS random number generator unavailable");
        Self(b)
    }
    pub fn from_hex(s: &str) -> Result<Self, SecretError> {
        let s = s.trim();
        if s.len() != 64 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(SecretError::Corrupt("the Ledger key is not 64 hex characters".into()));
        }
        let mut b = [0u8; 32];
        for (i, pair) in s.as_bytes().chunks(2).enumerate() {
            b[i] = u8::from_str_radix(std::str::from_utf8(pair).expect("ascii"), 16).expect("hex checked");
        }
        Ok(Self(b))
    }
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
    /// From the raw bytes `secrets::key_file::KeyFile` reads or generates. `open::AppKeyFile` is
    /// the only caller: every other `KeySource` goes through hex.
    pub(crate) fn from_bytes(b: [u8; 32]) -> Self {
        Self(b)
    }
    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for LedgerKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LedgerKey(…)")
    }
}

impl Drop for LedgerKey {
    fn drop(&mut self) {
        for b in self.0.iter_mut() {
            // SAFETY: `b` is a valid, aligned &mut u8 into our own array.
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

#[derive(Debug)]
pub enum LedgerError {
    /// The key doesn't open this file, or it isn't a Ledger. SQLCipher can't tell these apart.
    WrongKeyOrNotALedger,
    /// Written by a newer XMRange. Left untouched.
    TooNew { found: u32, known: u32 },
    Corrupt(String),
    /// Another connection held the write lock past `BUSY_WAIT` (SQLITE_BUSY/LOCKED), or the write
    /// itself ran out of room (SQLITE_FULL, final review M2): transient either way. The trial it
    /// refused is reported to its caller, and the next write tries again (T7 review m5).
    Busy(String),
    Io(String),
    Sql(String),
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LedgerError::WrongKeyOrNotALedger => write!(f, "the key does not open this Ledger"),
            LedgerError::TooNew { found, known } => write!(f, "schema {found} is newer than this build's {known}"),
            LedgerError::Corrupt(m) => write!(f, "the Ledger is damaged ({m})"),
            LedgerError::Busy(m) => write!(f, "the Ledger is busy ({m})"),
            LedgerError::Io(m) | LedgerError::Sql(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for LedgerError {}

pub(crate) fn sql(e: rusqlite::Error) -> LedgerError {
    match &e {
        rusqlite::Error::SqliteFailure(f, m) if matches!(f.code, ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => {
            LedgerError::Corrupt(m.clone().unwrap_or_else(|| e.to_string()))
        }
        rusqlite::Error::SqliteFailure(f, _) if matches!(f.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked | ErrorCode::DiskFull) => {
            LedgerError::Busy(e.to_string())
        }
        _ => LedgerError::Sql(e.to_string()),
    }
}

fn io(e: std::io::Error) -> LedgerError {
    LedgerError::Io(e.to_string())
}

pub struct Ledger {
    pub(crate) conn: Connection,
    path: PathBuf,
    /// The wallet tag's salt: SHA3-256("xmrange wallet tag v1" ‖ key). It derives from the Ledger
    /// key (P3-R4 amendment), so no table has to store it; a new key (a new Ledger) means a new salt.
    pub(crate) tag_salt: [u8; 32],
}

impl fmt::Debug for Ledger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ledger").field("path", &self.path).finish()
    }
}

impl Ledger {
    pub fn open(path: &Path, key: &LedgerKey) -> Result<Ledger, LedgerError> {
        Self::open_at(path, key, SCHEMA_VERSION)
    }

    /// `open`, migrating only as far as `target` (tests build older Ledgers with it).
    pub(crate) fn open_at(path: &Path, key: &LedgerKey, target: u32) -> Result<Ledger, LedgerError> {
        if !path.exists() {
            // Made 0600 before SQLite sees it: SQLite gives -wal and -shm the main file's mode.
            OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).map_err(io)?;
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut conn = Connection::open_with_flags(path, flags).map_err(sql)?;
        conn.busy_timeout(BUSY_WAIT).map_err(sql)?;
        conn.pragma_update(None, "key", format!("x'{}'", key.to_hex())).map_err(sql)?;
        // Pin the SQLCipher 4 format (the bundled 4.6.1's own defaults: 4096-byte pages, HMAC-SHA512,
        // PBKDF2-HMAC-SHA512 × 256000), so a future SQLCipher with other defaults can't make every
        // valid Ledger read as "not a Ledger" and be moved aside (Task 8 fix round 1, m6). It must
        // come after `key` and before the first read.
        conn.pragma_update(None, "cipher_compatibility", CIPHER_COMPATIBILITY).map_err(sql)?;
        // The first read decrypts page 1: a wrong key, or a file that isn't a Ledger, fails here,
        // before anything is written.
        match conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)) {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(f, _)) if f.code == ErrorCode::NotADatabase => {
                return Err(LedgerError::WrongKeyOrNotALedger)
            }
            Err(e) => return Err(sql(e)),
        }
        let found = schema::user_version(&conn)?;
        let known = SCHEMA_VERSION;
        if found > known {
            return Err(LedgerError::TooNew { found, known }); // before any pragma writes the header
        }
        conn.pragma_update(None, "temp_store", 2).map_err(sql)?; // MEMORY: no plaintext temp files
        if found == 0 {
            conn.pragma_update(None, "auto_vacuum", 2).map_err(sql)?; // INCREMENTAL; only before the first table
        }
        let mode: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0)).map_err(sql)?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(LedgerError::Sql(format!("journal_mode stayed {mode}")));
        }
        conn.pragma_update(None, "synchronous", 1).map_err(sql)?; // NORMAL: durable across an app crash in WAL
        if schema::migrate_to(&mut conn, target)? {
            reclaim(&conn)?; // v3 dropped the samples: their pages go back to the file system
        }
        let tag_salt: [u8; 32] = Sha3_256::new().chain_update(b"xmrange wallet tag v1").chain_update(key.0).finalize().into();
        Ok(Ledger { conn, path: path.to_path_buf(), tag_salt })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn schema_version(&self) -> Result<u32, LedgerError> {
        schema::user_version(&self.conn)
    }
    pub fn auto_vacuum(&self) -> Result<i64, LedgerError> {
        self.conn.pragma_query_value(None, "auto_vacuum", |r| r.get(0)).map_err(sql)
    }
    /// The Ledger's size: its pages times the page size.
    pub fn file_bytes(&self) -> Result<u64, LedgerError> {
        let pages: i64 = self.conn.pragma_query_value(None, "page_count", |r| r.get(0)).map_err(sql)?;
        // SQLCipher hijacks `PRAGMA page_size` on a keyed connection: it reports the cipher's page
        // size as `cipher_page_size`, and as TEXT rather than an integer, so it can't be read as an
        // i64 the way `page_count` can.
        let size: i64 = self
            .conn
            .query_row("PRAGMA page_size", [], |r| match r.get_ref(0)? {
                rusqlite::types::ValueRef::Integer(i) => Ok(i),
                rusqlite::types::ValueRef::Text(t) => {
                    let s = std::str::from_utf8(t).unwrap_or("0");
                    Ok(s.parse().unwrap_or(0))
                }
                _ => Ok(0),
            })
            .map_err(sql)?;
        Ok((pages * size) as u64)
    }
    pub fn cipher_version(&self) -> Result<String, LedgerError> {
        self.conn.pragma_query_value(None, "cipher_version", |r| r.get(0)).map_err(sql)
    }
    pub fn cipher_provider(&self) -> Result<String, LedgerError> {
        self.conn.pragma_query_value(None, "cipher_provider", |r| r.get(0)).map_err(sql)
    }
    /// SQLite's `integrity_check` and SQLCipher's per-page HMAC check, as one word: "ok", or
    /// "damaged (<n> problem(s))". Never row contents.
    pub fn integrity(&self) -> Result<String, LedgerError> {
        let mut st = self.conn.prepare("PRAGMA integrity_check").map_err(sql)?;
        let sqlite: Vec<String> = st.query_map([], |r| r.get(0)).map_err(sql)?.collect::<Result<_, _>>().map_err(sql)?;
        let sqlite_bad = if sqlite == ["ok"] { 0 } else { sqlite.len() };
        let mut st = self.conn.prepare("PRAGMA cipher_integrity_check").map_err(sql)?;
        let cipher_bad = st.query_map([], |_| Ok(())).map_err(sql)?.count();
        Ok(match sqlite_bad + cipher_bad {
            0 => "ok".to_string(),
            n => format!("damaged ({n} problem(s))"),
        })
    }
}

/// auto_vacuum = INCREMENTAL: hands the free pages back to the file system. SQLite emits one result
/// row per page it frees (F1), so the pragma is stepped to completion; a single `execute` step
/// would free one page and leave the rest on the freelist.
fn reclaim(conn: &Connection) -> Result<(), LedgerError> {
    let mut stmt = conn.prepare("PRAGMA incremental_vacuum").map_err(sql)?;
    let mut rows = stmt.query([]).map_err(sql)?;
    while rows.next().map_err(sql)?.is_some() {}
    Ok(())
}

#[cfg(test)]
pub(crate) fn test_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    // Inside the repo's target/ (Rule #1: never system temp), as settings.rs's tests do.
    let d = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/test-scratch")
        .join(format!("ledger-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[cfg(test)]
pub(crate) fn open_test(tag: &str) -> (Ledger, PathBuf, LedgerKey) {
    let p = test_dir(tag).join("l.sqlite");
    let k = LedgerKey::generate();
    (Ledger::open(&p, &k).unwrap(), p, k)
}

#[cfg(test)]
pub(crate) fn assert_no_plaintext(path: &Path, needles: &[&str]) {
    let bytes = std::fs::read(path).unwrap();
    for n in needles {
        assert!(!bytes.windows(n.len()).any(|w| w == n.as_bytes()), "{} holds plaintext {n:?}", path.display());
    }
}

/// A bare trials row whose label is `text`: the Task 4 tests' canary (Task 6 brings real trials).
#[cfg(test)]
pub(crate) fn put_canary(l: &Ledger, text: &str) {
    l.conn
        .execute(
            "INSERT INTO trials (t_start, t_end, label, source, config, settled, settle_s, partial, context_start, context_end)
             VALUES (0, 0, ?1, 'test', '{}', 0, 0, 0, '{}', '{}')",
            [text],
        )
        .unwrap();
}

#[cfg(test)]
pub(crate) fn read_canary(l: &Ledger) -> Option<String> {
    l.conn.query_row("SELECT label FROM trials ORDER BY id DESC LIMIT 1", [], |r| r.get(0)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn sqlcipher_is_compiled_in_with_apple_crypto() {
        let (l, _, _) = open_test("cipher");
        assert!(!l.cipher_version().unwrap().is_empty(), "SQLCipher, not plain SQLite");
        assert_eq!(l.cipher_provider().unwrap(), "commoncrypto", "Apple CommonCrypto: no OpenSSL in the build");
    }

    /// A pragma's value as text: SQLCipher returns some of its settings as TEXT, some as INTEGER.
    fn pragma_text(conn: &Connection, name: &str) -> String {
        match conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get::<_, rusqlite::types::Value>(0)).unwrap() {
            rusqlite::types::Value::Integer(i) => i.to_string(),
            rusqlite::types::Value::Text(t) => t,
            other => format!("{other:?}"),
        }
    }

    /// m6: the SQLCipher 4 settings are in force on every Ledger connection, whatever a future
    /// SQLCipher's defaults are. The values are SQLCipher 4's documented compatibility-4 set.
    #[test]
    fn the_cipher_format_is_pinned_to_sqlcipher_4() {
        let (l, _, _) = open_test("compat");
        assert_eq!(CIPHER_COMPATIBILITY, 4);
        assert_eq!(pragma_text(&l.conn, "cipher_page_size"), "4096");
        assert_eq!(pragma_text(&l.conn, "kdf_iter"), "256000");
        assert_eq!(pragma_text(&l.conn, "cipher_hmac_algorithm"), "HMAC_SHA512");
        assert_eq!(pragma_text(&l.conn, "cipher_kdf_algorithm"), "PBKDF2_HMAC_SHA512");
    }

    /// `cipher_compatibility` can't be read back, so this shows the pin is live in the bundled
    /// SQLCipher: it overrides a connection's other settings, both ways.
    #[test]
    fn cipher_compatibility_overrides_a_connections_settings() {
        let conn = Connection::open(test_dir("compat-live").join("raw.sqlite")).unwrap();
        conn.pragma_update(None, "key", format!("x'{}'", LedgerKey::generate().to_hex())).unwrap();
        conn.pragma_update(None, "cipher_page_size", 8192).unwrap();
        conn.pragma_update(None, "cipher_compatibility", 3).unwrap();
        assert_eq!(pragma_text(&conn, "cipher_page_size"), "1024");
        conn.pragma_update(None, "cipher_compatibility", CIPHER_COMPATIBILITY).unwrap();
        assert_eq!(pragma_text(&conn, "cipher_page_size"), "4096");
    }

    #[test]
    fn a_new_ledger_is_private_and_current() {
        let (l, p, _) = open_test("new");
        assert_eq!(l.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(l.auto_vacuum().unwrap(), 2, "incremental: a dropped table gives its space back");
    }

    #[test]
    fn wrong_key_is_refused_not_overwritten() {
        let (l, p, k) = open_test("wrongkey");
        put_canary(&l, "kept");
        drop(l);
        let before = fs::read(&p).unwrap();
        assert!(matches!(Ledger::open(&p, &LedgerKey::generate()), Err(LedgerError::WrongKeyOrNotALedger)));
        assert_eq!(fs::read(&p).unwrap(), before, "a failed open writes nothing");
        assert_eq!(read_canary(&Ledger::open(&p, &k).unwrap()).as_deref(), Some("kept"));
    }

    #[test]
    fn plain_sqlite_and_garbage_are_not_ledgers() {
        let dir = test_dir("notledger");
        let plain = dir.join("plain.sqlite");
        rusqlite::Connection::open(&plain).unwrap().execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1);").unwrap();
        assert!(matches!(Ledger::open(&plain, &LedgerKey::generate()), Err(LedgerError::WrongKeyOrNotALedger)));
        let junk = dir.join("junk.sqlite");
        fs::write(&junk, vec![0x5a; 8192]).unwrap();
        assert!(matches!(Ledger::open(&junk, &LedgerKey::generate()), Err(LedgerError::WrongKeyOrNotALedger)));
    }

    #[test]
    fn a_newer_schema_is_left_untouched() {
        let (l, p, k) = open_test("toonew");
        l.conn.pragma_update(None, "user_version", 99).unwrap();
        drop(l);
        let before = fs::read(&p).unwrap();
        match Ledger::open(&p, &k) {
            Err(LedgerError::TooNew { found: 99, known }) => assert_eq!(known, SCHEMA_VERSION),
            other => panic!("expected TooNew, got {other:?}"),
        }
        assert_eq!(fs::read(&p).unwrap(), before, "a newer Ledger is never migrated or rewritten");
    }

    fn tables(l: &Ledger) -> Vec<String> {
        let mut st = l.conn.prepare("SELECT name FROM sqlite_master WHERE type IN ('table', 'index') ORDER BY name").unwrap();
        st.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
    }

    #[test]
    fn migrations_run_once_and_leave_trials_and_payouts() {
        let (l, p, k) = open_test("migrate");
        drop(l);
        let l = Ledger::open(&p, &k).unwrap(); // a second open re-runs nothing
        assert_eq!(
            tables(&l),
            ["payouts", "sqlite_autoindex_payouts_1", "trials", "trials_by_t_end"],
            "spec §4 minus settings (P3-R4) and samples (P5-O1), plus p2pool payouts (v4)"
        );
    }

    /// A Ledger at `version` holding a trial and about a day of the old 1 Hz samples.
    fn old_ledger(tag: &str, version: u32) -> (PathBuf, LedgerKey) {
        let p = test_dir(tag).join("l.sqlite");
        let k = LedgerKey::generate();
        let l = Ledger::open_at(&p, &k, version).unwrap();
        assert_eq!(l.schema_version().unwrap(), version);
        put_canary(&l, "kept-trial");
        l.conn
            .execute_batch(
                "WITH RECURSIVE s(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM s WHERE i < 86399)
                 INSERT INTO samples (t, span_s, n, state, threads, hs10, sys_w, cpu_w, ram_w, temp_c, p_mhz, e_mhz, p_active, e_active)
                 SELECT 1790000000 + i, 1, 1, 'mining', 8, 1800.0, 9.0, 4.0, 0.8, 55.0, 1900, 2500, 1.0, 0.3 FROM s",
            )
            .unwrap();
        drop(l);
        (p, k)
    }

    /// P5-O1: v1 → v3 and v2 → v3 drop the samples (and their index), keep every trial, and give
    /// the freed pages back: the file shrinks and no page is left on the freelist.
    #[test]
    fn v3_drops_the_samples_and_reclaims_their_space_from_v1_and_v2() {
        for from in [1, 2] {
            let (p, k) = old_ledger(&format!("v{from}to3"), from);
            let before = Ledger::open_at(&p, &k, from).unwrap().file_bytes().unwrap();
            let l = Ledger::open_at(&p, &k, 3).unwrap();
            assert_eq!(l.schema_version().unwrap(), 3, "v{from}");
            assert_eq!(tables(&l), ["trials", "trials_by_t_end"], "v{from}: the samples table and its index are gone");
            assert_eq!(read_canary(&l).as_deref(), Some("kept-trial"), "v{from}: trials are kept");
            let cols: i64 = l.conn.query_row("SELECT count(*) FROM pragma_table_info('trials') WHERE name = 'thermal_max'", [], |r| r.get(0)).unwrap();
            assert_eq!(cols, 1, "v{from}: v2's trial columns are there too");
            let free: i64 = l.conn.pragma_query_value(None, "freelist_count", |r| r.get(0)).unwrap();
            assert_eq!(free, 0, "v{from}: every freed page went back to the file system");
            let after = l.file_bytes().unwrap();
            assert!(after * 10 < before, "v{from}: the Ledger shrank from {before} to {after} bytes");
            assert_eq!(l.integrity().unwrap(), "ok");
            drop(l);
            let bytes = std::fs::metadata(&p).unwrap().len();
            assert!(bytes * 10 < before, "v{from}: the file on disk shrank too: {bytes} bytes");
        }
    }

    #[test]
    fn schema_version_matches_the_migration_list() {
        assert_eq!(SCHEMA_VERSION as usize, schema::MIGRATIONS.len());
    }

    #[test]
    fn nothing_readable_on_disk() {
        let (l, p, _) = open_test("plaintext");
        put_canary(&l, "PLAINTEXT-CANARY-7f3e");
        let needles = ["PLAINTEXT-CANARY-7f3e", "SQLite format 3", "CREATE TABLE", "wallet_tag"];
        // Connection still open: the WAL holds the newest pages, so scan every file SQLite made.
        let dir = p.parent().unwrap();
        let files: Vec<_> = fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
        assert!(files.iter().any(|f| f.to_string_lossy().ends_with("-wal")), "the WAL is scanned too");
        assert!(files.iter().any(|f| f.to_string_lossy().ends_with("-shm")), "the shared-memory index file is scanned too");
        for f in &files {
            assert_no_plaintext(f, &needles);
        }
        drop(l); // the checkpoint moves the pages into the main file
        assert_no_plaintext(&p, &needles);
    }

    #[test]
    fn a_whole_ledger_reports_integrity_ok() {
        let (l, _, _) = open_test("integrity");
        put_canary(&l, "x");
        assert_eq!(l.integrity().unwrap(), "ok");
    }

    /// Final review M2: SQLITE_FULL (a full disk, or `max_page_count` standing in for one here)
    /// maps to `LedgerError::Busy`, the same transient error a held lock gives: the trial is
    /// refused to its caller, and a later one tries again.
    #[test]
    fn disk_full_maps_to_a_transient_busy_error() {
        let (mut l, _, _) = open_test("full");
        let pages: i64 = l.conn.query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap();
        l.conn.pragma_update(None, "max_page_count", pages).unwrap(); // no room left to grow
        // A trial whose label outgrows the schema's existing pages needs a page beyond the cap,
        // which is the SQLITE_FULL this pragma stands in for.
        let mut rec = TrialRecord::from_result(&crate::trial::TrialResult {
            threads: 5, settled: true, settle_s: 60, partial: true, score: None, ticks: vec![],
            context_start: Default::default(), context_end: Default::default(),
        }, "t", "cli", None, false);
        rec.label = "x".repeat(64 * 1024);
        let err = l.insert_trial(&rec).unwrap_err();
        assert!(matches!(err, LedgerError::Busy(_)), "{err:?}");
    }

    #[test]
    fn the_tag_salt_follows_the_key() {
        let (l, p, k) = open_test("salt");
        let salt = l.tag_salt;
        drop(l);
        assert_eq!(Ledger::open(&p, &k).unwrap().tag_salt, salt, "a reopen keeps the wallet tags stable");
        let (other, _, _) = open_test("salt2");
        assert_ne!(other.tag_salt, salt, "a new key means a new salt");
    }

    #[test]
    fn key_hex_roundtrips_and_debug_hides_it() {
        let k = LedgerKey::generate();
        let hex = k.to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(LedgerKey::from_hex(&format!("{hex}\n")).unwrap().to_hex(), hex);
        assert!(!format!("{k:?}").contains(&hex[..8]));
        for bad in ["", "abc", &"g".repeat(64), &"a".repeat(63)] {
            assert!(LedgerKey::from_hex(bad).is_err(), "{bad:?}");
        }
    }
}
