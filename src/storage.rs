//! Crash-safe replacement of the daemon's small persistent files.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Temporary(PathBuf);

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Write beside the destination, sync, rename atomically, then sync its directory.
/// Until rename succeeds, a failed write leaves the previous file intact.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing file name"))?;
    let (temp, mut file) = loop {
        let mut temp_name = name.to_os_string();
        temp_name.push(format!(
            ".{}.{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let temp = parent.join(temp_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
        {
            Ok(file) => break (Temporary(temp), file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                // This file belongs to an earlier process; never remove it.
                continue;
            }
            Err(error) => return Err(error),
        }
    };
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temp.0, path)?;
    File::open(parent)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_a_file_publishes_the_complete_new_contents() {
        let dir = std::env::temp_dir().join(format!("tmper-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.json");
        atomic_write(&path, b"old").unwrap();
        atomic_write(&path, b"{\"complete\":true}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"complete\":true}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_replacement_cleans_up_the_temporary_file() {
        let dir = std::env::temp_dir().join(format!("tmper-atomic-failure-{}", std::process::id()));
        let path = dir.join("destination");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("keep"), b"unchanged").unwrap();
        assert!(atomic_write(&path, b"new").is_err());
        assert_eq!(std::fs::read(path.join("keep")).unwrap(), b"unchanged");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
