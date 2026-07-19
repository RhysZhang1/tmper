use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::metadata::reader::read_metadata;
use crate::ui::TrackDisplay;

impl App {
    pub(super) fn save_state(&self) {
        let state_path = crate::paths::data_dir().join("state.json");

        if let Some(parent) = state_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let saved = super::SavedState {
            volume: self.ui_state.volume,
            repeat_mode: self.ui_state.repeat_mode,

            lyrics_offset_ms: self.ui_state.lyrics.lyrics_offset_ms,
            last_track_path: self
                .ui_state
                .player
                .playing_index
                .and_then(|i| self.ui_state.player.tracks.get(i))
                .map(|t| t.path.to_string_lossy().to_string()),
        };

        if let Ok(json) = serde_json::to_string_pretty(&saved) {
            if let Err(e) = std::fs::write(&state_path, json) {
                tracing::warn!("Failed to save state: {e}");
            }
        }
    }

    pub(super) fn save_playlists(&self) {
        #[derive(Serialize)]
        struct SavePlaylist {
            name: String,
            songs: Vec<String>,
        }
        let path = crate::paths::data_dir().join("playlists.json");
        let save: Vec<SavePlaylist> = self
            .ui_state
            .playlist_state
            .playlists
            .iter()
            .map(|pl| SavePlaylist {
                name: pl.name.clone(),
                songs: pl
                    .songs
                    .iter()
                    .map(|s| s.to_string_lossy().to_string())
                    .collect(),
            })
            .collect();
        if let Ok(json) = serde_json::to_string_pretty(&save) {
            let _ = std::fs::write(&path, json);
        }
    }

    pub(super) fn load_playlists(&mut self) {
        #[derive(Deserialize)]
        struct SavePlaylist {
            name: String,
            songs: Vec<String>,
        }
        let path = crate::paths::data_dir().join("playlists.json");
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(save) = serde_json::from_str::<Vec<SavePlaylist>>(&content) {
                for sp in save {
                    let songs: Vec<std::path::PathBuf> = sp
                        .songs
                        .iter()
                        .filter(|s| std::path::Path::new(s).exists())
                        .map(std::path::PathBuf::from)
                        .collect();
                    self.ui_state.playlist_state.playlists.push(
                        crate::ui::views::playlist_view::PlaylistData {
                            name: sp.name,
                            songs,
                        },
                    );
                }
            }
        }
    }

    pub(super) fn save_library_paths(&self) {
        let path = crate::paths::data_dir().join("library.json");
        let paths: Vec<String> = self
            .ui_state
            .file_browser_state
            .library_paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        if let Ok(json) = serde_json::to_string_pretty(&paths) {
            let _ = std::fs::write(&path, json);
        }
    }

    pub(super) fn load_library_paths(&mut self) {
        let path = crate::paths::data_dir().join("library.json");
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(paths) = serde_json::from_str::<Vec<String>>(&content) {
                for p_str in paths {
                    let pb = std::path::PathBuf::from(&p_str);
                    if pb.exists() && !self.ui_state.file_browser_state.library_paths.contains(&pb)
                    {
                        self.load_and_play_collect(&pb);
                    }
                }
            }
        }
    }

    pub(super) fn load_and_play_collect(&mut self, path: &std::path::Path) {
        if let Ok(info) = read_metadata(path) {
            let title = info.title.clone();
            let artist = info
                .artist
                .clone()
                .unwrap_or_else(|| "Unknown Artist".into());
            let duration = info.duration.as_secs_f64();
            if !self.ui_state.player.tracks.iter().any(|t| t.path == info.path) {
                self.ui_state.player.tracks.push(TrackDisplay {
                    path: info.path.clone(),
                    title,
                    artist,
                    duration_secs: duration,
                });
            }
        }
    }

    pub(super) fn load_and_play(&mut self, path: &PathBuf) {
        match read_metadata(path) {
            Ok(info) => {
                let title = info.title.clone();
                let artist = info
                    .artist
                    .clone()
                    .unwrap_or_else(|| "Unknown Artist".into());
                let duration = info.duration.as_secs_f64();

                if !self.ui_state.player.tracks.iter().any(|t| t.path == info.path) {
                    self.ui_state.player.tracks.push(TrackDisplay {
                        path: info.path.clone(),
                        title: title.clone(),
                        artist: artist.clone(),
                        duration_secs: duration,
                    });
                }

                if let Some(idx) = self
                    .ui_state
                    .player
                    .tracks
                    .iter()
                    .position(|t| t.path == info.path)
                {
                    self.ui_state.player.playing_index = Some(idx);
                    self.ui_state.player.selected_index = idx;
                }

                let result = self.engine.play_file_async(path);
                match result {
                    Ok(()) => {
                        self.ui_state.player.title = title;
                        self.ui_state.player.artist = artist;
                        self.ui_state.player.album = info.album.unwrap_or_default();
                        self.ui_state.player.genre = info.genre.unwrap_or_default();
                        self.ui_state.player.year = info.year.map(|y| y.to_string()).unwrap_or_default();
                        self.ui_state.player.codec = info.codec.clone();
                        self.ui_state.player.position = 0.0;
                        self.ui_state.player.duration = duration;
                        self.ui_state.player.is_playing = true;
                        self.ui_state.player.cover_art = info.cover_art.map(Arc::new);
                        self.ui_state
                            .player
                            .cover_gen
                            .set(self.ui_state.player.cover_gen.get() + 1);
                        self.engine.set_volume(self.ui_state.volume);
                        self.load_lyrics_for_current();
                        self.start_fft();
                        // Reset the cover-escape guard so the 800ms window
                        // starts NOW (right before the event loop resumes),
                        // not when on_track_ended() first set it (potentially
                        // seconds ago if the sync decode path was used).
                        self.cover_guard_until = Some(
                            std::time::Instant::now() + std::time::Duration::from_millis(800),
                        );
                        tracing::info!("Now playing: {:?}", path);
                    }
                    Err(e) => {
                        tracing::error!("Failed to play file: {e}");
                        self.ui_state.player.title = format!("Error: {e}");
                        self.ui_state.player.is_playing = false;
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to read metadata: {e}");
            }
        }
    }
}
