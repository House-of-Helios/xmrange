//! Self-check settings backup. Dev builds only: main.rs declares this module behind
//! `#[cfg(feature = "dev-diagnostics")]`, so a release binary has none of it.
//!
//! The static self-check round-trips a few settings (period, theme, measure) and puts each one
//! back. So that a watchdog kill or a crash mid-run can't leave them changed, a self-check launch
//! copies settings.json aside first; the report restores it, and any later dev launch that still
//! finds the backup (the run died) restores it before loading settings.

use std::path::Path;

const BACKUP: &str = "settings.selfcheck-backup.json";
/// Backup content meaning "there was no settings.json": restoring removes the file.
const NO_SETTINGS: &str = "";

pub fn write(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let current = std::fs::read_to_string(dir.join("settings.json")).unwrap_or_else(|_| NO_SETTINGS.to_string());
    let tmp = dir.join(".settings.selfcheck-backup.tmp");
    std::fs::write(&tmp, current)?;
    std::fs::rename(&tmp, dir.join(BACKUP))
}

/// Put settings.json back from the backup, if one exists, and remove the backup.
/// Returns whether settings.json differed from it (and so was restored).
pub fn restore(dir: &Path) -> std::io::Result<bool> {
    let bak = dir.join(BACKUP);
    let Ok(saved) = std::fs::read_to_string(&bak) else { return Ok(false) };
    let path = dir.join("settings.json");
    let now = std::fs::read_to_string(&path).ok();
    let changed = now.as_deref().unwrap_or(NO_SETTINGS) != saved;
    if changed {
        if saved == NO_SETTINGS {
            let _ = std::fs::remove_file(&path);
        } else {
            let tmp = dir.join(".settings.json.tmp");
            std::fs::write(&tmp, &saved)?;
            std::fs::rename(&tmp, &path)?;
        }
    }
    std::fs::remove_file(&bak)?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch dir inside the repo (Rule #1: never system temp).
    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = xmrange_core::paths::scratch_dir(&format!("app-{tag}")).expect("scratch dir");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("scratch dir");
        d
    }

    #[test]
    fn restores_changed_settings() {
        let dir = scratch("backup-changed");
        std::fs::write(dir.join("settings.json"), "original").unwrap();
        write(&dir).unwrap();
        std::fs::write(dir.join("settings.json"), "changed mid-run").unwrap();
        assert!(restore(&dir).unwrap(), "reports that it restored");
        assert_eq!(std::fs::read_to_string(dir.join("settings.json")).unwrap(), "original");
        assert!(!dir.join(BACKUP).exists(), "the backup is consumed");
        assert!(!restore(&dir).unwrap(), "no backup, nothing to do");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn leaves_unchanged_settings_alone() {
        let dir = scratch("backup-same");
        std::fs::write(dir.join("settings.json"), "same").unwrap();
        write(&dir).unwrap();
        assert!(!restore(&dir).unwrap());
        assert_eq!(std::fs::read_to_string(dir.join("settings.json")).unwrap(), "same");
        assert!(!dir.join(BACKUP).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_of_no_settings_removes_the_new_file() {
        let dir = scratch("backup-none");
        write(&dir).unwrap();
        std::fs::write(dir.join("settings.json"), "written by the run").unwrap();
        assert!(restore(&dir).unwrap());
        assert!(!dir.join("settings.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
