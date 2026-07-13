use crossterm::event::{KeyCode, KeyEvent};

use crate::app::App;
use crate::ui::views::playlist_view::{InsertMode, LineTarget, PlaylistFlatModel, PlaylistPanel};

/// Number of visible playlist lines used for scroll-window calculations.
pub const VISIBLE_LINES: usize = 10;

impl App {
    /// Clamp scroll offset and cursor after a playlist structural change.
    pub(super) fn clamp_playlist_scroll(
        state: &mut crate::ui::views::playlist_view::PlaylistManagerState,
    ) {
        let total = PlaylistFlatModel::new(&state.playlists, state.expanded_playlist).total_lines();
        if state.selected_playlist >= total {
            state.selected_playlist = total.saturating_sub(1);
        }
        let vis = VISIBLE_LINES;
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
                }
                PlaylistPanel::Playlists => {
                    let model = PlaylistFlatModel::new(&state.playlists, state.expanded_playlist);
                    let max_vis = model.total_lines().saturating_sub(1);
                    if state.selected_playlist < max_vis {
                        state.selected_playlist += 1;
                    }
                    Self::clamp_playlist_scroll(state);
                }
            },
            KeyCode::Char('k') | KeyCode::Up => match state.focused {
                PlaylistPanel::Library => {
                    state.selected_library_song = state.selected_library_song.saturating_sub(1);
                    if state.selected_library_song < state.scroll_library {
                        state.scroll_library = state.selected_library_song;
                    }
                }
                PlaylistPanel::Playlists => {
                    if state.selected_playlist > 0 {
                        state.selected_playlist -= 1;
                    }
                    if state.selected_playlist < state.scroll_playlists {
                        state.scroll_playlists = state.selected_playlist;
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
                            Self::clamp_playlist_scroll(state);
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
