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
    use std::sync::atomic::AtomicUsize;

    static DIR_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A unique scratch directory per call: these tests must not collide when
    /// the suite runs them in parallel.
    fn scratch(tag: &str) -> PathBuf {
        let n = DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("tmper-scan-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Copy a real, decodable file into `dir`.
    ///
    /// A stub with an audio extension is not enough: the scan only emits a
    /// `Track` once `read_metadata` succeeds, so bytes that are not a valid
    /// container just exercise the failure path.
    fn write_real_track(dir: &Path, name: &str) -> PathBuf {
        let dest = dir.join(name);
        std::fs::copy(Path::new("tests/fixtures/test.flac"), &dest)
            .expect("fixture tests/fixtures/test.flac");
        dest
    }

    /// `(size, mtime)` exactly as the scanner will compute it.
    fn fingerprint(path: &Path) -> (u64, i64) {
        let metadata = std::fs::metadata(path).unwrap();
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);
        (metadata.len(), mtime)
    }

    /// Run a scan to completion and return every update it produced.
    fn scan(root: PathBuf, known: HashMap<PathBuf, (u64, i64)>, cancel: bool) -> Vec<ScanUpdate> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        scan_incremental(root, known, tx, Arc::new(AtomicBool::new(cancel)));
        let mut updates = Vec::new();
        while let Ok(update) = rx.try_recv() {
            updates.push(update);
        }
        updates
    }

    /// The `Finished` summary, as a tuple to keep assertions readable.
    fn summary(updates: &[ScanUpdate]) -> (usize, usize, usize, bool, bool) {
        for update in updates {
            if let ScanUpdate::Finished {
                scanned,
                changed,
                failed,
                complete,
                cancelled,
                ..
            } = update
            {
                return (*scanned, *changed, *failed, *complete, *cancelled);
            }
        }
        panic!("scan produced no Finished update");
    }

    fn track_count(updates: &[ScanUpdate]) -> usize {
        updates
            .iter()
            .filter(|update| matches!(update, ScanUpdate::Track(_)))
            .count()
    }

    #[test]
    fn test_scan_filters_by_extension() {
        let tmp = scratch("filters");
        std::fs::create_dir_all(tmp.join("subdir")).unwrap();
        std::fs::write(tmp.join("song.mp3"), b"fake").unwrap();
        std::fs::write(tmp.join("subdir/album.flac"), b"fake").unwrap();
        std::fs::write(tmp.join("readme.txt"), b"fake").unwrap();

        let results = scan_directory(&tmp);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|path| is_audio_file(path)));

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn is_audio_file_is_case_insensitive_and_rejects_non_audio() {
        assert!(is_audio_file(Path::new("a.MP3")));
        assert!(is_audio_file(Path::new("/x/y/b.FlAc")));
        assert!(!is_audio_file(Path::new("cover.jpg")));
        assert!(!is_audio_file(Path::new("noextension")));
        assert!(!is_audio_file(Path::new("archive.mp3.bak")));
    }

    /// A missing root is the cheapest way to produce a walk error: walkdir
    /// yields one error entry and no files. That must not be reported as a
    /// complete listing, or the caller would prune the whole subtree.
    #[test]
    fn an_unreadable_root_is_reported_as_incomplete() {
        let missing = std::env::temp_dir().join("tmper-scan-does-not-exist-9f3a1c");
        let _ = std::fs::remove_dir_all(&missing);

        let updates = scan(missing, HashMap::new(), false);
        let (scanned, changed, failed, complete, cancelled) = summary(&updates);

        assert_eq!(scanned, 0);
        assert_eq!(changed, 0);
        assert_eq!(failed, 0);
        assert!(!complete, "a walk that failed must not look complete");
        assert!(!cancelled);
        assert_eq!(
            track_count(&updates),
            0,
            "nothing may be emitted as a track"
        );
    }

    /// The scenario the `complete` flag exists for: one unreadable subtree in
    /// an otherwise fine tree. Every file under it is missing from `seen`, so
    /// pruning on this listing would delete them from the index.
    #[test]
    fn an_unreadable_subdirectory_makes_the_listing_incomplete() {
        let root = scratch("partial");
        std::fs::write(root.join("visible.mp3"), b"x").unwrap();
        let locked = root.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("hidden.mp3"), b"x").unwrap();

        // chmod 000 does not restrict root, which would make this test vacuous.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        }
        if std::fs::read_dir(&locked).is_ok() {
            // Running as root (or on a filesystem that ignores the mode):
            // restore and skip rather than assert something untrue.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let _ = std::fs::remove_dir_all(&root);
            return;
        }

        let updates = scan(root.clone(), HashMap::new(), false);
        let (scanned, _, _, complete, cancelled) = summary(&updates);
        assert_eq!(scanned, 1, "only the readable file is listed");
        assert!(
            !complete,
            "the unreadable subtree must invalidate the listing"
        );
        assert!(!cancelled);

        // Restore permissions so the scratch directory can be cleaned up.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_complete_scan_of_new_files_reports_every_track() {
        let root = scratch("fresh");
        write_real_track(&root, "a.flac");
        write_real_track(&root, "b.flac");
        std::fs::write(root.join("notes.txt"), b"x").unwrap();

        let updates = scan(root.clone(), HashMap::new(), false);
        let (scanned, _, _, complete, cancelled) = summary(&updates);

        assert_eq!(scanned, 2, "the .txt file is not a candidate");
        assert!(complete);
        assert!(!cancelled);
        assert_eq!(
            track_count(&updates),
            2,
            "both files are emitted for indexing"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A fingerprint match is the whole point of an incremental scan: the file
    /// is still listed (so pruning keeps it) but not re-read.
    #[test]
    fn unchanged_files_are_listed_but_not_re_emitted() {
        let root = scratch("unchanged");
        let song = write_real_track(&root, "song.flac");

        let known = HashMap::from([(song, fingerprint(&root.join("song.flac")))]);
        let updates = scan(root.clone(), known, false);
        let (scanned, changed, _, complete, _) = summary(&updates);

        assert_eq!(scanned, 1, "still counted as seen, so it is not pruned");
        assert_eq!(changed, 0);
        assert_eq!(track_count(&updates), 0, "no re-read for an unchanged file");
        assert!(complete);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A file whose metadata changed is re-read, which is what makes the
    /// fingerprint comparison worth anything.
    #[test]
    fn a_changed_fingerprint_triggers_a_re_read() {
        let root = scratch("changed");
        let song = write_real_track(&root, "song.flac");

        // Same path, deliberately wrong fingerprint.
        let known = HashMap::from([(song, (1u64, 1i64))]);
        let updates = scan(root.clone(), known, false);
        let (_, changed, _, _, _) = summary(&updates);

        assert_eq!(changed, 1);
        assert_eq!(track_count(&updates), 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A file that is not decodable counts as failed, and must not be reported
    /// as a track or as a complete, clean scan.
    #[test]
    fn unreadable_metadata_is_counted_as_failed() {
        let root = scratch("corrupt");
        // An audio extension over bytes that are not a valid container.
        std::fs::write(root.join("broken.mp3"), b"definitely not audio").unwrap();

        let updates = scan(root.clone(), HashMap::new(), false);
        let (scanned, changed, failed, complete, _) = summary(&updates);

        assert_eq!(scanned, 1);
        assert_eq!(changed, 0);
        assert_eq!(failed, 1, "the failure is counted, not swallowed");
        assert_eq!(track_count(&updates), 0);
        assert!(
            complete,
            "a file that failed to parse is not a walk error — the listing is still complete"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_cancelled_scan_reports_incomplete_and_cancelled() {
        let root = scratch("cancel");
        std::fs::write(root.join("a.mp3"), b"x").unwrap();

        let updates = scan(root.clone(), HashMap::new(), true);
        let (_, _, _, complete, cancelled) = summary(&updates);

        assert!(cancelled);
        assert!(!complete, "a cancelled walk has not listed everything");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Progress is reported periodically so a long scan shows movement. The
    /// counter is `seen.len()` at the time, not the update index.
    #[test]
    fn progress_is_reported_every_hundred_files() {
        let root = scratch("progress");
        // Progress is emitted from the *skip* branch, so the files have to
        // match their fingerprints — otherwise each one goes to
        // `read_metadata` and the reporting never runs. Stub bytes are fine
        // here precisely because nothing ever reads them.
        let mut known = HashMap::new();
        for i in 0..100 {
            let path = root.join(format!("song{i:03}.mp3"));
            std::fs::write(&path, b"x").unwrap();
            known.insert(path.clone(), fingerprint(&path));
        }

        let updates = scan(root.clone(), known, false);
        let progress: Vec<usize> = updates
            .iter()
            .filter_map(|update| match update {
                ScanUpdate::Progress { scanned, .. } => Some(*scanned),
                _ => None,
            })
            .collect();

        assert!(
            !progress.is_empty(),
            "a 100-file scan must report progress at least once"
        );
        assert!(
            progress.iter().all(|scanned| *scanned > 0),
            "progress reports 1-based counts, got {progress:?}"
        );
        assert_eq!(track_count(&updates), 0, "every file was skipped");

        let _ = std::fs::remove_dir_all(&root);
    }
}
