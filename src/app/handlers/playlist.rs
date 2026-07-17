use crossterm::event::{KeyCode, KeyEvent};

use crate::app::App;
use crate::ui::views::playlist_view::{InsertMode, LineTarget, PlaylistFlatModel, PlaylistPanel};

impl App {
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
                    if !name.is_empty() {
                        let new_idx = state.playlists.len();
                        state
                            .playlists
                            .push(crate::ui::views::playlist_view::PlaylistData {
                                name,
                                songs: Vec::new(),
                            });
                        state.expanded_playlist = Some(new_idx);
                        state.selected_playlist = 0;
                        state.scroll_playlists = 0;
                    }
                    state.insert_mode = InsertMode::Off;
                    self.save_playlists();
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

        // Export expanded playlist to M3U
        if key.code == KeyCode::Char('e') && key.modifiers.is_empty() {
            if let Some(ep) = state.expanded_playlist {
                if ep < state.playlists.len() {
                    let pl_data = &state.playlists[ep];
                    let mut playlist = crate::playlist::Playlist::new(&pl_data.name);
                    for song in &pl_data.songs {
                        let title = song
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Unknown")
                            .to_string();
                        playlist.push(crate::playlist::TrackEntry::new(
                            song.clone(),
                            title,
                            String::new(),
                            0.0,
                        ));
                    }
                    let export_path =
                        crate::paths::data_dir().join(format!("{}.m3u", pl_data.name));
                    match crate::library::playlist_manager::export_m3u(&playlist, &export_path) {
                        Ok(()) => {
                            state.notification = Some((
                                format!("Exported to {}", export_path.display()),
                                std::time::Instant::now(),
                            ));
                        }
                        Err(e) => {
                            state.notification =
                                Some((format!("Export failed: {e}"), std::time::Instant::now()));
                        }
                    }
                }
            } else {
                state.notification = Some((
                    "Expand a playlist first, then press 'e' to export".to_string(),
                    std::time::Instant::now(),
                ));
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
                    if state.selected_playlist < state.scroll_playlists {
                        state.scroll_playlists = state.selected_playlist;
                    }
                    if state.selected_playlist
                        >= state.scroll_playlists + self.ui_state.visible_rows.get()
                    {
                        state.scroll_playlists = state
                            .selected_playlist
                            .saturating_sub(self.ui_state.visible_rows.get())
                            + 1;
                    }
                }
            },
            KeyCode::Enter => match state.focused {
                PlaylistPanel::Library => {
                    if let Some(ep) = state.expanded_playlist {
                        let lib_idx = state.selected_library_song;
                        if lib_idx < state.library_paths.len() {
                            let path = state.library_paths[lib_idx].clone();
                            if !state.playlists[ep].songs.contains(&path) {
                                state.playlists[ep].songs.push(path);
                                self.save_playlists();
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
                                state.playlists[playlist].songs.remove(song_index);
                                let new_model = PlaylistFlatModel::new(
                                    &state.playlists,
                                    state.expanded_playlist,
                                );
                                let total = new_model.total_lines();
                                state.selected_playlist =
                                    state.selected_playlist.min(total.saturating_sub(1));
                            }
                        }
                        LineTarget::Empty(_) => { /* "(empty)" — no-op */ }
                    }
                }
            },
            _ => {}
        }

        self.save_playlists();
    }

    pub(super) fn enter_playlist_view(&mut self) {
        self.ui_state.playlist_state.library_paths = self
            .ui_state
            .tracks
            .iter()
            .map(|t| t.path.clone())
            .collect();
    }
}
