use anyhow::{Context, Result};
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// Wallet-bearing xmrig config, written 0600 inside a private 0700 dir, removed as soon as
/// xmrig has read it (doctrine §4). Dropping the Handoff removes both, even on error paths.
pub struct Handoff {
    dir: PathBuf,
    file: PathBuf,
}

impl Handoff {
    pub fn create(base: &Path) -> Result<Self> {
        DirBuilder::new().recursive(true).create(base)
            .with_context(|| format!("create {}", base.display()))?;
        let dir = base.join(format!("handoff-{}", crate::token::new_token()));
        DirBuilder::new().mode(0o700).create(&dir)
            .with_context(|| format!("create {}", dir.display()))?;
        let file = dir.join("config.json");
        Ok(Self { dir, file })
    }
    pub fn write(&self, json: &serde_json::Value) -> Result<()> {
        let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&self.file)
            .with_context(|| format!("write {}", self.file.display()))?;
        f.write_all(serde_json::to_string(json)?.as_bytes())?;
        f.sync_all()?;
        Ok(())
    }
    pub fn config_path(&self) -> &Path {
        &self.file
    }
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn remove_file(&self) -> Result<()> {
        match fs::remove_file(&self.file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

impl Drop for Handoff {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.file);
        let _ = fs::remove_dir(&self.dir);
    }
}
