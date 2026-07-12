use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::error::AppResult;
use crate::library::database::LibraryDb;
use crate::metadata::reader::read_metadata;

#[allow(dead_code)]
pub enum ScanEvent {
    Progress {
        found: usize,
        total: usize,
    },
    NewTrack {
        path: String,
        title: String,
    },
    Done {
        added: usize,
        updated: usize,
        removed: usize,
    },
}

#[allow(dead_code)]
pub async fn scan_library(
    music_dirs: &[PathBuf],
    extensions: &[String],
    db: &LibraryDb,
    progress_tx: tokio::sync::mpsc::UnboundedSender<ScanEvent>,
) -> AppResult<()> {
    // Collect all audio files
    let mut all_files: Vec<PathBuf> = Vec::new();
    for dir in music_dirs {
        if !dir.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                if let Some(ext) = entry.path().extension().and_then(|s| s.to_str()) {
                    if extensions.iter().any(|e| e == &ext.to_lowercase()) {
                        all_files.push(entry.path().to_path_buf());
                    }
                }
            }
        }
    }

    let total = all_files.len();
    let mut added = 0usize;
    let mut updated = 0usize;
    let mut scanned_paths: Vec<String> = Vec::new();

    for (i, path) in all_files.iter().enumerate() {
        let _ = progress_tx.send(ScanEvent::Progress {
            found: i + 1,
            total,
        });

        let path_str = path.to_string_lossy().to_string();
        scanned_paths.push(path_str.clone());

        // Get file mtime
        let mtime = match std::fs::metadata(path) {
            Ok(meta) => meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            Err(_) => continue,
        };

        // Check if already in DB with same mtime
        if let Ok(Some(existing)) = db.get_by_path(&path_str) {
            if existing.file_mtime == mtime {
                continue; // unchanged
            }
        }

        // Read metadata and upsert
        match read_metadata(path) {
            Ok(info) => {
                let is_new = db.get_by_path(&path_str).ok().flatten().is_none();
                let _ = db.upsert(
                    &path_str,
                    &info.title,
                    info.artist.as_deref(),
                    info.album.as_deref(),
                    info.album_artist.as_deref(),
                    info.track_number,
                    info.disc_number,
                    info.genre.as_deref(),
                    info.year,
                    info.duration.as_secs_f64(),
                    info.bitrate,
                    info.sample_rate,
                    info.channels as u32,
                    &info.codec,
                    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
                    mtime,
                );
                if is_new {
                    added += 1;
                    let _ = progress_tx.send(ScanEvent::NewTrack {
                        path: path_str,
                        title: info.title,
                    });
                } else {
                    updated += 1;
                }
            }
            Err(e) => {
                tracing::warn!("Failed to read metadata for {:?}: {e}", path);
            }
        }
    }

    // Remove stale entries (DB records for files that no longer exist)
    let mut removed = 0usize;
    for dir in music_dirs {
        let dir_str = dir.to_string_lossy().to_string();
        if let Ok(paths) = db.get_paths_in_dir(&dir_str) {
            for db_path in paths {
                if !scanned_paths.contains(&db_path)
                    && !Path::new(&db_path).exists()
                    && db.delete_by_path(&db_path).is_ok()
                {
                    removed += 1;
                }
            }
        }
    }

    let _ = progress_tx.send(ScanEvent::Done {
        added,
        updated,
        removed,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    #[test]
    fn test_scan_filters_by_extension() {
        let tmp = std::env::temp_dir().join("termusic_test_scan2");
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("song.mp3"), b"fake").unwrap();
        fs::write(tmp.join("readme.txt"), b"fake").unwrap();

        let exts: Vec<String> = vec!["mp3".into(), "flac".into()];
        let results = crate::library::scanner::scan_directory(&tmp, &exts);
        assert_eq!(results.len(), 1);

        fs::remove_dir_all(&tmp).unwrap();
    }
}

#[allow(dead_code)]
pub fn scan_directory(dir: &std::path::Path, extensions: &[String]) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| {
                    extensions
                        .iter()
                        .any(|allowed| allowed == &ext.to_lowercase())
                })
                .unwrap_or(false)
        })
        .map(|e| e.path().to_path_buf())
        .collect()
}
