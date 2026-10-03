//! The music index, and the scanner that fills it.
//!
//! The library is the one piece of state that outlives both the TUI and the
//! daemon's own run: a SQLite file describing what is on disk. It lives on this
//! side of the socket because SQLite has one writer, and because a second TUI
//! must not be a second writer.
//!
//! Queries are answered straight from the index in `Player::apply` — an artist
//! list is a `SELECT`, not work. Scans are the opposite: a walk of a music
//! directory reads tags out of every file it finds, so the walk runs on a
//! blocking thread that reports back over a channel, and the index is written
//! here, on the daemon's own thread, one tick at a time (see `drain`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::ipc::proto::{ScanReport, TrackLine};
use crate::library::database::{LibraryDb, TrackRow};
use crate::library::scanner::{scan_incremental, ScanUpdate, ScannedTrack};
use crate::metadata::reader::read_metadata;

impl From<TrackRow> for TrackLine {
    fn from(row: TrackRow) -> Self {
        Self {
            path: PathBuf::from(row.path),
            title: row.title,
            artist: row.artist.unwrap_or_default(),
        }
    }
}

/// What a running — or just-finished — scan has to report.
///
/// Facts only, counted by the scanner and by the index that absorbed it. The
/// sentence the user reads ("done: 12 scanned, 3 updated…") is the view's
/// business: the daemon has no keys to advertise and no status bar to fill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanNotice {
    /// A scan is still walking.
    Progress { scanned: usize, changed: usize },
    /// A scan ended. The index has already been written and pruned by the
    /// time this is returned.
    Finished(ScanReport),
}

pub struct Library {
    db: LibraryDb,
    /// The directories and files the user has added, in the order they added
    /// them. Persisted to `library.json` — which used to be the client's file,
    /// and is the daemon's now for the same reason the index is: the list and
    /// the rows it stands for have to move together.
    paths: Vec<PathBuf>,
    /// Where `paths` is written. `None` for an index that never touches disk:
    /// the in-memory fallback, and every test.
    paths_file: Option<PathBuf>,
    scan_tx: tokio::sync::mpsc::UnboundedSender<ScanUpdate>,
    scan_rx: tokio::sync::mpsc::UnboundedReceiver<ScanUpdate>,
    /// Flags that stop a running walk at its next file boundary.
    cancels: Vec<Arc<AtomicBool>>,
    /// Scans in flight, for the status line and for the pruning rule.
    active: usize,
    /// Where a scan is spawned. Absent means there is no async runtime to
    /// spawn onto — a synchronous test — and a scan request is refused rather
    /// than allowed to panic inside `spawn_blocking`.
    runtime: Option<tokio::runtime::Handle>,
}

impl Library {
    /// The library of a real run: `library.db` in the data dir, the path list
    /// in `library.json` next to the rest of the daemon's state.
    pub fn open() -> Self {
        let path = crate::paths::data_dir().join("library.db");
        let paths_file = crate::paths::state_dir().join("library.json");
        let db = match LibraryDb::open(&path) {
            Ok(db) => db,
            // An index that will not open is not a reason to refuse to play
            // music: fall back to one that lives as long as the process.
            Err(error) => {
                tracing::error!("Failed to open {}: {error}", path.display());
                LibraryDb::open_memory().expect("in-memory library")
            }
        };
        let mut library = Self::with_db(db, Some(paths_file));
        library.load_paths();
        library
    }

    /// An index that never touches disk, so no two tests share one.
    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::with_db(LibraryDb::open_memory().expect("in-memory library"), None)
    }

    fn with_db(db: LibraryDb, paths_file: Option<PathBuf>) -> Self {
        let (scan_tx, scan_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            db,
            paths: Vec::new(),
            paths_file,
            scan_tx,
            scan_rx,
            cancels: Vec::new(),
            active: 0,
            runtime: None,
        }
    }

    // ── The path list ──

    /// The roots the user added. What a scan should walk, and what the browser
    /// panel lists.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Add a path to the collection. Returns whether it was new — the caller
    /// scans either way, because adding a path the library already has is how
    /// the user asks for a fresh walk of it.
    pub fn add_path(&mut self, path: &Path) -> bool {
        if self.paths.iter().any(|known| known == path) {
            return false;
        }
        self.paths.push(path.to_path_buf());
        self.save_paths();
        true
    }

    /// Take a path out of the collection, rows and all.
    pub fn remove_path(&mut self, path: &Path) {
        let before = self.paths.len();
        self.paths.retain(|known| known != path);
        if self.paths.len() != before {
            self.save_paths();
        }
        self.forget(path);
    }

    fn load_paths(&mut self) {
        let Some(file) = self.paths_file.clone() else {
            return;
        };
        let Ok(content) = std::fs::read_to_string(&file) else {
            return;
        };
        match serde_json::from_str::<Vec<String>>(&content) {
            Ok(paths) => {
                self.paths = paths
                    .into_iter()
                    .map(PathBuf::from)
                    // A path that has left the disk is dropped here rather
                    // than scanned forever: the two loaders agreed on this
                    // when the client owned the file, and a stale root is
                    // still a stale root.
                    .filter(|path| path.exists())
                    .collect();
            }
            Err(error) => tracing::warn!("Ignoring {}: {error}", file.display()),
        }
    }

    fn save_paths(&self) {
        let Some(file) = self.paths_file.as_ref() else {
            return;
        };
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let paths: Vec<String> = self
            .paths
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect();
        match serde_json::to_string_pretty(&paths) {
            Ok(json) => {
                if let Err(error) = std::fs::write(file, json) {
                    tracing::warn!("Failed to write {}: {error}", file.display());
                }
            }
            Err(error) => tracing::warn!("Failed to encode the library paths: {error}"),
        }
    }

    /// Hand over the runtime scans are spawned on. Called once, by the daemon.
    pub fn attach_scanner(&mut self, handle: tokio::runtime::Handle) {
        self.runtime = Some(handle);
    }

    // ── Queries ──

    pub fn artists(&self) -> Vec<String> {
        self.db.get_artists().unwrap_or_else(|error| {
            tracing::warn!("Artist query failed: {error}");
            Vec::new()
        })
    }

    pub fn albums_by(&self, artist: &str) -> Vec<String> {
        self.db
            .get_albums_by_artist(artist)
            .unwrap_or_else(|error| {
                tracing::warn!("Album query failed: {error}");
                Vec::new()
            })
    }

    pub fn tracks_by(&self, artist: &str, album: &str) -> Vec<TrackLine> {
        self.db
            .get_tracks_by_album(artist, album)
            .unwrap_or_else(|error| {
                tracing::warn!("Track query failed: {error}");
                Vec::new()
            })
            .into_iter()
            .map(TrackLine::from)
            .collect()
    }

    pub fn search(&self, query: &str) -> Vec<TrackLine> {
        self.db
            .search(query)
            .unwrap_or_else(|error| {
                tracing::warn!("Search for {query:?} failed: {error}");
                Vec::new()
            })
            .into_iter()
            .map(TrackLine::from)
            .collect()
    }

    // ── Indexing ──

    /// Put a track the scanner read into the index.
    pub fn index(&mut self, track: &ScannedTrack) {
        let info = &track.info;
        let path = info.path.to_string_lossy().to_string();
        if let Err(error) = self.db.upsert(
            &path,
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
            track.file_size,
            track.file_mtime,
        ) {
            tracing::warn!("Failed to index {path}: {error}");
        }
    }

    /// Read the tags of any path that is not in the index yet.
    ///
    /// This is what puts a track the user just played — or a single file they
    /// added from the browser — into the library without walking anything. The
    /// caller passes the paths it knows about; the queue's own paths come from
    /// `Player`, which owns them.
    pub fn ensure_indexed(&mut self, paths: &[PathBuf]) {
        for path in paths {
            let path_str = path.to_string_lossy().to_string();
            if self.db.get_by_path(&path_str).ok().flatten().is_some() {
                continue;
            }
            let Ok(info) = read_metadata(path) else {
                continue;
            };
            let mtime = file_mtime(path);
            let _ = self.db.upsert(
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
        }
    }

    /// Drop what the index holds for a path the user has taken out of the
    /// library: everything under it if it is a directory, and the file itself
    /// if it is a single one — the browser lets the user add either.
    ///
    /// Both queries are run rather than picking one by `is_dir()`: the prefix
    /// query addresses a directory *by construction* (it matches `root/…`), so
    /// it can never touch a file, and the exact delete is a no-op on a
    /// directory, since rows are files. No `stat`, and no hole for a path that
    /// has already left the disk.
    pub fn forget(&self, root: &Path) {
        match self.db.delete_missing_under(root, &[]) {
            Ok(removed) if removed > 0 => {
                tracing::info!("Removed {removed} tracks under {}", root.display());
            }
            Ok(_) => {}
            Err(error) => tracing::warn!("Failed to forget {}: {error}", root.display()),
        }
        if let Err(error) = self.db.delete_by_path(&root.to_string_lossy()) {
            tracing::warn!("Failed to forget {}: {error}", root.display());
        }
    }

    // ── Scanning ──

    /// Start an incremental walk of `root`, or refuse if there is no runtime
    /// to run it on. Returns whether it started.
    pub fn start_scan(&mut self, root: &Path) -> bool {
        let Some(runtime) = self.runtime.clone() else {
            tracing::warn!("No runtime to scan on; refusing {}", root.display());
            return false;
        };
        // The fingerprints the walk compares against — read here, on the
        // index's own thread, so the scanner thread only ever reads files.
        let known = self
            .db
            .file_fingerprints_under(root)
            .unwrap_or_else(|error| {
                tracing::warn!("Failed to fingerprint {}: {error}", root.display());
                Default::default()
            });
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancels.push(cancel.clone());
        self.active += 1;
        let tx = self.scan_tx.clone();
        let root = root.to_path_buf();
        runtime.spawn_blocking(move || scan_incremental(root, known, tx, cancel));
        true
    }

    /// Ask every running scan to stop. Each one ends at its next file.
    pub fn cancel_scans(&mut self) {
        for cancel in &self.cancels {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// How many scans are running.
    #[cfg(test)]
    pub fn scans_active(&self) -> usize {
        self.active
    }

    /// Absorb whatever the scanner threads have produced since the last call,
    /// writing the index on this thread. One call per tick: the queue between
    /// here and the walkers is unbounded, so a burst of tracks costs one
    /// slightly longer tick, never a dropped file.
    pub fn drain(&mut self) -> Vec<ScanNotice> {
        let mut notices = Vec::new();
        while let Ok(update) = self.scan_rx.try_recv() {
            match update {
                ScanUpdate::Track(track) => self.index(&track),
                ScanUpdate::Progress { scanned, changed } => {
                    notices.push(ScanNotice::Progress { scanned, changed });
                }
                ScanUpdate::Finished {
                    root,
                    seen,
                    scanned,
                    changed,
                    failed,
                    complete,
                    cancelled,
                } => {
                    // Prune only when the listing is trustworthy. A partial
                    // walk (unreadable subtree) reports a `seen` that is
                    // missing files which still exist, and deleting those
                    // would silently drop them from the library.
                    let removed = if cancelled || !complete {
                        0
                    } else {
                        self.db.delete_missing_under(&root, &seen).unwrap_or(0)
                    };
                    self.active = self.active.saturating_sub(1);
                    if self.active == 0 {
                        self.cancels.clear();
                    }
                    notices.push(ScanNotice::Finished(ScanReport {
                        scanned,
                        changed,
                        removed,
                        failed,
                        cancelled,
                        complete,
                        active: self.active,
                    }));
                }
            }
        }
        notices
    }

    /// The index itself, for tests that seed or assert on rows directly.
    #[cfg(test)]
    pub fn db(&self) -> &LibraryDb {
        &self.db
    }

    #[cfg(test)]
    pub fn db_mut(&mut self) -> &mut LibraryDb {
        &mut self.db
    }
}

fn file_mtime(path: &Path) -> i64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(library: &Library, path: &str, title: &str, artist: &str, album: &str) {
        library
            .db()
            .upsert(
                path,
                title,
                Some(artist),
                Some(album),
                None,
                Some(1),
                Some(1),
                Some("Rock"),
                Some(2024),
                200.0,
                320,
                44100,
                2,
                "FLAC",
                10000,
                1000,
            )
            .expect("upsert");
    }

    fn library_with_one_artist() -> Library {
        let library = Library::in_memory();
        seed(&library, "/music/a.flac", "A", "Artist", "Album");
        seed(&library, "/music/b.flac", "B", "Artist", "Album");
        seed(&library, "/music/c.flac", "C", "Other", "Elsewhere");
        library
    }

    #[test]
    fn queries_walk_artist_album_and_track() {
        let library = library_with_one_artist();
        assert_eq!(library.artists(), vec!["Artist", "Other"]);
        assert_eq!(library.albums_by("Artist"), vec!["Album"]);
        assert_eq!(library.albums_by("Nobody"), Vec::<String>::new());
        let tracks = library.tracks_by("Artist", "Album");
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].path, PathBuf::from("/music/a.flac"));
        assert_eq!(tracks[0].title, "A");
    }

    #[test]
    fn search_matches_a_prefix_anywhere_in_the_row() {
        let library = library_with_one_artist();
        let hits = library.search("oth");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, PathBuf::from("/music/c.flac"));
        assert!(library.search("zzz").is_empty());
    }

    #[test]
    fn forgetting_a_directory_drops_its_rows() {
        let library = library_with_one_artist();
        library.forget(Path::new("/music"));
        assert!(library.artists().is_empty());
    }

    /// A library path may be a single file — the browser adds one that way —
    /// and the prefix query cannot address it: it matches `root/…`, which a
    /// file path followed by a separator never is.
    #[test]
    fn forgetting_a_single_file_drops_that_row() {
        let library = library_with_one_artist();
        library.forget(Path::new("/music/a.flac"));
        let tracks = library.tracks_by("Artist", "Album");
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].path, PathBuf::from("/music/b.flac"));
    }

    /// A partial walk must not prune. `seen` only lists what the walk could
    /// actually read, so an unreadable subtree would otherwise look like
    /// "these files disappeared" and every track under it would be deleted
    /// from the index — while still sitting on disk.
    #[test]
    fn an_incomplete_scan_does_not_prune() {
        let mut library = Library::in_memory();
        seed(&library, "/music/a.flac", "A", "Artist", "Album");
        seed(&library, "/music/sub/b.flac", "B", "Artist", "Album");

        library.finish_for_test(ScanUpdate::Finished {
            root: PathBuf::from("/music"),
            seen: vec![PathBuf::from("/music/a.flac")],
            scanned: 1,
            changed: 0,
            failed: 0,
            complete: false,
            cancelled: false,
        });

        assert!(
            library
                .db()
                .get_by_path("/music/sub/b.flac")
                .unwrap()
                .is_some(),
            "an incomplete scan must not delete tracks it could not enumerate"
        );
    }

    /// Companion guard: a trustworthy listing still prunes, so the rule above
    /// cannot silently degrade into "never remove anything".
    #[test]
    fn a_complete_scan_prunes_missing_tracks() {
        let mut library = Library::in_memory();
        seed(&library, "/music/a.flac", "A", "Artist", "Album");
        seed(&library, "/music/gone.flac", "Gone", "Artist", "Album");

        let notices = library.finish_for_test(ScanUpdate::Finished {
            root: PathBuf::from("/music"),
            seen: vec![PathBuf::from("/music/a.flac")],
            scanned: 1,
            changed: 0,
            failed: 0,
            complete: true,
            cancelled: false,
        });

        assert!(library.db().get_by_path("/music/a.flac").unwrap().is_some());
        assert!(
            library
                .db()
                .get_by_path("/music/gone.flac")
                .unwrap()
                .is_none(),
            "a complete listing must still prune deleted files"
        );
        assert_eq!(
            notices,
            vec![ScanNotice::Finished(ScanReport {
                scanned: 1,
                changed: 0,
                removed: 1,
                failed: 0,
                cancelled: false,
                complete: true,
                active: 0,
            })]
        );
    }

    /// A cancelled scan is also an untrustworthy listing.
    #[test]
    fn a_cancelled_scan_does_not_prune() {
        let mut library = Library::in_memory();
        seed(&library, "/music/a.flac", "A", "Artist", "Album");

        library.finish_for_test(ScanUpdate::Finished {
            root: PathBuf::from("/music"),
            seen: Vec::new(),
            scanned: 0,
            changed: 0,
            failed: 0,
            complete: true,
            cancelled: true,
        });

        assert!(library.db().get_by_path("/music/a.flac").unwrap().is_some());
    }

    /// A scan with no runtime to run on is refused, not panicked: the client's
    /// tests drive this code on a plain `#[test]` thread, where
    /// `spawn_blocking` would abort the process.
    #[test]
    fn a_scan_without_a_runtime_is_refused() {
        let mut library = Library::in_memory();
        assert!(!library.start_scan(Path::new("/music")));
        assert_eq!(library.scans_active(), 0);
    }

    /// The whole point of moving the scanner behind the socket: a walk of a
    /// real directory ends up in the index without the caller touching the
    /// database, and the caller learns how it went from the notices.
    #[tokio::test]
    async fn a_scan_indexes_the_directory_and_reports_when_it_is_done() {
        let mut library = Library::in_memory();
        library.attach_scanner(tokio::runtime::Handle::current());
        assert!(library.start_scan(Path::new("tests/fixtures")));
        assert_eq!(library.scans_active(), 1);

        let finished = library.wait_for_finish().await;
        assert!(
            matches!(
                finished,
                ScanNotice::Finished(ScanReport {
                    complete: true,
                    cancelled: false,
                    active: 0,
                    ..
                })
            ),
            "unexpected notice: {finished:?}"
        );
        // The three fixtures, by the artist their tags carry.
        assert!(
            library.artists().contains(&"Test Artist".to_string()),
            "artists: {:?}",
            library.artists()
        );
        let tracks = library.tracks_by("Test Artist", "Test Album");
        assert!(
            tracks
                .iter()
                .any(|t| t.path.to_string_lossy().ends_with("test.flac")),
            "tracks: {tracks:?}"
        );
    }

    /// A cancelled walk stops before it finishes the tree and says so.
    #[tokio::test]
    async fn a_cancelled_scan_reports_itself_cancelled() {
        let mut library = Library::in_memory();
        library.attach_scanner(tokio::runtime::Handle::current());
        library.start_scan(Path::new("tests/fixtures"));
        library.cancel_scans();

        let finished = library.wait_for_finish().await;
        assert!(
            matches!(
                finished,
                ScanNotice::Finished(ScanReport {
                    cancelled: true,
                    ..
                })
            ),
            "unexpected notice: {finished:?}"
        );
    }

    // ── The path list ──

    /// A private file for one test: the runtime root is per process, so two
    /// tests sharing a name would read each other's writes.
    fn paths_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tmper-library-{}-{name}.json", std::process::id()))
    }

    fn library_with_file(file: &Path) -> Library {
        Library::with_db(
            LibraryDb::open_memory().expect("in-memory library"),
            Some(file.to_path_buf()),
        )
    }

    /// A path that has left the disk is dropped on the way in, or every start
    /// would scan a directory that is not there and list it forever.
    #[test]
    fn the_path_list_round_trips_and_drops_what_is_gone() {
        let file = paths_file("round-trip");
        let _ = std::fs::remove_file(&file);
        let directory = std::env::temp_dir().join(format!("tmper-lib-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp dir");
        let song = PathBuf::from("tests/fixtures/test.flac");

        let mut library = library_with_file(&file);
        assert!(library.add_path(&directory));
        assert!(library.add_path(&song));
        assert!(library.add_path(Path::new("/definitely/missing")));

        let mut reloaded = library_with_file(&file);
        reloaded.load_paths();
        assert_eq!(reloaded.paths(), [directory.clone(), song.clone()]);

        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// Adding the same path twice is one entry: the panel lists each root
    /// once, and a second `a` on it means "scan again", not "two rows".
    #[test]
    fn adding_a_path_twice_keeps_one_entry() {
        let file = paths_file("dedup");
        let _ = std::fs::remove_file(&file);
        let mut library = library_with_file(&file);

        assert!(library.add_path(Path::new("/music")));
        assert!(!library.add_path(Path::new("/music")));
        assert_eq!(library.paths(), [PathBuf::from("/music")]);

        let _ = std::fs::remove_file(&file);
    }

    /// Removing a path is the list and the rows together — the browser's
    /// `Enter` means "this is not my library any more", not "hide the rows".
    #[test]
    fn removing_a_path_drops_it_and_its_rows() {
        let file = paths_file("remove");
        let _ = std::fs::remove_file(&file);
        let mut library = library_with_file(&file);
        seed(&library, "/music/a.flac", "A", "Artist", "Album");
        library.add_path(Path::new("/music"));
        assert_eq!(library.paths(), [PathBuf::from("/music")]);

        library.remove_path(Path::new("/music"));

        assert!(library.paths().is_empty());
        assert!(library.artists().is_empty());
        let _ = std::fs::remove_file(&file);
    }

    impl Library {
        /// Feed one scanner message straight in, as if a walk had sent it.
        fn finish_for_test(&mut self, update: ScanUpdate) -> Vec<ScanNotice> {
            self.scan_tx.send(update).expect("send");
            self.drain()
        }

        /// Poll the channel until a scan ends. The walk runs on a blocking
        /// thread, so there is no join handle to await — the notice is the
        /// completion signal, and it is bounded in time by the callers.
        async fn wait_for_finish(&mut self) -> ScanNotice {
            for _ in 0..600 {
                for notice in self.drain() {
                    if matches!(notice, ScanNotice::Finished(_)) {
                        return notice;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            panic!("the scan never finished");
        }
    }
}
