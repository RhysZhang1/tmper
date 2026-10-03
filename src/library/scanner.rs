use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::metadata::reader::TrackInfo;

pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "opus", "wav", "aac", "m4a", "ape", "wv", "aiff", "wma",
];

pub struct ScannedTrack {
    pub info: TrackInfo,
    pub file_size: u64,
    pub file_mtime: i64,
}

pub enum ScanUpdate {
    Track(Box<ScannedTrack>),
    Progress {
        scanned: usize,
        changed: usize,
    },
    Finished {
        root: PathBuf,
        seen: Vec<PathBuf>,
        scanned: usize,
        changed: usize,
        failed: usize,
        /// `false` when the directory walk could not read part of the tree (or
        /// the scan was cancelled). `seen` is then incomplete and must not be
        /// used to prune the index.
        complete: bool,
        cancelled: bool,
    },
}

pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            AUDIO_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

#[cfg(test)]
pub fn scan_directory(dir: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && is_audio_file(entry.path()))
        .map(|entry| entry.into_path())
        .collect()
}

pub fn scan_incremental(
    root: PathBuf,
    known: HashMap<PathBuf, (u64, i64)>,
    tx: tokio::sync::mpsc::UnboundedSender<ScanUpdate>,
    cancel: Arc<AtomicBool>,
) {
    let mut seen = Vec::new();
    let mut changed = 0;
    let mut failed = 0;
    // A walk error hides a whole subtree from `seen`. Pruning on an
    // incomplete listing would delete every indexed track under a directory
    // that merely failed to read (permissions, I/O), so the caller has to
    // know the listing was partial.
    let mut walk_errors = 0usize;

    for entry in walkdir::WalkDir::new(&root).follow_links(false) {
        if cancel.load(Ordering::Relaxed) {
            let scanned = seen.len();
            let _ = tx.send(ScanUpdate::Finished {
                root,
                seen,
                scanned,
                changed,
                failed,
                complete: false,
                cancelled: true,
            });
            return;
        }

        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                walk_errors += 1;
                tracing::warn!("Scan of {root:?} could not read a path: {error}");
                continue;
            }
        };
        if !entry.file_type().is_file() || !is_audio_file(entry.path()) {
            continue;
        }

        let path = entry.into_path();
        seen.push(path.clone());
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                failed += 1;
                continue;
            }
        };
        let file_size = metadata.len();
        let file_mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);

        if known.get(&path) == Some(&(file_size, file_mtime)) {
            if seen.len() % 100 == 0 {
                let _ = tx.send(ScanUpdate::Progress {
                    scanned: seen.len(),
                    changed,
                });
            }
            continue;
        }

        match crate::metadata::reader::read_metadata(&path) {
            Ok(info) => {
                changed += 1;
                if tx
                    .send(ScanUpdate::Track(Box::new(ScannedTrack {
                        info,
                        file_size,
                        file_mtime,
                    })))
                    .is_err()
                {
                    return;
                }
            }
            Err(error) => {
                failed += 1;
                tracing::warn!("Failed to index {:?}: {error}", path);
            }
        }
    }

    let scanned = seen.len();
    let _ = tx.send(ScanUpdate::Finished {
        root,
        seen,
        scanned,
        changed,
        failed,
        complete: walk_errors == 0,
        cancelled: false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_filters_by_extension() {
        let tmp = std::env::temp_dir().join(format!("tmper_scan_{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("subdir")).unwrap();
        std::fs::write(tmp.join("song.mp3"), b"fake").unwrap();
        std::fs::write(tmp.join("subdir/album.flac"), b"fake").unwrap();
        std::fs::write(tmp.join("readme.txt"), b"fake").unwrap();

        let results = scan_directory(&tmp);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|path| is_audio_file(path)));

        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
