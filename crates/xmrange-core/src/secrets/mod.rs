//! The app's one secret: the key that opens the Ledger (`key_file::KeyFile`). The wallet is not a
//! secret here — it lives in plain `settings.json`, like the worker name (owner decision
//! 2026-09-28: no separate secret store, and no OS prompt, ever).
use std::fmt;

pub mod key_file;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    /// The stored data isn't what XMRange writes. It is left untouched.
    Corrupt(String),
    Io(String),
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretError::Corrupt(m) => write!(f, "the saved key is unreadable ({m}); it was left untouched"),
            SecretError::Io(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for SecretError {}
