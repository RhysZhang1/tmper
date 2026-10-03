use crossterm::event::{KeyCode, KeyEvent};

use crate::app::App;
use crate::ipc::proto::Request;
use crate::playlist::PlaylistData;
use crate::ui::views::playlist_view::{InsertMode, LineTarget, PlaylistFlatModel, PlaylistPanel};

impl App {
    /// Mirror the daemon's playlist store.
    ///
    /// The store is pushed whole on every edit, so this is a replace, not a
    /// patch — the client never holds a playlist the daemon does not. The
    /// cursors are clamped here rather than at the keypress that asked for the
    /// edit: the store can also shrink because *another* client deleted or
    /// emptied something, and those edits arrive with no keypress at all.
    pub(crate) fn apply_playlists(&mut self, playlists: Vec<PlaylistData>) {
        let ui = &mut self.ui_state;
        ui.playlist_state.playlists = playlists;
        {
            let state = &mut ui.playlist_state;
            if state
                .expanded_playlist
                .is_some_and(|index| index >= state.playlists.len())
            {
                state.expanded_playlist = None;
            }
            let total =
                PlaylistFlatModel::new(&state.playlists, state.expanded_playlist).total_lines();
            state.selected_playlist = state.selected_playlist.min(total.saturating_sub(1));
        }
        // An open playlist that is gone — deleted here or by another client —
        // takes the highlight with it.
        if ui
            .active_playlist
            .is_some_and(|id| !ui.playlist_state.playlists.iter().any(|pl| pl.id == id))
        {
            ui.active_playlist = None;
        }
    }

    /// Tell the player which playlist `Next` should walk. `None` means the
    /// queue, which is also what an empty playlist means.
    pub(super) fn set_active_playlist(&mut self, id: Option<u64>) {
        self.ui_state.active_playlist = id;
        self.dispatch(Request::SetActivePlaylist { id });
    }

    /// A playlist the user just made: open it and put the cursor on it, so the
    /// next thing they do — adding songs from the library panel — lands in the
    /// playlist they just named.
    pub(crate) fn apply_playlist_added(&mut self, id: u64) {
        let state = &mut self.ui_state.playlist_state;
        let Some(index) = state.playlists.iter().position(|pl| pl.id == id) else {
            return;
        };
        state.expanded_playlist = Some(index);
        if let Some(line) =
            PlaylistFlatModel::new(&state.playlists, Some(index)).line_of_playlist(index)
        {
            state.selected_playlist = line;
        }
        Self::clamp_playlist_scroll(state, self.ui_state.visible_rows.get());
    }

    /// Where an export landed. One file is the answer to `e` and `:export`;
    /// several are the answer to "export everything", which reports a count.
    pub(crate) fn apply_playlists_exported(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        failed: usize,
    ) {
        let message = match (paths.len(), failed) {
            // Only reachable for a single export naming a playlist the store
            // no longer has: "export everything" over an empty store is
            // refused before it is sent.
            (0, _) => "Export failed: no such playlist".to_string(),
            (1, 0) => format!("Exported to {}", paths[0].display()),
            (written, failed) => format!("已导出 {written}/{} 个歌单到 data/", written + failed),
        };
        self.ui_state.notification = Some((message, std::time::Instant::now()));
    }

    /// Clamp scroll offset and cursor after a playlist structural change.
    pub(super) fn clamp_playlist_scroll(
        state: &mut crate::ui::views::playlist_view::PlaylistManagerState,
        vis: usize,
    ) {
        let total = PlaylistFlatModel::new(&state.playlists, state.expanded_playlist).total_lines();
        if state.selected_playlist >= total {
            state.selected_playlist = total.saturating_sub(1);
        }
        if state.selected_playlist < state.scroll_playlists {
            state.scroll_playlists = state.selected_playlist;
        }
        if state.selected_playlist >= state.scroll_playlists + vis {
            state.scroll_playlists = state.selected_playlist.saturating_sub(vis) + 1;
        }
    }

    /// Handle all keyboard events in View 5 (Playlist Manager).
    pub(super) fn handle_playlist_key(&mut self, key: &KeyEvent) {
        let state = &mut self.ui_state.playlist_state;

        // ── Insert mode ──
        if let InsertMode::Typing(ref s) = state.insert_mode {
            match key.code {
                KeyCode::Enter => {
                    let name = s.clone();
                    state.insert_mode = InsertMode::Off;
                    if !name.is_empty() {
                        // The store is the daemon's, so the new playlist comes
                        // back as an event rather than appearing here — which
                        // is also where its id comes from.
                        self.dispatch(Request::PlaylistCreate { name });
                    }
                }
                KeyCode::Esc => {
                    state.insert_mode = InsertMode::Off;
                }
                KeyCode::Backspace => {
                    let mut new_s = s.clone();
                    new_s.pop();
                    state.insert_mode = InsertMode::Typing(new_s);
                }
                KeyCode::Char(c) => {
                    let mut new_s = s.clone();
                    new_s.push(c);
                    state.insert_mode = InsertMode::Typing(new_s);
                }
                _ => {}
            }
            return;
        }

        state.notification = None;

        // Export expanded playlist to M3U. The file is written by the daemon —
        // the store's owner — so the path to report arrives with the answer.
        if key.code == KeyCode::Char('e') && key.modifiers.is_empty() {
            match state
                .expanded_playlist
                .and_then(|ep| state.playlists.get(ep))
                .map(|playlist| playlist.id)
            {
                Some(id) => self.dispatch(Request::PlaylistExport { id: Some(id) }),
                None => {
                    state.notification = Some((
                        "Expand a playlist first, then press 'e' to export".to_string(),
                        std::time::Instant::now(),
                    ));
                }
            }
            return;
        }

        match key.code {
            KeyCode::Tab | KeyCode::Char('l') | KeyCode::Right => {
                state.focused = match state.focused {
                    PlaylistPanel::Library => PlaylistPanel::Playlists,
                    PlaylistPanel::Playlists => PlaylistPanel::Library,
                };
            }
            KeyCode::Char('h') | KeyCode::Left => {
                state.focused = PlaylistPanel::Library;
            }
            KeyCode::Char('j') | KeyCode::Down => match state.focused {
                PlaylistPanel::Library => {
                    let max = state.library_paths.len().saturating_sub(1);
                    state.selected_library_song = (state.selected_library_song + 1).min(max);
                    let sel = state.selected_library_song;
                    let vis = self.ui_state.visible_rows.get();
                    if sel < state.scroll_library {
                        state.scroll_library = sel;
                    }
                    if sel >= state.scroll_library + vis {
                        state.scroll_library = sel.saturating_sub(vis) + 1;
                    }
                }
                PlaylistPanel::Playlists => {
                    let model = PlaylistFlatModel::new(&state.playlists, state.expanded_playlist);
                    let max_vis = model.total_lines().saturating_sub(1);
                    if state.selected_playlist < max_vis {
                        state.selected_playlist += 1;
                    }
                    Self::clamp_playlist_scroll(state, self.ui_state.visible_rows.get());
                }
            },
            KeyCode::Char('k') | KeyCode::Up => match state.focused {
                PlaylistPanel::Library => {
                    state.selected_library_song = state.selected_library_song.saturating_sub(1);
                    let sel = state.selected_library_song;
                    let vis = self.ui_state.visible_rows.get();
                    if sel < state.scroll_library {
                        state.scroll_library = sel;
                    }
                    if sel >= state.scroll_library + vis {
                        state.scroll_library = sel.saturating_sub(vis) + 1;
                    }
                }
                PlaylistPanel::Playlists => {
                    if state.selected_playlist > 0 {
                        state.selected_playlist -= 1;
                    }
                    Self::clamp_playlist_scroll(state, self.ui_state.visible_rows.get());
                }
            },
            KeyCode::Enter => match state.focused {
                PlaylistPanel::Library => {
                    if let Some(ep) = state.expanded_playlist {
                        let lib_idx = state.selected_library_song;
                        // The duplicate check is a courtesy — it saves a round
                        // trip on a keystroke that would change nothing. The
                        // store refuses the duplicate either way.
                        if lib_idx < state.library_paths.len() {
                            let path = state.library_paths[lib_idx].clone();
                            if !state.playlists[ep].songs.contains(&path) {
                                let id = state.playlists[ep].id;
                                self.dispatch(Request::PlaylistAddSong { id, path });
                            }
                        }
                    } else {
                        state.notification =
                            Some(("请先展开一个歌单".to_string(), std::time::Instant::now()));
                    }
                }
                PlaylistPanel::Playlists => {
                    let model = PlaylistFlatModel::new(&state.playlists, state.expanded_playlist);
                    match model.resolve(state.selected_playlist) {
                        LineTarget::AddNew => {
                            state.insert_mode = InsertMode::Typing(String::new());
                        }
                        LineTarget::PlaylistName(i) => {
                            if state.expanded_playlist == Some(i) {
                                state.expanded_playlist = None;
                            } else {
                                state.expanded_playlist = Some(i);
                            }
                            let new_model =
                                PlaylistFlatModel::new(&state.playlists, state.expanded_playlist);
                            if let Some(new_line) = new_model.line_of_playlist(i) {
                                state.selected_playlist = new_line;
                            }
                            Self::clamp_playlist_scroll(state, self.ui_state.visible_rows.get());
                        }
                        LineTarget::Song {
                            playlist,
                            song_index,
                        } => {
                            if playlist < state.playlists.len()
                                && song_index < state.playlists[playlist].songs.len()
                            {
                                let id = state.playlists[playlist].id;
                                self.dispatch(Request::PlaylistRemoveSong {
                                    id,
                                    index: song_index,
                                });
                                // The cursor is clamped when the new store
                                // arrives, which is also the only moment the
                                // row count is known to have changed.
                            }
                        }
                        LineTarget::Empty(_) => { /* "(empty)" — no-op */ }
                    }
                }
            },
            _ => {}
        }
    }

    pub(super) fn enter_playlist_view(&mut self) {
        self.ui_state.playlist_state.library_paths = self
            .ui_state
            .player
            .tracks
            .iter()
            .map(|t| t.path.clone())
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::handlers::test_support::{pl, seed_playlists, test_app};
    use crate::ui::views::playlist_view::{
        InsertMode, PlaylistFlatModel, PlaylistManagerState, PlaylistPanel,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_playlist_key(&key(code));
    }

    fn press_char(app: &mut App, c: char) {
        press(app, KeyCode::Char(c));
    }

    // ── Focus switching ──

    #[test]
    fn test_tab_toggles_focus() {
        let mut app = test_app();
        assert_eq!(app.ui_state.playlist_state.focused, PlaylistPanel::Library);
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.ui_state.playlist_state.focused,
            PlaylistPanel::Playlists
        );
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.ui_state.playlist_state.focused, PlaylistPanel::Library);
        press_char(&mut app, 'l');
        assert_eq!(
            app.ui_state.playlist_state.focused,
            PlaylistPanel::Playlists
        );
        press(&mut app, KeyCode::Right);
        assert_eq!(app.ui_state.playlist_state.focused, PlaylistPanel::Library);
    }

    #[test]
    fn test_h_forces_library_focus() {
        let mut app = test_app();
        app.ui_state.playlist_state.focused = PlaylistPanel::Playlists;
        press_char(&mut app, 'h');
        assert_eq!(app.ui_state.playlist_state.focused, PlaylistPanel::Library);
        press(&mut app, KeyCode::Left);
        assert_eq!(app.ui_state.playlist_state.focused, PlaylistPanel::Library);
    }

    // ── Library panel navigation ──

    #[test]
    fn test_j_k_navigate_library_panel() {
        let mut app = test_app();
        {
            let s = &mut app.ui_state.playlist_state;
            s.focused = PlaylistPanel::Library;
            s.library_paths = vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c"),
            ];
        }
        press_char(&mut app, 'j');
        assert_eq!(app.ui_state.playlist_state.selected_library_song, 1);
        press_char(&mut app, 'j');
        press_char(&mut app, 'j');
        assert_eq!(app.ui_state.playlist_state.selected_library_song, 2);
        press_char(&mut app, 'k');
        assert_eq!(app.ui_state.playlist_state.selected_library_song, 1);
        press_char(&mut app, 'k');
        press_char(&mut app, 'k');
        assert_eq!(app.ui_state.playlist_state.selected_library_song, 0);
    }

    // ── Playlists panel navigation ──

    #[test]
    fn test_j_k_navigate_playlists_panel() {
        let mut app = test_app();
        {
            let s = &mut app.ui_state.playlist_state;
            s.focused = PlaylistPanel::Playlists;
            s.playlists = vec![pl("A", &[]), pl("B", &[])]; // total 3 lines
        }
        press_char(&mut app, 'j');
        assert_eq!(app.ui_state.playlist_state.selected_playlist, 1);
        press_char(&mut app, 'j');
        assert_eq!(app.ui_state.playlist_state.selected_playlist, 2);
        press_char(&mut app, 'j'); // clamp
        assert_eq!(app.ui_state.playlist_state.selected_playlist, 2);
        press_char(&mut app, 'k');
        assert_eq!(app.ui_state.playlist_state.selected_playlist, 1);
        press_char(&mut app, 'k');
        press_char(&mut app, 'k'); // saturate
        assert_eq!(app.ui_state.playlist_state.selected_playlist, 0);
    }

    // ── Enter actions ──

    #[test]
    fn test_enter_addnew_enters_insert_mode() {
        let mut app = test_app();
        {
            let s = &mut app.ui_state.playlist_state;
            s.focused = PlaylistPanel::Playlists;
            s.selected_playlist = 0; // "..."
        }
        press(&mut app, KeyCode::Enter);
        assert!(matches!(
            app.ui_state.playlist_state.insert_mode,
            InsertMode::Typing(ref s) if s.is_empty()
        ));
    }

    #[test]
    fn test_insert_mode_creates_playlist() {
        let mut app = test_app();
        {
            let s = &mut app.ui_state.playlist_state;
            s.focused = PlaylistPanel::Playlists;
            s.selected_playlist = 0;
        }
        press(&mut app, KeyCode::Enter); // start typing
        press_char(&mut app, 'M');
        press_char(&mut app, 'y');
        press(&mut app, KeyCode::Backspace);
        assert!(matches!(
            app.ui_state.playlist_state.insert_mode,
            InsertMode::Typing(ref s) if s == "M"
        ));
        press_char(&mut app, 'y');
        press(&mut app, KeyCode::Enter);

        let s = &app.ui_state.playlist_state;
        assert_eq!(s.insert_mode, InsertMode::Off);
        assert_eq!(s.playlists.len(), 1);
        assert_eq!(s.playlists[0].name, "My");
        assert_eq!(s.expanded_playlist, Some(0));
        // Line 0 is the "…" row the name was typed into, line 1 is the
        // playlist that came back — and the cursor follows it there, so the
        // next Enter fills the playlist the user just named rather than
        // opening a second one.
        assert_eq!(s.selected_playlist, 1);
    }

    #[test]
    fn test_insert_mode_esc_cancels() {
        let mut app = test_app();
        {
            let s = &mut app.ui_state.playlist_state;
            s.focused = PlaylistPanel::Playlists;
        }
        press(&mut app, KeyCode::Enter);
        press_char(&mut app, 'X');
        press(&mut app, KeyCode::Esc);
        let s = &app.ui_state.playlist_state;
        assert_eq!(s.insert_mode, InsertMode::Off);
        assert!(s.playlists.is_empty());
    }

    #[test]
    fn test_insert_mode_empty_enter_is_noop() {
        let mut app = test_app();
        {
            let s = &mut app.ui_state.playlist_state;
            s.focused = PlaylistPanel::Playlists;
        }
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter); // empty name
        let s = &app.ui_state.playlist_state;
        assert_eq!(s.insert_mode, InsertMode::Off);
        assert!(s.playlists.is_empty());
    }

    #[test]
    fn test_enter_library_adds_song_to_expanded() {
        let mut app = test_app();
        seed_playlists(&mut app, vec![pl("P", &[])]);
        {
            let s = &mut app.ui_state.playlist_state;
            s.focused = PlaylistPanel::Library;
            s.expanded_playlist = Some(0);
            s.library_paths = vec![
                PathBuf::from("/music/a.flac"),
                PathBuf::from("/music/b.flac"),
            ];
            s.selected_library_song = 0;
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.ui_state.playlist_state.playlists[0].songs,
            vec![PathBuf::from("/music/a.flac")]
        );
        // Duplicate guard: pressing Enter again must not add a duplicate.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.ui_state.playlist_state.playlists[0].songs.len(), 1);
    }

    #[test]
    fn test_enter_library_without_expanded_notifies() {
        let mut app = test_app();
        {
            let s = &mut app.ui_state.playlist_state;
            s.focused = PlaylistPanel::Library;
            s.playlists = vec![pl("P", &[])];
            s.expanded_playlist = None;
            s.library_paths = vec![PathBuf::from("/music/a.flac")];
        }
        press(&mut app, KeyCode::Enter);
        assert!(app.ui_state.playlist_state.notification.is_some());
    }

    #[test]
    fn test_enter_playlist_name_toggles_expand() {
        let mut app = test_app();
        {
            let s = &mut app.ui_state.playlist_state;
            s.focused = PlaylistPanel::Playlists;
            s.playlists = vec![pl("Alpha", &["/a1", "/a2"]), pl("Beta", &["/b1"])];
            s.selected_playlist = 1; // Alpha name row
            s.expanded_playlist = None;
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.ui_state.playlist_state.expanded_playlist, Some(0));
        // Cursor stays on the Alpha name row.
        assert_eq!(app.ui_state.playlist_state.selected_playlist, 1);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.ui_state.playlist_state.expanded_playlist, None);
    }

    #[test]
    fn test_enter_song_removes_it() {
        let mut app = test_app();
        seed_playlists(
            &mut app,
            vec![pl("Alpha", &["/a1", "/a2"]), pl("Beta", &["/b1"])],
        );
        {
            let s = &mut app.ui_state.playlist_state;
            s.focused = PlaylistPanel::Playlists;
            s.expanded_playlist = Some(0);
            s.selected_playlist = 2; // first song of Alpha
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.ui_state.playlist_state.playlists[0].songs,
            vec![PathBuf::from("/a2")]
        );
        assert_eq!(app.ui_state.playlist_state.selected_playlist, 2);
    }

    // ── M3U export ──

    #[test]
    fn test_export_writes_m3u() {
        let mut app = test_app();
        std::fs::create_dir_all(crate::paths::data_dir()).unwrap();
        seed_playlists(&mut app, vec![pl("Alpha", &["/music/a.flac"])]);
        {
            let s = &mut app.ui_state.playlist_state;
            s.focused = PlaylistPanel::Playlists;
            s.expanded_playlist = Some(0);
        }
        press_char(&mut app, 'e');

        // The notice is the *global* one: the panel's own is keypress feedback
        // ("expand a playlist first"), and this sentence is the daemon's
        // answer — which the client may be showing in any view by the time it
        // lands.
        let notice = app.ui_state.notification.clone();
        assert!(
            notice
                .as_ref()
                .is_some_and(|(msg, _)| msg.starts_with("Exported to")),
            "expected export notification, got {notice:?}"
        );
        let export_path = crate::paths::data_dir().join("Alpha.m3u");
        let content = std::fs::read_to_string(&export_path).expect("m3u file written");
        assert!(content.contains("/music/a.flac"));
        std::fs::remove_file(&export_path).ok();
    }

    #[test]
    fn test_export_without_expanded_notifies() {
        let mut app = test_app();
        {
            let s = &mut app.ui_state.playlist_state;
            s.focused = PlaylistPanel::Playlists;
            s.playlists = vec![pl("Alpha", &["/music/a.flac"])];
            s.expanded_playlist = None;
        }
        press_char(&mut app, 'e');
        let msg = app
            .ui_state
            .playlist_state
            .notification
            .as_ref()
            .map(|(m, _)| m.clone())
            .unwrap_or_default();
        assert!(
            msg.contains("Expand a playlist first"),
            "unexpected notification: {msg}"
        );
    }

    // ── enter_playlist_view ──

    #[test]
    fn test_enter_playlist_view_syncs_library_paths() {
        let mut app = test_app();
        app.ui_state.player.tracks = vec![
            crate::ui::TrackDisplay {
                path: PathBuf::from("/m1.flac"),
                title: "M1".into(),
                artist: "A".into(),
                duration_secs: 1.0,
            },
            crate::ui::TrackDisplay {
                path: PathBuf::from("/m2.flac"),
                title: "M2".into(),
                artist: "A".into(),
                duration_secs: 1.0,
            },
        ];
        app.enter_playlist_view();
        assert_eq!(
            app.ui_state.playlist_state.library_paths,
            vec![PathBuf::from("/m1.flac"), PathBuf::from("/m2.flac")]
        );
    }

    // ── clamp_playlist_scroll ──

    #[test]
    fn test_clamp_playlist_scroll_forward() {
        let mut state = PlaylistManagerState {
            playlists: vec![pl("A", &[]), pl("B", &[])], // total 3 lines
            selected_playlist: 10,
            scroll_playlists: 0,
            ..Default::default()
        };
        App::clamp_playlist_scroll(&mut state, 2);
        assert_eq!(state.selected_playlist, 2);
        assert_eq!(state.scroll_playlists, 1); // 2 >= 0+2 → 2-2+1
    }

    #[test]
    fn test_clamp_playlist_scroll_backward() {
        let mut state = PlaylistManagerState {
            playlists: vec![pl("A", &[]), pl("B", &[])], // total 3 lines
            selected_playlist: 1,
            scroll_playlists: 5,
            ..Default::default()
        };
        App::clamp_playlist_scroll(&mut state, 20);
        assert_eq!(state.selected_playlist, 1);
        assert_eq!(state.scroll_playlists, 1); // cursor above scroll → pull down
    }

    // ── PlaylistFlatModel integration ──

    #[test]
    fn test_flat_model_resolution_matches_handler() {
        let playlists = vec![pl("Alpha", &["/a1", "/a2"]), pl("Beta", &["/b1"])];
        let model = PlaylistFlatModel::new(&playlists, Some(0));
        assert_eq!(model.total_lines(), 5); // "..." + Alpha + 2 songs + Beta (collapsed)
        assert_eq!(model.resolve(0), super::LineTarget::AddNew);
        assert_eq!(model.resolve(1), super::LineTarget::PlaylistName(0));
        assert_eq!(
            model.resolve(2),
            super::LineTarget::Song {
                playlist: 0,
                song_index: 0
            }
        );
        assert_eq!(model.line_of_playlist(1), Some(4));
    }
}
