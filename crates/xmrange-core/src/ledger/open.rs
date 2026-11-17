//! Opening the Ledger with the build's key: one file (`paths::key_path()`; `dev.key` in dev
//! builds, `app.key` in release — same code path, only the data dir differs). A Ledger that can't
//! be read is moved aside, never deleted. A key that can't be read is not a lost key: it never
//! makes a new key and never moves the Ledger (pre-flight F4: one refusal means no history this
//! session, never a second attempt).
use super::{Ledger, LedgerError, LedgerKey};
use crate::secrets::{key_file::KeyFile, SecretError};
use std::fs::{self, File, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Where the Ledger key is kept.
pub trait KeySource {
    /// `Ok(None)`: no key was ever kept here. Any `Err` is not "no key".
    fn load(&self) -> Result<Option<LedgerKey>, SecretError>;
    /// Keeps a new key. Never overwrites an existing one.
    fn store(&self, key: &LedgerKey) -> Result<(), SecretError>;
}

/// The Ledger key in the app's one key file (`secrets::key_file::KeyFile` owns its format and
/// atomic write). `dev.key` in dev builds, `app.key` in release — one code path either way (owner
/// decision 2026-09-28: no OS credential store, ever, so opening a Ledger never prompts).
pub struct AppKeyFile(KeyFile);

impl AppKeyFile {
    pub fn new(path: PathBuf) -> Self {
        Self(KeyFile::new(path))
    }
}

impl KeySource for AppKeyFile {
    fn load(&self) -> Result<Option<LedgerKey>, SecretError> {
        Ok(self.0.load()?.map(LedgerKey::from_bytes))
    }
    fn store(&self, key: &LedgerKey) -> Result<(), SecretError> {
        self.0.store(key.as_bytes())
    }
}

pub struct Opened {
    pub ledger: Option<Ledger>,
    /// One plain-words line for the tape, when something needs saying.
    pub note: Option<String>,
}

impl Opened {
    fn without(note: String) -> Self {
        Opened { ledger: None, note: Some(note) }
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `<path>` with `suffix` appended to its file name: `-wal`, `-shm`.
fn side(path: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{suffix}", path.display()))
}

/// How files are moved; tests swap in a failing one (fix round 1, I1).
type Rename = fn(&Path, &Path) -> std::io::Result<()>;

fn os_rename(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

/// Moves an unreadable Ledger aside to `<name>.unreadable-<now>`, its `-wal`/`-shm` with it.
/// Never deletes and never overwrites: if that name is taken, `-2`, `-3`, … is appended.
///
/// The side files move FIRST and the main file last (I1). A `-wal` left beside a missing or empty
/// main file is deleted by SQLite on the next open, and with it every commit not yet checkpointed.
/// So either all of the Ledger moves, or nothing does: a failed main move puts the side files
/// back, and a `-wal` still at the old path afterwards is an error.
///
/// A crash between the renames is covered by a marker (fix round 2): `<name>.quarantine` names
/// the target and is synced before the first rename, and removed once the move is whole or
/// undone. The next quarantine or open first `resume`s any marked attempt, so a retry never
/// strands the first attempt's `-wal` under a different name from its Ledger.
pub fn quarantine(path: &Path, now_unix: u64) -> std::io::Result<PathBuf> {
    quarantine_with(path, now_unix, os_rename)
}

/// `<name>.quarantine`: the move-aside in progress for the Ledger at `path`.
fn marker(path: &Path) -> PathBuf {
    side(path, ".quarantine")
}

fn sync_dir(path: &Path) -> std::io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::File::open(dir)?.sync_all()
}

/// Settles a move-aside that a crash interrupted, from its marker. If the main file already
/// moved, the attempt was whole and only the marker goes. Otherwise the side files come back to
/// `path`, so the caller's next quarantine moves the whole Ledger under one name. Fails closed,
/// moving nothing, if a side file is at both names, the marker is unreadable, or neither main
/// file exists.
fn resume(path: &Path, rename: Rename) -> std::io::Result<()> {
    let m = marker(path);
    let named = match fs::read_to_string(&m) {
        Ok(t) => t.trim().to_string(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let prefix = format!("{}.unreadable-", file_name(path));
    // Only a name `quarantine_with` writes: `<name>.unreadable-<secs>` or `…-<secs>-<n>`.
    let stamp_ok = named.strip_prefix(&prefix).is_some_and(|rest| {
        let mut parts = rest.split('-');
        let digits = |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        digits(parts.next()) && parts.clone().count() <= 1 && parts.next().map_or(true, |n| digits(Some(n)))
    });
    if !stamp_ok || named.contains('/') {
        return Err(std::io::Error::other(format!("{} does not name a move-aside; it was left for a human to look at", file_name(&m))));
    }
    let to = path.with_file_name(&named);
    if !to.exists() {
        if !path.exists() {
            return Err(std::io::Error::other(format!("{} names a move-aside, but neither Ledger file exists", file_name(&m))));
        }
        let back: Vec<&str> = ["-wal", "-shm"].into_iter().filter(|s| side(&to, s).exists()).collect();
        if let Some(s) = back.iter().find(|s| side(path, s).exists()) {
            return Err(std::io::Error::other(format!(
                "an interrupted move-aside left both {} and {}; both were left in place",
                file_name(&side(path, s)),
                file_name(&side(&to, s))
            )));
        }
        for s in back {
            rename(&side(&to, s), &side(path, s))?;
        }
    }
    fs::remove_file(&m)?;
    sync_dir(path)
}

fn quarantine_with(path: &Path, now_unix: u64, rename: Rename) -> std::io::Result<PathBuf> {
    resume(path, rename)?;
    let base = format!("{}.unreadable-{now_unix}", file_name(path));
    let mut to = path.with_file_name(&base);
    let mut n = 1;
    while to.exists() || side(&to, "-wal").exists() || side(&to, "-shm").exists() {
        n += 1;
        to = path.with_file_name(format!("{base}-{n}"));
    }
    {
        use std::io::Write;
        let mut f = fs::File::create(marker(path))?;
        f.write_all(format!("{}\n", file_name(&to)).as_bytes())?;
        f.sync_all()?;
        sync_dir(path)?; // the marker is on disk before anything moves
    }
    let mut moved: Vec<&str> = Vec::new();
    // Puts the moved side files back; the marker goes only if all of them made it, so a failed
    // undo is finished by the next `resume`.
    let undo = |moved: &[&str]| {
        if moved.iter().all(|s| rename(&side(&to, s), &side(path, s)).is_ok()) {
            let _ = fs::remove_file(marker(path));
        }
    };
    for s in ["-wal", "-shm"] {
        if side(path, s).exists() {
            if let Err(e) = rename(&side(path, s), &side(&to, s)) {
                undo(&moved);
                return Err(e);
            }
            moved.push(s);
        }
    }
    if let Err(e) = rename(path, &to) {
        undo(&moved);
        return Err(e);
    }
    fs::remove_file(marker(path))?;
    sync_dir(path)?;
    if side(path, "-wal").exists() {
        return Err(std::io::Error::other("a -wal file is still beside the old Ledger's name; it was left in place"));
    }
    Ok(to)
}

/// Some(refusal) when a `-wal` sits at `path` with no Ledger beside it: creating a Ledger there
/// would make SQLite delete that WAL, and the commits in it (I1 backstop).
fn orphan_wal(path: &Path) -> Option<Opened> {
    (!path.exists() && side(path, "-wal").exists()).then(|| {
        Opened::without(format!(
            "a {}-wal file has no Ledger beside it; it was left untouched and this session keeps no history",
            file_name(path)
        ))
    })
}

fn fresh(path: &Path, key: &LedgerKey, note: Option<String>) -> Opened {
    if let Some(refused) = orphan_wal(path) {
        return refused;
    }
    match Ledger::open(path, key) {
        Ok(l) => Opened { ledger: Some(l), note },
        Err(e) => Opened::without(format!("could not start the Ledger ({e}); this session keeps no history")),
    }
}

/// Opens the Ledger at `path` with the key `keys` holds, making both on first launch.
/// - A lost key: the old Ledger is moved aside, a new key is kept, and only then is a new Ledger made.
/// - A key that doesn't open the file: the file is moved aside and a new Ledger made with that key.
/// - A key that can't be read (unavailable, malformed): no Ledger, no new key, nothing moved.
/// - A Ledger from a newer XMRange: left untouched, no Ledger.
/// - A Ledger that can't be moved aside whole, or a `-wal` with no Ledger: left untouched, no Ledger.
pub fn open_with(path: &Path, keys: &dyn KeySource, now_unix: u64) -> Opened {
    open_inner(path, keys, now_unix, os_rename)
}

fn open_inner(path: &Path, keys: &dyn KeySource, now_unix: u64, rename: Rename) -> Opened {
    let loaded = match keys.load() {
        Err(e) => return Opened::without(format!("the Ledger key could not be read ({e}); this session keeps no history")),
        Ok(k) => k,
    };
    // A move-aside a crash interrupted is settled before anything reads or makes a Ledger here.
    if let Err(e) = resume(path, rename) {
        return Opened::without(format!("an interrupted move-aside could not be settled ({e}); this session keeps no history"));
    }
    let key = match loaded {
        Some(k) => k,
        None => {
            // No key was ever kept here, so an existing Ledger can never be read again: move it
            // aside first, then keep a new key, and only then create the new Ledger.
            let mut note = None;
            if path.exists() {
                match quarantine_with(path, now_unix, rename) {
                    Ok(to) => {
                        note = Some(format!(
                            "saved trials could not be read · starting fresh (old file kept as {})",
                            file_name(&to)
                        ))
                    }
                    Err(e) => {
                        return Opened::without(format!(
                            "saved trials could not be read · this session will not save results ({e})"
                        ))
                    }
                }
            }
            if let Some(refused) = orphan_wal(path) {
                return refused; // before a key is kept: nothing is made while a WAL is orphaned
            }
            let k = LedgerKey::generate();
            if let Err(e) = keys.store(&k) {
                return Opened::without(format!("could not keep a new Ledger key ({e}); this session keeps no history"));
            }
            return fresh(path, &k, note);
        }
    };
    if let Some(refused) = orphan_wal(path) {
        return refused;
    }
    match Ledger::open(path, &key) {
        Ok(l) => Opened { ledger: Some(l), note: None },
        Err(LedgerError::WrongKeyOrNotALedger) | Err(LedgerError::Corrupt(_)) => match quarantine_with(path, now_unix, rename) {
            Ok(to) => fresh(
                path,
                &key,
                Some(format!("the Ledger could not be read with this Mac's key; it was moved aside to {} and a new one started", file_name(&to))),
            ),
            Err(e) => Opened::without(format!("the Ledger could not be read and could not be moved aside ({e}); this session keeps no history")),
        },
        Err(LedgerError::TooNew { found, known }) => Opened::without(format!(
            "the Ledger was written by a newer XMRange (schema {found}; this build knows {known}); it was left untouched and this session keeps no history"
        )),
        Err(e) => Opened::without(format!("could not open the Ledger ({e}); this session keeps no history")),
    }
}

/// How long `open_default` waits on another process's open (T9 review m5). A CLI `record` holds
/// the lock across its key read, which touches the disk only, never the network; an app launched
/// meanwhile would otherwise block in `main()`, with no window, until that read finishes.
const OPEN_LOCK_WAIT: Duration = Duration::from_secs(10);

/// Takes `file`'s exclusive lock, trying again until `limit` passes. `Ok(None)`: another process
/// still holds it.
fn lock_within(file: File, limit: Duration) -> std::io::Result<Option<File>> {
    let deadline = Instant::now() + limit;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(file)),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(e)) => return Err(e),
        }
    }
}

/// The build's Ledger, keyed by the one file at `paths::key_path()` (`dev.key` in dev builds,
/// `app.key` in release; same code path either way, owner decision 2026-09-28: no OS
/// credential store, ever). Makes the data folder (0700), the key and the Ledger on first launch.
///
/// Two processes (the app and a CLI `record`) opening for the first time at once could each make
/// and keep a key, the last one kept winning. So the whole open — load, keep, create — runs under
/// an exclusive lock on `<data dir>/ledger.lock` (T8 review m1). It is held only while opening.
pub fn open_default() -> Opened {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let dir = crate::paths::data_dir();
    if let Err(e) = fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir) {
        return Opened::without(format!("could not create the data folder ({e}); this session keeps no history"));
    }
    let lock = fs::OpenOptions::new().create(true).truncate(false).write(true).mode(0o600).open(dir.join("ledger.lock"));
    let _lock = match lock.and_then(|f| lock_within(f, OPEN_LOCK_WAIT)) {
        Ok(Some(f)) => f, // released when dropped, at the end of this function
        Ok(None) => return Opened::without("another XMRange process is opening the Ledger; this session keeps no history".into()),
        Err(e) => return Opened::without(format!("could not lock the Ledger for opening ({e}); this session keeps no history")),
    };
    let now = crate::telemetry::now_secs() as u64;
    open_with(&crate::paths::ledger_path(), &AppKeyFile::new(crate::paths::key_path()), now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{put_canary, read_canary, test_dir};
    use std::fs;
    use std::sync::Mutex;

    /// An in-memory `KeySource`, for tests that want to control exactly when a key does or
    /// doesn't come back (`open::AppKeyFile`'s own file-backed behaviour is covered separately,
    /// in `app_key_file` below, and in `secrets::key_file`'s own tests).
    #[derive(Default)]
    struct MemKey {
        hex: Mutex<Option<String>>,
        fail: Mutex<Option<SecretError>>,
    }
    impl MemKey {
        fn new() -> Self {
            Self::default()
        }
        fn set_failure(&self, e: Option<SecretError>) {
            *self.fail.lock().unwrap() = e;
        }
        fn get_hex(&self) -> Option<String> {
            self.hex.lock().unwrap().clone()
        }
        fn seed_hex(&self, h: String) {
            *self.hex.lock().unwrap() = Some(h);
        }
    }
    impl KeySource for MemKey {
        fn load(&self) -> Result<Option<LedgerKey>, SecretError> {
            if let Some(e) = self.fail.lock().unwrap().clone() {
                return Err(e);
            }
            self.hex.lock().unwrap().as_deref().map(LedgerKey::from_hex).transpose()
        }
        fn store(&self, key: &LedgerKey) -> Result<(), SecretError> {
            if let Some(e) = self.fail.lock().unwrap().clone() {
                return Err(e);
            }
            let mut slot = self.hex.lock().unwrap();
            if slot.is_some() {
                return Err(SecretError::Io("a Ledger key is already kept".into())); // T8 review m3: no nested "left untouched"
            }
            *slot = Some(key.to_hex());
            Ok(())
        }
    }

    /// T9 review m5: a held open lock is waited on for a bounded time, then given up; once it is
    /// released it is taken at once.
    #[test]
    fn the_open_lock_waits_a_bounded_time() {
        let dir = test_dir("open-lock");
        let open = || fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("ledger.lock")).unwrap();
        let first = lock_within(open(), Duration::ZERO).unwrap().expect("a free lock is taken at once");
        let t0 = Instant::now();
        assert!(lock_within(open(), Duration::from_millis(200)).unwrap().is_none(), "held elsewhere: given up");
        let waited = t0.elapsed();
        assert!(waited >= Duration::from_millis(200) && waited < Duration::from_secs(3), "{waited:?}");
        drop(first);
        assert!(lock_within(open(), Duration::ZERO).unwrap().is_some(), "free again once released");
        let _ = fs::remove_dir_all(&dir);
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn first_launch_makes_a_key_then_a_ledger() {
        let p = test_dir("first").join("l.sqlite");
        let mem = MemKey::new();
        let o = open_with(&p, &mem, 1);
        assert!(o.ledger.is_some() && o.note.is_none());
        let key = mem.get_hex().expect("a key is kept");
        assert_eq!(key.len(), 64);
        drop(o);
        assert!(open_with(&p, &mem, 2).ledger.is_some());
        assert_eq!(mem.get_hex(), Some(key), "the same key, reused");
    }

    #[test]
    fn lost_key_quarantines_and_starts_fresh() {
        let dir = test_dir("lost");
        let p = dir.join("l.sqlite");
        let mem = MemKey::new();
        put_canary(&open_with(&p, &mem, 1).ledger.unwrap(), "old");
        let old_bytes = fs::read(&p).unwrap();
        let emptied = MemKey::new(); // the key file is gone
        let o = open_with(&p, &emptied, 1_800_000_000);
        assert!(o.note.as_deref().unwrap().contains("starting fresh"), "{:?}", o.note);
        assert_eq!(fs::read(dir.join("l.sqlite.unreadable-1800000000")).unwrap(), old_bytes, "moved, never deleted or rewritten");
        assert_eq!(read_canary(&o.ledger.unwrap()), None, "a fresh Ledger");
    }

    #[test]
    fn a_foreign_ledger_is_moved_aside_and_the_kept_key_stays() {
        let dir = test_dir("foreign");
        let p = dir.join("l.sqlite");
        drop(Ledger::open(&p, &LedgerKey::generate()).unwrap()); // made on "another Mac"
        let mem = MemKey::new();
        let kept = LedgerKey::generate();
        mem.seed_hex(kept.to_hex());
        let o = open_with(&p, &mem, 7);
        assert!(o.ledger.is_some() && o.note.as_deref().unwrap().contains("moved aside"));
        assert!(dir.join("l.sqlite.unreadable-7").exists());
        assert_eq!(mem.get_hex(), Some(kept.to_hex()));
    }

    /// A key that can't be read — unavailable, a damaged file (T8 review m5) — is never "lost":
    /// no new key, nothing moved, no history this session.
    #[test]
    fn an_unreadable_key_never_rekeys() {
        let failures = [SecretError::Io("disk full".into()), SecretError::Corrupt("not valid hex".into())];
        for (i, failure) in failures.into_iter().enumerate() {
            let dir = test_dir(&format!("denied-{i}"));
            let p = dir.join("l.sqlite");
            let mem = MemKey::new();
            drop(open_with(&p, &mem, 1));
            let key_before = mem.get_hex();
            let files_before = listing(&dir);
            let bytes_before = fs::read(&p).unwrap();
            mem.set_failure(Some(failure.clone()));
            let o = open_with(&p, &mem, 2);
            assert!(o.ledger.is_none(), "{failure:?}");
            assert!(o.note.as_deref().unwrap().contains("no history"), "{failure:?}: {:?}", o.note);
            mem.set_failure(None);
            assert_eq!(mem.get_hex(), key_before, "{failure:?} never makes a new key");
            assert_eq!(listing(&dir), files_before, "{failure:?}: nothing moved aside");
            assert_eq!(fs::read(&p).unwrap(), bytes_before, "{failure:?}");
        }
    }

    #[test]
    fn a_newer_ledger_is_left_alone() {
        let dir = test_dir("newer");
        let p = dir.join("l.sqlite");
        let mem = MemKey::new();
        open_with(&p, &mem, 1).ledger.unwrap().conn.pragma_update(None, "user_version", 99).unwrap();
        let before = fs::read(&p).unwrap();
        let o = open_with(&p, &mem, 2);
        assert!(o.ledger.is_none() && o.note.as_deref().unwrap().contains("newer"));
        assert_eq!(fs::read(&p).unwrap(), before);
        assert_eq!(listing(&dir).iter().filter(|n| n.contains("unreadable")).count(), 0);
    }

    struct CannotKeep;
    impl KeySource for CannotKeep {
        fn load(&self) -> Result<Option<LedgerKey>, SecretError> {
            Ok(None)
        }
        fn store(&self, _: &LedgerKey) -> Result<(), SecretError> {
            Err(SecretError::Io("could not keep a new Ledger key".into()))
        }
    }

    #[test]
    fn a_key_that_cannot_be_kept_starts_no_ledger() {
        let p = test_dir("cannotkeep").join("l.sqlite");
        let o = open_with(&p, &CannotKeep, 1);
        assert!(o.ledger.is_none());
        assert!(!p.exists(), "no Ledger is ever written without a kept key");
    }

    #[test]
    fn a_second_move_aside_never_overwrites_the_first() {
        let dir = test_dir("twice");
        let p = dir.join("l.sqlite");
        fs::write(&p, b"first").unwrap();
        assert_eq!(quarantine(&p, 5).unwrap(), dir.join("l.sqlite.unreadable-5"));
        fs::write(&p, b"second").unwrap();
        assert_eq!(quarantine(&p, 5).unwrap(), dir.join("l.sqlite.unreadable-5-2"));
        assert_eq!(fs::read(dir.join("l.sqlite.unreadable-5")).unwrap(), b"first");
        assert_eq!(fs::read(dir.join("l.sqlite.unreadable-5-2")).unwrap(), b"second");
    }

    #[test]
    fn a_kept_key_is_never_overwritten_through_a_store() {
        let mem = MemKey::new();
        let kept = LedgerKey::generate();
        mem.store(&kept).unwrap();
        assert!(mem.store(&LedgerKey::generate()).is_err());
        assert_eq!(mem.get_hex(), Some(kept.to_hex()));
    }

    /// A Ledger as a crash leaves it: the main file plus a `-wal` holding its newest commit
    /// (the canary), copied while the writing connection was still open. Returns (dir, path, key).
    fn crashed(tag: &str) -> (PathBuf, PathBuf, LedgerKey) {
        let src = test_dir(&format!("{tag}-src")).join("l.sqlite");
        let key = LedgerKey::generate();
        let l = Ledger::open(&src, &key).unwrap();
        put_canary(&l, "tail");
        let dir = test_dir(tag);
        let p = dir.join("l.sqlite");
        fs::copy(&src, &p).unwrap();
        fs::copy(side(&src, "-wal"), side(&p, "-wal")).unwrap();
        let main_only = test_dir(&format!("{tag}-mainonly")).join("l.sqlite");
        fs::copy(&src, &main_only).unwrap();
        drop(l);
        let without_wal = Ledger::open(&main_only, &key).ok().and_then(|l| read_canary(&l));
        assert_eq!(without_wal, None, "the newest commit lives only in the -wal");
        (dir, p, key)
    }

    fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
        listing(dir).into_iter().map(|n| (n.clone(), fs::read(dir.join(&n)).unwrap())).collect()
    }

    fn refuse_wal(from: &Path, to: &Path) -> std::io::Result<()> {
        if from.to_string_lossy().ends_with("-wal") && !to.to_string_lossy().ends_with("l.sqlite-wal") {
            return Err(std::io::Error::other("test: the -wal move is refused"));
        }
        fs::rename(from, to)
    }

    fn refuse_main(from: &Path, to: &Path) -> std::io::Result<()> {
        if from.to_string_lossy().ends_with(".sqlite") {
            return Err(std::io::Error::other("test: the main move is refused"));
        }
        fs::rename(from, to)
    }

    /// I1: the aside copy keeps the commits that were still in its WAL.
    #[test]
    fn quarantine_keeps_the_wal_with_the_ledger() {
        let (dir, p, key) = crashed("q-wal");
        let to = quarantine(&p, 11).unwrap();
        assert_eq!(to, dir.join("l.sqlite.unreadable-11"));
        assert!(!p.exists() && !side(&p, "-wal").exists());
        assert_eq!(read_canary(&Ledger::open(&to, &key).unwrap()).as_deref(), Some("tail"), "the newest commit survives the move");
    }

    /// I1: a side file that can't be moved means nothing moves and nothing new is made.
    #[test]
    fn a_failed_wal_move_moves_nothing_and_makes_nothing() {
        let (dir, p, key) = crashed("q-walfail");
        let before = snapshot(&dir);
        let emptied = MemKey::new(); // the key is lost, so open_with tries to move the Ledger aside
        let o = open_inner(&p, &emptied, 12, refuse_wal);
        assert!(o.ledger.is_none() && o.note.as_deref().unwrap().contains("will not save results"), "{:?}", o.note);
        assert_eq!(snapshot(&dir), before, "every file where it was, byte for byte; no new Ledger");
        assert_eq!(emptied.get_hex(), None, "no new key");
        assert_eq!(read_canary(&Ledger::open(&p, &key).unwrap()).as_deref(), Some("tail"), "no data lost");
    }

    /// I1: a main file that can't be moved puts the already-moved side files back.
    #[test]
    fn a_failed_main_move_puts_the_wal_back() {
        let (dir, p, key) = crashed("q-mainfail");
        let before = snapshot(&dir);
        assert!(quarantine_with(&p, 13, refuse_main).is_err());
        assert_eq!(snapshot(&dir), before, "the -wal is back beside its Ledger");
        assert_eq!(read_canary(&Ledger::open(&p, &key).unwrap()).as_deref(), Some("tail"));
    }

    /// I1 backstop: a `-wal` with no Ledger beside it is never discarded by a new Ledger.
    #[test]
    fn an_orphan_wal_is_never_discarded() {
        let (dir, p, key) = crashed("orphan");
        fs::remove_file(&p).unwrap(); // as a crash between the old order's renames left it
        let before = snapshot(&dir);
        let emptied = MemKey::new();
        let o = open_with(&p, &emptied, 14);
        assert!(o.ledger.is_none() && o.note.as_deref().unwrap().contains("no Ledger beside it"), "{:?}", o.note);
        assert_eq!(emptied.get_hex(), None, "no new key");
        let kept = MemKey::new();
        kept.seed_hex(key.to_hex());
        assert!(open_with(&p, &kept, 15).ledger.is_none(), "not with a kept key either");
        assert_eq!(snapshot(&dir), before, "no Ledger made, the -wal untouched");
    }

    /// A process killed right before the main file's move: the side files are already aside.
    fn killed_before_main(from: &Path, to: &Path) -> std::io::Result<()> {
        if from.to_string_lossy().ends_with(".sqlite") {
            panic!("test: the process is killed before the main move");
        }
        fs::rename(from, to)
    }

    /// A process killed right after the main file's move, before the marker is removed.
    fn killed_after_main(from: &Path, to: &Path) -> std::io::Result<()> {
        fs::rename(from, to)?;
        if from.to_string_lossy().ends_with(".sqlite") {
            panic!("test: the process is killed after the main move");
        }
        Ok(())
    }

    fn killed(p: &Path, now: u64, rename: Rename) {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| quarantine_with(p, now, rename)));
        assert!(r.is_err(), "the seam stands in for a kill");
    }

    /// Fix round 2: a kill between the side and main moves, then a retry. Both parts of the
    /// Ledger end up under the retry's one name, and its newest commit is readable there.
    #[test]
    fn a_move_aside_killed_midway_is_resumed_whole() {
        let (dir, p, key) = crashed("q-killed");
        killed(&p, 20, killed_before_main);
        assert!(p.exists() && !side(&p, "-wal").exists(), "the crash left the main file behind");
        assert!(side(&dir.join("l.sqlite.unreadable-20"), "-wal").exists() && marker(&p).exists());
        let emptied = MemKey::new(); // the relaunch: the key is still lost
        let o = open_with(&p, &emptied, 21);
        assert!(o.ledger.is_some() && o.note.as_deref().unwrap().contains("l.sqlite.unreadable-21"), "{:?}", o.note);
        let aside = dir.join("l.sqlite.unreadable-21");
        assert_eq!(read_canary(&Ledger::open(&aside, &key).unwrap()).as_deref(), Some("tail"), "the newest commit is in the aside copy");
        assert!(listing(&dir).iter().all(|n| !n.contains("unreadable-20") && !n.ends_with(".quarantine")), "{:?}", listing(&dir));
    }

    /// Fix round 2: a kill after the main move left a whole aside copy; only the marker goes.
    #[test]
    fn a_move_aside_killed_after_the_main_move_is_kept() {
        let (dir, p, key) = crashed("q-killed-late");
        killed(&p, 22, killed_after_main);
        assert!(!p.exists() && marker(&p).exists());
        let o = open_with(&p, &MemKey::new(), 23);
        assert!(o.ledger.is_some() && o.note.is_none(), "{:?}", o.note);
        assert_eq!(read_canary(&Ledger::open(&dir.join("l.sqlite.unreadable-22"), &key).unwrap()).as_deref(), Some("tail"));
        assert!(!marker(&p).exists());
    }

    /// Fix round 2: a side file at both names can't be settled safely, so nothing moves.
    #[test]
    fn a_conflicting_interrupted_move_aside_fails_closed() {
        let (dir, p, _) = crashed("q-conflict");
        killed(&p, 24, killed_before_main);
        fs::write(side(&p, "-wal"), b"another wal").unwrap();
        let before = snapshot(&dir);
        let emptied = MemKey::new();
        let o = open_with(&p, &emptied, 25);
        let note = o.note.as_deref().unwrap();
        assert!(o.ledger.is_none() && note.contains("interrupted move-aside"), "{note}");
        assert!(note.contains("l.sqlite-wal") && note.contains("l.sqlite.unreadable-24-wal"), "the note names both files: {note}");
        assert!(quarantine(&p, 26).is_err());
        assert_eq!(snapshot(&dir), before, "nothing moved");
        assert_eq!(emptied.get_hex(), None, "no new key");
    }

    /// A Ledger with its canary and a marker holding `text`, as if a move-aside had begun.
    fn marked(tag: &str, text: &str) -> (PathBuf, PathBuf, MemKey) {
        let dir = test_dir(tag);
        let p = dir.join("l.sqlite");
        let mem = MemKey::new();
        put_canary(&open_with(&p, &mem, 1).ledger.unwrap(), "kept");
        fs::write(marker(&p), text).unwrap();
        (dir, p, mem)
    }

    /// Re-review 2: a marker that doesn't name a move-aside this code writes fails closed:
    /// nothing moves, the marker stays for a human, and this session keeps no history.
    #[test]
    fn a_garbled_marker_fails_closed() {
        for (i, text) in ["", "garbage\n", "l.sqlite.unreadable-\n", "l.sqlite.unreadable-12x\n", "l.sqlite.unreadable-1-2-3\n", "../l.sqlite.unreadable-5\n"].into_iter().enumerate() {
            let (dir, p, mem) = marked(&format!("m-garbled-{i}"), text);
            let before = snapshot(&dir);
            let o = open_with(&p, &mem, 30);
            assert!(o.ledger.is_none() && o.note.as_deref().unwrap().contains("does not name a move-aside"), "{text:?}: {:?}", o.note);
            assert!(quarantine(&p, 31).is_err(), "{text:?}: a quarantine fails closed too");
            assert_eq!(snapshot(&dir), before, "{text:?}: nothing moved, the marker kept");
        }
    }

    /// Re-review 2: a marker cut short while it was written (so before anything moved) still
    /// names a well-formed move-aside. Nothing is at that name, so nothing moves back, only the
    /// marker goes, and the Ledger opens with its data.
    #[test]
    fn a_truncated_marker_moves_nothing_and_the_ledger_opens() {
        let (dir, p, mem) = marked("m-truncated", "l.sqlite.unreadable-2"); // cut from "…-20\n"
        let o = open_with(&p, &mem, 32);
        assert!(o.note.is_none(), "{:?}", o.note);
        assert_eq!(read_canary(&o.ledger.unwrap()).as_deref(), Some("kept"));
        assert!(!marker(&p).exists());
        assert!(listing(&dir).iter().all(|n| !n.contains("unreadable")), "{:?}", listing(&dir));
    }

    /// Re-review 2: a marker whose Ledger is at neither name fails closed, and no key is made.
    #[test]
    fn a_marker_with_neither_ledger_fails_closed() {
        let dir = test_dir("m-neither");
        let p = dir.join("l.sqlite");
        fs::write(marker(&p), "l.sqlite.unreadable-5\n").unwrap();
        let emptied = MemKey::new();
        let o = open_with(&p, &emptied, 33);
        assert!(o.ledger.is_none() && o.note.as_deref().unwrap().contains("neither Ledger file exists"), "{:?}", o.note);
        assert_eq!(emptied.get_hex(), None, "no new key");
        assert_eq!(listing(&dir), ["l.sqlite.quarantine"], "nothing made; the marker kept");
    }

    mod app_key_file {
        use super::*;

        /// `AppKeyFile` bridges `secrets::key_file::KeyFile`'s raw bytes to `LedgerKey`'s hex; the
        /// file format and atomic write themselves are `secrets::key_file`'s own tests to keep.
        #[test]
        fn round_trips_through_the_key_source_trait() {
            let dir = test_dir("appkeyfile");
            let f = AppKeyFile::new(dir.join("app.key"));
            assert!(f.load().unwrap().is_none());
            let k = LedgerKey::generate();
            f.store(&k).unwrap();
            assert!(f.store(&LedgerKey::generate()).is_err(), "an existing key is never overwritten");
            assert_eq!(f.load().unwrap().unwrap().to_hex(), k.to_hex());
        }

        #[test]
        fn a_malformed_key_file_is_kept_and_no_ledger_opens() {
            let dir = test_dir("appkeyfile-bad");
            let key = dir.join("app.key");
            fs::write(&key, "not a key").unwrap();
            let o = open_with(&dir.join("l.sqlite"), &AppKeyFile::new(key.clone()), 1);
            assert!(o.ledger.is_none() && o.note.as_deref().unwrap().contains("key could not be read"));
            assert_eq!(fs::read_to_string(&key).unwrap(), "not a key", "left for a human to look at");
            assert!(!dir.join("l.sqlite").exists());
        }
    }
}
