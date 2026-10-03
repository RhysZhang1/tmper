use crossterm::event::{KeyCode, KeyEvent};

use crate::app::App;
use crate::library::scanner::{ScanUpdate, ScannedTrack};
use crate::ui::views::library_view::LibraryPanel;

impl App {
    pub(crate) fn start_library_scan(&mut self, root: std::path::PathBuf) {
        let Some(tx) = self.library_scan_tx.clone() else {
            return;
        };
        let known = self
            .library_db
            .file_fingerprints_under(&root)
            .unwrap_or_default();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.library_scan_cancels.push(cancel.clone());
        self.library_scans_active += 1;
        self.ui_state.file_browser_state.scan_status = Some("scanning… c: cancel".into());
        tokio::task::spawn_blocking(move || {
            crate::library::scanner::scan_incremental(root, known, tx, cancel);
        });
    }

    pub(super) fn cancel_library_scan(&mut self) {
        for cancel in &self.library_scan_cancels {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub(crate) fn handle_scan_update(&mut self, update: ScanUpdate) {
        match update {
            ScanUpdate::Track(track) => self.index_scanned_track(*track),
            ScanUpdate::Progress { scanned, changed } => {
                self.ui_state.file_browser_state.scan_status =
                    Some(format!("scanned {scanned}, updated {changed} — c: cancel"));
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
                // Prune only when the listing is trustworthy. A partial walk
                // (unreadable subtree) reports a `seen` that is missing files
                // which still exist, and deleting those would silently drop
                // them from the library.
                let removed = if cancelled || !complete {
                    0
                } else {
                    self.library_db
                        .delete_missing_under(&root, &seen)
                        .unwrap_or(0)
                };
                self.library_scans_active = self.library_scans_active.saturating_sub(1);
                if self.library_scans_active == 0 {
                    self.library_scan_cancels.clear();
                }
                let status = if cancelled {
                    format!("scan cancelled after {scanned} files")
                } else if !complete {
                    format!(
                        "done: {scanned} scanned, {changed} updated, {failed} failed; \
                         some paths unreadable — nothing removed"
                    )
                } else {
                    format!(
                        "done: {scanned} scanned, {changed} updated, {removed} removed, {failed} failed"
                    )
                };
                self.ui_state.file_browser_state.scan_status =
                    Some(if self.library_scans_active > 0 {
                        format!(
                            "{status}; {} scan(s) still running — c: cancel",
                            self.library_scans_active
                        )
                    } else {
                        status
                    });
                self.refresh_library_artists();
            }
        }
    }

    fn index_scanned_track(&mut self, track: ScannedTrack) {
        let info = track.info;
        let path = info.path.to_string_lossy().to_string();
        if let Err(error) = self.library_db.upsert(
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

    /// Handle keyboard events in View 2 (Library Browser).
    pub(super) fn handle_library_key(&mut self, key: &KeyEvent) {
        let s = &mut self.ui_state.library_state;

        // ── Search mode: ALL keys go to input, Enter to search, Backspace to delete/exit ──
        if s.search_mode {
            match key.code {
                KeyCode::Enter => {
                    let query = s.search_query.clone();
                    if !query.is_empty() {
                        // Execute search
                        if let Ok(results) = self.library_db.search(&query) {
                            // Show results in track panel, switch focus to tracks
                            s.track_paths = results.iter().map(|t| t.path.clone()).collect();
                            s.track_titles = results
                                .iter()
                                .map(|t| {
                                    let artist = t.artist.as_deref().unwrap_or("?");
                                    format!("{}  -  {}", t.title, artist)
                                })
                                .collect();
                            s.track_index = 0;
                            s.scroll_tracks = 0;
                            s.focused = LibraryPanel::Tracks;
                        }
                        s.search_query.clear();
                        s.search_mode = false;
                    }
                }
                KeyCode::Backspace => {
                    if s.search_query.is_empty() {
                        // Exit search mode, restore browse view
                        s.search_mode = false;
                        s.focused = LibraryPanel::Artists;
                        self.refresh_library_artists();
                    } else {
                        s.search_query.pop();
                    }
                }
                KeyCode::Char(c) => {
                    s.search_query.push(c);
                }
                _ => {} // Ignore other keys (arrows, etc.) in search mode
            }
            return;
        }

        // ── Normal browse mode ──
        match key.code {
            KeyCode::Char('/') => {
                s.search_mode = true;
                s.search_query.clear();
            }
            KeyCode::Backspace => {
                // Drop borrow before calling refresh
                let _ = s;
                self.refresh_library_artists();
                self.ui_state.library_state.focused = LibraryPanel::Artists;
            }
            KeyCode::Char('h') | KeyCode::Left => {
                s.focused = match s.focused {
                    LibraryPanel::Tracks => LibraryPanel::Albums,
                    LibraryPanel::Albums => LibraryPanel::Artists,
                    LibraryPanel::Artists => LibraryPanel::Artists,
                };
            }
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Tab => {
                s.focused = match s.focused {
                    LibraryPanel::Artists => LibraryPanel::Albums,
                    LibraryPanel::Albums => LibraryPanel::Tracks,
                    LibraryPanel::Tracks => LibraryPanel::Tracks,
                };
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let mut need_albums = false;
                let mut need_tracks = false;
                {
                    let s = &mut self.ui_state.library_state;
                    match s.focused {
                        LibraryPanel::Artists => {
                            if s.artist_index + 1 < s.artists.len() {
                                s.artist_index += 1;
                                need_albums = true;
                            } else {
                                need_albums = false;
                            }
                            Self::clamp_scroll(
                                s.artist_index,
                                s.artists.len(),
                                &mut s.scroll_artists,
                            );
                        }
                        LibraryPanel::Albums => {
                            if s.album_index + 1 < s.albums.len() {
                                s.album_index += 1;
                                need_tracks = true;
                            } else {
                                need_tracks = false;
                            }
                            Self::clamp_scroll(s.album_index, s.albums.len(), &mut s.scroll_albums);
                        }
                        LibraryPanel::Tracks => {
                            if s.track_index + 1 < s.track_paths.len() {
                                s.track_index += 1;
                            }
                            Self::clamp_scroll(
                                s.track_index,
                                s.track_paths.len(),
                                &mut s.scroll_tracks,
                            );
                        }
                    }
                }
                if need_albums {
                    self.refresh_library_albums();
                }
                if need_tracks {
                    self.refresh_library_tracks();
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let mut need_albums = false;
                let mut need_tracks = false;
                {
                    let s = &mut self.ui_state.library_state;
                    match s.focused {
                        LibraryPanel::Artists => {
                            need_albums = s.artist_index > 0;
                            s.artist_index = s.artist_index.saturating_sub(1);
                            if s.artist_index < s.scroll_artists {
                                s.scroll_artists = s.artist_index;
                            }
                        }
                        LibraryPanel::Albums => {
                            need_tracks = s.album_index > 0;
                            s.album_index = s.album_index.saturating_sub(1);
                            if s.album_index < s.scroll_albums {
                                s.scroll_albums = s.album_index;
                            }
                        }
                        LibraryPanel::Tracks => {
                            s.track_index = s.track_index.saturating_sub(1);
                            if s.track_index < s.scroll_tracks {
                                s.scroll_tracks = s.track_index;
                            }
                        }
                    }
                }
                if need_albums {
                    self.refresh_library_albums();
                }
                if need_tracks {
                    self.refresh_library_tracks();
                }
            }
            KeyCode::Enter => {
                let s = &self.ui_state.library_state;
                if s.focused == LibraryPanel::Tracks && s.track_index < s.track_paths.len() {
                    let path = std::path::PathBuf::from(&s.track_paths[s.track_index]);
                    let _ = s;
                    self.load_and_play(&path);
                }
            }
            _ => {}
        }
    }

    fn clamp_scroll(index: usize, _total: usize, scroll: &mut usize) {
        let vis = 10;
        if index < *scroll {
            *scroll = index;
        }
        if index >= *scroll + vis {
            *scroll = index.saturating_sub(vis) + 1;
        }
    }

    pub(super) fn ensure_library_loaded(&mut self) {
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        for p in &self.ui_state.playlist_state.library_paths {
            if !paths.contains(p) {
                paths.push(p.clone());
            }
        }
        for p in &self.ui_state.file_browser_state.library_paths {
            if !paths.contains(p) {
                paths.push(p.clone());
            }
        }
        for t in &self.ui_state.player.tracks {
            if !paths.contains(&t.path) {
                paths.push(t.path.clone());
            }
        }
        for path in &paths {
            let path_str = path.to_string_lossy().to_string();
            if self
                .library_db
                .get_by_path(&path_str)
                .ok()
                .flatten()
                .is_some()
            {
                continue;
            }
            if let Ok(info) = crate::metadata::reader::read_metadata(path) {
                let mtime = std::fs::metadata(path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let _ = self.library_db.upsert(
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
        self.refresh_library_artists();
    }

    pub(super) fn refresh_library_artists(&mut self) {
        if let Ok(artists) = self.library_db.get_artists() {
            self.ui_state.library_state.artists = artists;
            self.ui_state.library_state.db_loaded = true;
            self.refresh_library_albums();
        }
    }

    fn refresh_library_albums(&mut self) {
        let artist = {
            let s = &self.ui_state.library_state;
            s.artists.get(s.artist_index).cloned()
        };
        if let Some(artist) = artist {
            if let Ok(albums) = self.library_db.get_albums_by_artist(&artist) {
                let s = &mut self.ui_state.library_state;
                s.albums = albums;
                s.album_index = 0;
                s.scroll_albums = 0;
                let _ = s;
                self.refresh_library_tracks();
                return;
            }
        }
        let s = &mut self.ui_state.library_state;
        s.albums.clear();
        s.track_paths.clear();
        s.track_titles.clear();
    }

    fn refresh_library_tracks(&mut self) {
        let (artist, album) = {
            let s = &self.ui_state.library_state;
            (
                s.artists.get(s.artist_index).cloned(),
                s.albums.get(s.album_index).cloned(),
            )
        };
        let s = &mut self.ui_state.library_state;
        match (artist, album) {
            (Some(artist), Some(album)) => {
                if let Ok(tracks) = self.library_db.get_tracks_by_album(&artist, &album) {
                    s.track_paths = tracks.iter().map(|t| t.path.clone()).collect();
                    s.track_titles = tracks.iter().map(|t| t.title.clone()).collect();
                } else {
                    s.track_paths.clear();
                    s.track_titles.clear();
                }
            }
            _ => {
                s.track_paths.clear();
                s.track_titles.clear();
            }
        }
        s.track_index = 0;
        s.scroll_tracks = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::handlers::test_support::test_app;
    use crate::library::database::LibraryDb;
    use crate::ui::views::library_view::LibraryPanel;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_library_key(&key(code));
    }

    fn press_char(app: &mut App, c: char) {
        press(app, KeyCode::Char(c));
    }

    fn insert_track(db: &LibraryDb, path: &str, title: &str, artist: &str, album: &str) {
        db.upsert(
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
        .expect("upsert failed");
    }

    fn seed_memory_library(app: &mut App) {
        app.library_db = LibraryDb::open_memory().expect("open memory db");
    }

    /// A partial walk must not prune. `seen` only lists what the walk could
    /// actually read, so an unreadable subtree would otherwise look like
    /// "these files disappeared" and every track under it would be deleted
    /// from the index — while still sitting on disk.
    #[test]
    fn test_incomplete_scan_does_not_prune_library() {
        let mut app = test_app();
        seed_memory_library(&mut app);
        insert_track(&app.library_db, "/music/a.flac", "A", "Artist", "Album");
        insert_track(&app.library_db, "/music/sub/b.flac", "B", "Artist", "Album");

        app.handle_scan_update(ScanUpdate::Finished {
            root: PathBuf::from("/music"),
            seen: vec![PathBuf::from("/music/a.flac")],
            scanned: 1,
            changed: 0,
            failed: 0,
            complete: false,
            cancelled: false,
        });

        assert!(
            app.library_db
                .get_by_path("/music/sub/b.flac")
                .unwrap()
                .is_some(),
            "an incomplete scan must not delete tracks it could not enumerate"
        );
    }

    /// Companion guard: a trustworthy listing still prunes, so the fix above
    /// cannot silently degrade into "never remove anything".
    #[test]
    fn test_complete_scan_prunes_missing_tracks() {
        let mut app = test_app();
        seed_memory_library(&mut app);
        insert_track(&app.library_db, "/music/a.flac", "A", "Artist", "Album");
        insert_track(
            &app.library_db,
            "/music/gone.flac",
            "Gone",
            "Artist",
            "Album",
        );

        app.handle_scan_update(ScanUpdate::Finished {
            root: PathBuf::from("/music"),
            seen: vec![PathBuf::from("/music/a.flac")],
            scanned: 1,
            changed: 0,
            failed: 0,
            complete: true,
            cancelled: false,
        });

        assert!(app
            .library_db
            .get_by_path("/music/a.flac")
            .unwrap()
            .is_some());
        assert!(
            app.library_db
                .get_by_path("/music/gone.flac")
                .unwrap()
                .is_none(),
            "a complete listing must still prune deleted files"
        );
    }

    // ── Focus navigation ──

    #[test]
    fn test_l_focus_moves_right_and_clamps() {
        let mut app = test_app();
        assert_eq!(app.ui_state.library_state.focused, LibraryPanel::Artists);
        press_char(&mut app, 'l');
        assert_eq!(app.ui_state.library_state.focused, LibraryPanel::Albums);
        press_char(&mut app, 'l');
        assert_eq!(app.ui_state.library_state.focused, LibraryPanel::Tracks);
        press_char(&mut app, 'l'); // clamps at Tracks
        assert_eq!(app.ui_state.library_state.focused, LibraryPanel::Tracks);
        // Tab behaves like right-arrow
        app.ui_state.library_state.focused = LibraryPanel::Albums;
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.ui_state.library_state.focused, LibraryPanel::Tracks);
    }

    #[test]
    fn test_h_focus_moves_left_and_clamps() {
        let mut app = test_app();
        app.ui_state.library_state.focused = LibraryPanel::Tracks;
        press_char(&mut app, 'h');
        assert_eq!(app.ui_state.library_state.focused, LibraryPanel::Albums);
        press_char(&mut app, 'h');
        assert_eq!(app.ui_state.library_state.focused, LibraryPanel::Artists);
        press_char(&mut app, 'h'); // clamps at Artists
        assert_eq!(app.ui_state.library_state.focused, LibraryPanel::Artists);
    }

    // ── Selection navigation ──

    #[test]
    fn test_j_navigates_artists_and_clamps() {
        let mut app = test_app();
        seed_memory_library(&mut app);
        {
            let s = &mut app.ui_state.library_state;
            s.artists = vec!["A".into(), "B".into(), "C".into()];
        }
        press_char(&mut app, 'j');
        assert_eq!(app.ui_state.library_state.artist_index, 1);
        press_char(&mut app, 'j');
        assert_eq!(app.ui_state.library_state.artist_index, 2);
        press_char(&mut app, 'j'); // clamp
        assert_eq!(app.ui_state.library_state.artist_index, 2);
    }

    #[test]
    fn test_k_navigates_artists_and_saturates() {
        let mut app = test_app();
        seed_memory_library(&mut app);
        {
            let s = &mut app.ui_state.library_state;
            s.artists = vec!["A".into(), "B".into(), "C".into()];
            s.artist_index = 2;
        }
        press_char(&mut app, 'k');
        assert_eq!(app.ui_state.library_state.artist_index, 1);
        press_char(&mut app, 'k');
        assert_eq!(app.ui_state.library_state.artist_index, 0);
        press_char(&mut app, 'k'); // saturate
        assert_eq!(app.ui_state.library_state.artist_index, 0);
    }

    #[test]
    fn test_down_key_moves_tracks_selection() {
        let mut app = test_app();
        {
            let s = &mut app.ui_state.library_state;
            s.focused = LibraryPanel::Tracks;
            s.track_paths = vec!["/a".into(), "/b".into(), "/c".into()];
        }
        press(&mut app, KeyCode::Down);
        assert_eq!(app.ui_state.library_state.track_index, 1);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.ui_state.library_state.track_index, 2);
    }

    // ── Search mode ──

    #[test]
    fn test_slash_enters_search_mode() {
        let mut app = test_app();
        press_char(&mut app, '/');
        let s = &app.ui_state.library_state;
        assert!(s.search_mode);
        assert!(s.search_query.is_empty());
    }

    #[test]
    fn test_search_typing_and_backspace() {
        let mut app = test_app();
        seed_memory_library(&mut app);
        press_char(&mut app, '/');
        press_char(&mut app, 'a');
        press_char(&mut app, 'b');
        assert_eq!(app.ui_state.library_state.search_query, "ab");
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.ui_state.library_state.search_query, "a");
        // Arrows are ignored while typing.
        press(&mut app, KeyCode::Down);
        assert_eq!(app.ui_state.library_state.search_query, "a");
        assert!(app.ui_state.library_state.search_mode);
    }

    #[test]
    fn test_search_backspace_at_empty_exits() {
        let mut app = test_app();
        seed_memory_library(&mut app);
        press_char(&mut app, '/');
        press(&mut app, KeyCode::Backspace);
        let s = &app.ui_state.library_state;
        assert!(!s.search_mode);
        assert_eq!(s.focused, LibraryPanel::Artists);
        assert!(s.db_loaded); // refresh ran against the memory db
    }

    #[test]
    fn test_search_enter_populates_tracks() {
        let mut app = test_app();
        seed_memory_library(&mut app);
        insert_track(
            &app.library_db,
            "/music/alpha.flac",
            "Hello World",
            "John",
            "Debut",
        );
        insert_track(
            &app.library_db,
            "/music/beta.flac",
            "Goodbye",
            "Jane",
            "Farewell",
        );

        press_char(&mut app, '/');
        for c in ['h', 'e', 'l', 'l', 'o'] {
            press_char(&mut app, c);
        }
        press(&mut app, KeyCode::Enter);

        let s = &app.ui_state.library_state;
        assert!(!s.search_mode);
        assert!(s.search_query.is_empty());
        assert_eq!(s.focused, LibraryPanel::Tracks);
        assert_eq!(s.track_paths, vec!["/music/alpha.flac".to_string()]);
        assert_eq!(s.track_titles, vec!["Hello World  -  John".to_string()]);
        assert_eq!(s.track_index, 0);
    }

    #[test]
    fn test_search_enter_no_match_clears_tracks() {
        let mut app = test_app();
        seed_memory_library(&mut app);
        insert_track(
            &app.library_db,
            "/music/alpha.flac",
            "Hello World",
            "John",
            "Debut",
        );

        press_char(&mut app, '/');
        for c in ['z', 'z', 'z'] {
            press_char(&mut app, c);
        }
        press(&mut app, KeyCode::Enter);
        let s = &app.ui_state.library_state;
        assert!(s.track_paths.is_empty());
        assert_eq!(s.focused, LibraryPanel::Tracks);
    }

    #[test]
    fn test_search_empty_query_enter_is_noop() {
        let mut app = test_app();
        seed_memory_library(&mut app);
        press_char(&mut app, '/');
        press(&mut app, KeyCode::Enter); // empty query → stays in search mode
        let s = &app.ui_state.library_state;
        assert!(s.search_mode);
    }

    // ── clamp_scroll ──

    #[test]
    fn test_clamp_scroll_forward() {
        let mut scroll = 0usize;
        App::clamp_scroll(15, 100, &mut scroll);
        assert_eq!(scroll, 6); // 15 >= 0 + 10 → 15 - 10 + 1
    }

    #[test]
    fn test_clamp_scroll_backward() {
        let mut scroll = 5usize;
        App::clamp_scroll(3, 100, &mut scroll);
        assert_eq!(scroll, 3);
    }

    #[test]
    fn test_clamp_scroll_within_window() {
        let mut scroll = 0usize;
        App::clamp_scroll(4, 100, &mut scroll);
        assert_eq!(scroll, 0);
    }

    // ── ensure_library_loaded ──

    #[test]
    fn test_ensure_library_loaded_upserts_fixture() {
        let mut app = test_app();
        seed_memory_library(&mut app);
        let fixture = PathBuf::from("tests/fixtures/test.flac");
        app.ui_state.playlist_state.library_paths = vec![fixture.clone()];
        app.ensure_library_loaded();
        let row = app
            .library_db
            .get_by_path(&fixture.to_string_lossy())
            .unwrap();
        assert!(row.is_some());
        assert_eq!(row.unwrap().artist.as_deref(), Some("Test Artist"));
        let s = &app.ui_state.library_state;
        assert!(s.db_loaded);
        assert_eq!(s.artists, vec!["Test Artist".to_string()]);
    }

    #[test]
    fn test_ensure_library_loaded_dedups_paths() {
        let mut app = test_app();
        seed_memory_library(&mut app);
        let fixture = PathBuf::from("tests/fixtures/test.flac");
        // Same path in three places — must be inserted once.
        app.ui_state.playlist_state.library_paths = vec![fixture.clone()];
        app.ui_state.file_browser_state.library_paths = vec![fixture.clone()];
        app.ui_state.player.tracks.push(crate::ui::TrackDisplay {
            path: fixture.clone(),
            title: "Test Song".into(),
            artist: "Test Artist".into(),
            duration_secs: 2.0,
        });
        app.ensure_library_loaded();
        assert_eq!(app.library_db.count().unwrap(), 1);
    }

    // ── Enter plays track ──

    #[tokio::test]
    async fn test_enter_track_plays_fixture() {
        let mut app = test_app();
        seed_memory_library(&mut app);
        let fixture = PathBuf::from("tests/fixtures/test.flac");
        app.library_db
            .upsert(
                &fixture.to_string_lossy(),
                "Test Song",
                Some("Test Artist"),
                Some("Test Album"),
                None,
                Some(3),
                Some(1),
                Some("Rock"),
                Some(2024),
                2.0,
                0,
                44100,
                2,
                "FLAC",
                20000,
                1000,
            )
            .unwrap();
        {
            let s = &mut app.ui_state.library_state;
            s.focused = LibraryPanel::Tracks;
            s.track_paths = vec![fixture.to_string_lossy().to_string()];
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.ui_state.player.playing_index, Some(0));
        assert_eq!(app.ui_state.player.selected_index, 0);
        // Stop the engine so the async decode task drains.
        app.engine.stop();
    }
}
