//! The library view: ask, and render the answer.
//!
//! Every question here goes to the player as a [`Request`] and every answer
//! comes back through `App::apply_event` — the client holds no index and
//! touches no database. The two halves sit in one file because they are one
//! exchange: `refresh_library_albums` asks about the artist the cursor is on,
//! and `apply_library_albums` is what happens when the answer describes an
//! artist the cursor is still on.

use crossterm::event::{KeyCode, KeyEvent};

use crate::app::App;
use crate::ipc::proto::{Request, ScanReport, TrackLine};
use crate::ui::views::library_view::LibraryPanel;

impl App {
    /// Scan a directory into the index, in the daemon, where the index lives.
    pub(crate) fn start_library_scan(&mut self, root: std::path::PathBuf) {
        self.ui_state.file_browser_state.scan_status = Some("scanning… c: cancel".into());
        self.dispatch(Request::ScanLibrary { root });
    }

    pub(super) fn cancel_library_scan(&mut self) {
        self.dispatch(Request::CancelScan);
    }

    /// Make sure everything the user can already see is in the index: the
    /// library paths, and (on the daemon's side) the queue. Files that were
    /// played but never scanned belong in the collection too.
    pub(super) fn ensure_library_loaded(&mut self) {
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        for path in self
            .ui_state
            .playlist_state
            .library_paths
            .iter()
            .chain(self.ui_state.file_browser_state.library_paths.iter())
        {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
        self.dispatch(Request::IndexPaths { paths });
        self.refresh_library_artists();
    }

    pub(super) fn refresh_library_artists(&mut self) {
        self.dispatch(Request::LibraryArtists);
    }

    fn refresh_library_albums(&mut self) {
        if let Some(artist) = self.selected_artist() {
            self.dispatch(Request::LibraryAlbums { artist });
        }
    }

    fn refresh_library_tracks(&mut self) {
        let (artist, album) = self.selected_album();
        if let (Some(artist), Some(album)) = (artist, album) {
            self.dispatch(Request::LibraryTracks { artist, album });
        } else {
            self.clear_tracks();
        }
    }

    fn selected_artist(&self) -> Option<String> {
        let s = &self.ui_state.library_state;
        s.artists.get(s.artist_index).cloned()
    }

    fn selected_album(&self) -> (Option<String>, Option<String>) {
        let s = &self.ui_state.library_state;
        (
            s.artists.get(s.artist_index).cloned(),
            s.albums.get(s.album_index).cloned(),
        )
    }

    fn clear_tracks(&mut self) {
        let s = &mut self.ui_state.library_state;
        s.track_paths.clear();
        s.track_titles.clear();
        s.track_index = 0;
        s.scroll_tracks = 0;
    }

    // ── The answers ──

    pub(crate) fn apply_library_artists(&mut self, artists: Vec<String>) {
        let s = &mut self.ui_state.library_state;
        s.artists = artists;
        s.db_loaded = true;
        // A cursor left past the end — the list shrank under it — would point
        // at nothing and leave both lower panels empty.
        s.artist_index = s.artist_index.min(s.artists.len().saturating_sub(1));
        self.refresh_library_albums();
    }

    pub(crate) fn apply_library_albums(&mut self, artist: String, albums: Vec<String>) {
        // The answer to a question about an artist the cursor has left. Two
        // `j` presses put two of these on the wire; only the last one is about
        // where the user is.
        if self.selected_artist().as_deref() != Some(artist.as_str()) {
            return;
        }
        let s = &mut self.ui_state.library_state;
        s.albums = albums;
        s.album_index = 0;
        s.scroll_albums = 0;
        self.refresh_library_tracks();
    }

    pub(crate) fn apply_library_tracks(
        &mut self,
        artist: String,
        album: String,
        tracks: Vec<TrackLine>,
    ) {
        let (current_artist, current_album) = self.selected_album();
        if current_artist.as_deref() != Some(artist.as_str())
            || current_album.as_deref() != Some(album.as_str())
        {
            return;
        }
        let s = &mut self.ui_state.library_state;
        s.track_paths = tracks
            .iter()
            .map(|track| track.path.to_string_lossy().to_string())
            .collect();
        s.track_titles = tracks.into_iter().map(|track| track.title).collect();
        s.track_index = 0;
        s.scroll_tracks = 0;
    }

    pub(crate) fn apply_search_results(&mut self, query: String, tracks: Vec<TrackLine>) {
        // The key the client kept is the query it last submitted; anything
        // else is a reply to a search the user has already replaced.
        if self.ui_state.library_state.search_query != query {
            return;
        }
        let s = &mut self.ui_state.library_state;
        s.track_paths = tracks
            .iter()
            .map(|track| track.path.to_string_lossy().to_string())
            .collect();
        // A search mixes artists together, so the row has to say which one
        // it is. The browse panels do not: there, every row is the artist
        // the cursor is already on.
        s.track_titles = tracks
            .into_iter()
            .map(|track| {
                let artist = if track.artist.is_empty() {
                    "?"
                } else {
                    track.artist.as_str()
                };
                format!("{}  -  {}", track.title, artist)
            })
            .collect();
        s.track_index = 0;
        s.scroll_tracks = 0;
        s.focused = LibraryPanel::Tracks;
    }

    // ── Scan progress ──

    pub(crate) fn apply_scan_progress(&mut self, scanned: usize, changed: usize) {
        self.ui_state.file_browser_state.scan_status =
            Some(format!("scanned {scanned}, updated {changed} — c: cancel"));
    }

    pub(crate) fn apply_scan_finished(&mut self, report: ScanReport) {
        let ScanReport {
            scanned,
            changed,
            removed,
            failed,
            cancelled,
            complete,
            active,
        } = report;
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
        self.ui_state.file_browser_state.scan_status = Some(if active > 0 {
            format!("{status}; {active} scan(s) still running — c: cancel")
        } else {
            status
        });
        // A scan that changed nothing left the panels describing the truth
        // already; one that did gets them asked again.
        if changed > 0 || removed > 0 {
            self.refresh_library_artists();
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
                        // The query stays in the field after Enter: the search
                        // bar is closed (`search_mode` is what draws it), and
                        // the string is the key the answer comes back with.
                        s.search_mode = false;
                        let _ = s;
                        self.dispatch(Request::SearchLibrary { query });
                        return;
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
                    self.dispatch(Request::Play { path });
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::handlers::test_support::test_app;
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

    /// Put a row in the player's index. The library lives on the daemon's side
    /// of the seam now, so a test seeds it the way a scan would: by writing a
    /// row into the index the daemon reads from.
    fn insert_track(app: &mut App, path: &str, title: &str, artist: &str, album: &str) {
        app.player_mut()
            .library_mut()
            .db_mut()
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
            .expect("upsert failed");
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
        press_char(&mut app, '/');
        press(&mut app, KeyCode::Backspace);
        let s = &app.ui_state.library_state;
        assert!(!s.search_mode);
        assert_eq!(s.focused, LibraryPanel::Artists);
        assert!(s.db_loaded); // the artist answer came back
    }

    #[test]
    fn test_search_enter_populates_tracks() {
        let mut app = test_app();
        insert_track(
            &mut app,
            "/music/alpha.flac",
            "Hello World",
            "John",
            "Debut",
        );
        insert_track(&mut app, "/music/beta.flac", "Goodbye", "Jane", "Farewell");

        press_char(&mut app, '/');
        for c in ['h', 'e', 'l', 'l', 'o'] {
            press_char(&mut app, c);
        }
        press(&mut app, KeyCode::Enter);

        let s = &app.ui_state.library_state;
        assert!(!s.search_mode);
        assert_eq!(s.search_query, "hello"); // the key the answer matched on
        assert_eq!(s.focused, LibraryPanel::Tracks);
        assert_eq!(s.track_paths, vec!["/music/alpha.flac".to_string()]);
        assert_eq!(s.track_titles, vec!["Hello World  -  John".to_string()]);
        assert_eq!(s.track_index, 0);
    }

    #[test]
    fn test_search_enter_no_match_clears_tracks() {
        let mut app = test_app();
        insert_track(
            &mut app,
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
        press_char(&mut app, '/');
        press(&mut app, KeyCode::Enter); // empty query → stays in search mode
        let s = &app.ui_state.library_state;
        assert!(s.search_mode);
    }

    /// A reply to a search the user has already replaced must not repaint the
    /// panel: the query is the key, and only the newest one matches.
    #[test]
    fn test_a_stale_search_reply_is_dropped() {
        let mut app = test_app();
        app.ui_state.library_state.search_query = "newer".into();
        app.ui_state.library_state.track_paths = vec!["/kept".into()];
        app.apply_search_results(
            "older".into(),
            vec![TrackLine {
                path: PathBuf::from("/stale"),
                title: "Stale".into(),
                artist: "Nobody".into(),
            }],
        );
        assert_eq!(
            app.ui_state.library_state.track_paths,
            vec!["/kept".to_string()]
        );
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
        let fixture = PathBuf::from("tests/fixtures/test.flac");
        app.ui_state.playlist_state.library_paths = vec![fixture.clone()];
        app.ensure_library_loaded();
        let row = app
            .player()
            .library()
            .db()
            .get_by_path(&fixture.to_string_lossy())
            .unwrap();
        assert!(row.is_some());
        assert_eq!(row.unwrap().artist.as_deref(), Some("Test Artist"));
        let s = &app.ui_state.library_state;
        assert!(s.db_loaded);
        assert_eq!(s.artists, vec!["Test Artist".to_string()]);
    }

    /// The same path reached three ways is one row — the index is keyed by
    /// path, and the request may list it more than once.
    #[test]
    fn test_ensure_library_loaded_dedups_paths() {
        let mut app = test_app();
        let fixture = PathBuf::from("tests/fixtures/test.flac");
        app.ui_state.playlist_state.library_paths = vec![fixture.clone()];
        app.ui_state.file_browser_state.library_paths = vec![fixture.clone()];
        app.dispatch(Request::IndexPaths {
            paths: vec![fixture.clone(), fixture.clone()],
        });
        assert_eq!(app.player().library().db().count().unwrap(), 1);
    }

    /// Everything the queue holds is indexed too, without the client naming
    /// it: the queue is the daemon's, and so is the index.
    #[tokio::test]
    async fn test_the_queue_is_indexed_without_being_asked() {
        let mut app = test_app();
        let fixture = PathBuf::from("tests/fixtures/test.flac");
        app.dispatch(Request::QueuePush {
            path: fixture.clone(),
        });
        app.dispatch(Request::IndexPaths { paths: Vec::new() });
        assert!(
            app.player()
                .library()
                .db()
                .get_by_path(&fixture.to_string_lossy())
                .unwrap()
                .is_some(),
            "a queued track belongs in the library"
        );
        app.stop_player();
    }

    // ── Enter plays track ──

    #[tokio::test]
    async fn test_enter_track_plays_fixture() {
        let mut app = test_app();
        let fixture = PathBuf::from("tests/fixtures/test.flac");
        {
            let s = &mut app.ui_state.library_state;
            s.focused = LibraryPanel::Tracks;
            s.track_paths = vec![fixture.to_string_lossy().to_string()];
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.ui_state.player.playing_index, Some(0));
        assert_eq!(app.ui_state.player.selected_index, 0);
        // Stop the player so the async decode task drains.
        app.stop_player();
    }
}
