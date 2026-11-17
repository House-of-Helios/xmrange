//! The app's one 32-byte key (`paths::key_path()`): a 0600 file holding 64 hex characters and a
//! newline, written atomically (a temp file, fsync, hard-linked into place so an existing key is
//! never overwritten, then the directory synced). This is the Ledger's key, and only the
//! Ledger's: the wallet lives in plain `settings.json` (owner decision 2026-09-28: no separate
//! secret store, and no OS prompt, ever, so opening the Ledger never prompts).
use super::SecretError;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

fn io(e: std::io::Error) -> SecretError {
    SecretError::Io(e.to_string())
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn parse_hex(s: &str) -> Result<[u8; 32], SecretError> {
    let s = s.trim();
    if s.len() != 64 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(SecretError::Corrupt("the key file is not 64 hex characters".into()));
    }
    let mut b = [0u8; 32];
    for (i, pair) in s.as_bytes().chunks(2).enumerate() {
        b[i] = u8::from_str_radix(std::str::from_utf8(pair).expect("ascii"), 16).expect("hex checked");
    }
    Ok(b)
}

fn to_hex(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub struct KeyFile {
    path: PathBuf,
}

impl KeyFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// `Ok(None)`: no key was ever kept here. A malformed file is `Corrupt`, never "absent".
    pub fn load(&self) -> Result<Option<[u8; 32]>, SecretError> {
        match fs::read_to_string(&self.path) {
            Ok(text) => {
                // A key file someone loosened is tightened back; a malformed one is left as is.
                let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600));
                Ok(Some(parse_hex(&text)?))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io(e)),
        }
    }

    /// Keeps a new key. Never overwrites an existing one: the write lands under a temporary name
    /// in the same folder, then `hard_link`s into place, which fails if the target already
    /// exists.
    pub fn store(&self, bytes: &[u8; 32]) -> Result<(), SecretError> {
        let dir = self.path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let tmp = dir.join(format!("{}.tmp-{}", file_name(&self.path), std::process::id()));
        let _ = fs::remove_file(&tmp); // a leftover of this pid's crashed attempt, never a kept key
        let written = (|| {
            let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
            f.write_all(format!("{}\n", to_hex(bytes)).as_bytes())?;
            f.sync_all()?;
            fs::hard_link(&tmp, &self.path)
        })();
        let _ = fs::remove_file(&tmp);
        written.map_err(io)?;
        fs::File::open(dir).and_then(|d| d.sync_all()).map_err(io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        // Inside the repo's target/ (Rule #1: never system temp), as ledger::test_dir does.
        let d = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-scratch")
            .join(format!("key-file-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn mode(p: &Path) -> u32 {
        fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_key_is_created_private_and_never_overwritten() {
        let dir = test_dir("basic");
        let f = KeyFile::new(dir.join("app.key"));
        assert!(f.load().unwrap().is_none());
        let k = [7u8; 32];
        f.store(&k).unwrap();
        assert_eq!(mode(&dir.join("app.key")), 0o600);
        assert!(f.store(&[9u8; 32]).is_err(), "an existing key is never overwritten");
        assert_eq!(f.load().unwrap(), Some(k));
        let names: Vec<String> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["app.key"], "the atomic write leaves no temporary file");
    }

    #[test]
    fn a_lax_key_file_is_tightened() {
        let dir = test_dir("lax");
        let path = dir.join("app.key");
        fs::write(&path, format!("{}\n", to_hex(&[1u8; 32]))).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        KeyFile::new(path.clone()).load().unwrap().unwrap();
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn a_malformed_key_is_corrupt_not_absent() {
        let dir = test_dir("bad");
        let path = dir.join("app.key");
        fs::write(&path, "not a key").unwrap();
        assert!(matches!(KeyFile::new(path).load(), Err(SecretError::Corrupt(_))));
    }

}
