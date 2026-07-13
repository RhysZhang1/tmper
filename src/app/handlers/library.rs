use crossterm::event::{KeyCode, KeyEvent};

use crate::app::App;
use crate::ui::views::library_view::LibraryPanel;

impl App {
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
        for t in &self.ui_state.tracks {
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
