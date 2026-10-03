use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::metadata::reader::read_metadata;
use crate::ui::TrackDisplay;

impl App {
    pub(super) fn save_state(&self) {
        let state_path = crate::paths::state_dir().join("state.json");

        if let Some(parent) = state_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let saved = super::SavedState {
            volume: Some(self.ui_state.volume),
            repeat_mode: Some(self.ui_state.repeat_mode),
            lyrics_offset_ms: Some(self.ui_state.lyrics.lyrics_offset_ms),
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

    /// Restore persisted playback settings (volume, repeat mode, lyrics
    /// offset) from `state.json` in the state directory on startup. Silently
    /// ignores missing or corrupt state — a fresh install must not error out.
    pub(super) fn load_state(&mut self) {
        let state_path = crate::paths::state_dir().join("state.json");
        let Ok(content) = std::fs::read_to_string(&state_path) else {
            return;
        };
        let Ok(saved) = serde_json::from_str::<super::SavedState>(&content) else {
            tracing::warn!("Ignoring unreadable state.json");
            return;
        };
        // Apply each setting independently so a partial file still restores
        // what it has; anything absent keeps the config-derived value.
        if let Some(volume) = saved.volume {
            self.ui_state.volume = volume;
            self.engine.set_volume(volume);
        }
        if let Some(mode) = saved.repeat_mode {
            self.ui_state.repeat_mode = mode;
        }
        if let Some(offset) = saved.lyrics_offset_ms {
            self.ui_state.lyrics.lyrics_offset_ms = offset;
        }
        tracing::info!("Restored saved state (volume={:.2})", self.ui_state.volume);
    }

    pub(super) fn save_playlists(&self) {
        #[derive(Serialize)]
        struct SavePlaylist {
            name: String,
            songs: Vec<String>,
        }
        let path = crate::paths::state_dir().join("playlists.json");
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
        let path = crate::paths::state_dir().join("playlists.json");
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
        let path = crate::paths::state_dir().join("library.json");
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
        let path = crate::paths::state_dir().join("library.json");
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(paths) = serde_json::from_str::<Vec<String>>(&content) {
                for p_str in paths {
                    let pb = std::path::PathBuf::from(&p_str);
                    if pb.exists() && !self.ui_state.file_browser_state.library_paths.contains(&pb)
                    {
                        self.ui_state
                            .file_browser_state
                            .library_paths
                            .push(pb.clone());
                        if pb.is_dir() {
                            self.start_library_scan(pb);
                        } else {
                            self.load_and_play_collect(&pb);
                        }
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
            if !self
                .ui_state
                .player
                .tracks
                .iter()
                .any(|t| t.path == info.path)
            {
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

                if !self
                    .ui_state
                    .player
                    .tracks
                    .iter()
                    .any(|t| t.path == info.path)
                {
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
                        self.ui_state.player.year =
                            info.year.map(|y| y.to_string()).unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use crate::app::handlers::test_support::test_app;
    use crate::paths;
    use crate::ui::views::playlist_view::PlaylistData;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// The runtime directories are per-process, not per-test, so tests that
    /// write the same file must not run concurrently.
    static FILE_LOCK: Mutex<()> = Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        FILE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write_state_file(name: &str, contents: &str) {
        let dir = paths::state_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), contents).unwrap();
    }

    fn read_state_file(name: &str) -> String {
        std::fs::read_to_string(paths::state_dir().join(name)).unwrap()
    }

    /// A path that really exists, since both loaders drop entries that do not.
    fn fixture(name: &str) -> PathBuf {
        std::fs::canonicalize(format!("tests/fixtures/{name}")).expect("fixture file")
    }

    // ── state.json ──

    #[test]
    fn partial_state_file_restores_what_it_contains() {
        let _guard = lock();
        let mut app = test_app();
        // Values that must survive an absent field.
        app.ui_state.volume = 0.5;
        app.ui_state.lyrics.lyrics_offset_ms = 0;
        app.ui_state.repeat_mode = crate::ui::RepeatMode::Sequential;

        write_state_file("state.json", r#"{"volume": 0.25, "lyrics_offset_ms": 700}"#);
        app.load_state();

        assert!(
            (app.ui_state.volume - 0.25).abs() < 1e-6,
            "present field must be restored, got {}",
            app.ui_state.volume
        );
        assert_eq!(app.ui_state.lyrics.lyrics_offset_ms, 700);
        assert_eq!(
            app.ui_state.repeat_mode,
            crate::ui::RepeatMode::Sequential,
            "absent field keeps the value the app already had"
        );
    }

    #[test]
    fn state_round_trips_through_the_file() {
        let _guard = lock();
        let mut app = test_app();
        app.ui_state.volume = 0.42;
        app.ui_state.repeat_mode = crate::ui::RepeatMode::Shuffle;
        app.ui_state.lyrics.lyrics_offset_ms = -1500;
        app.save_state();

        // A fresh app starts from config defaults; loading must overwrite them.
        let mut reloaded = test_app();
        reloaded.ui_state.volume = 1.0;
        reloaded.ui_state.repeat_mode = crate::ui::RepeatMode::Sequential;
        reloaded.ui_state.lyrics.lyrics_offset_ms = 0;
        reloaded.load_state();

        assert!((reloaded.ui_state.volume - 0.42).abs() < 1e-6);
        assert_eq!(
            reloaded.ui_state.repeat_mode,
            crate::ui::RepeatMode::Shuffle
        );
        assert_eq!(reloaded.ui_state.lyrics.lyrics_offset_ms, -1500);
    }

    /// Reading a state file that is not there is the normal first-run case.
    #[test]
    fn a_missing_state_file_leaves_the_app_untouched() {
        let _guard = lock();
        let mut app = test_app();
        app.ui_state.volume = 0.33;
        let _ = std::fs::remove_file(paths::state_dir().join("state.json"));

        app.load_state();

        assert!((app.ui_state.volume - 0.33).abs() < 1e-6);
    }

    #[test]
    fn a_corrupt_state_file_is_ignored_rather_than_fatal() {
        let _guard = lock();
        let mut app = test_app();
        app.ui_state.volume = 0.33;
        write_state_file("state.json", "{ not json at all");

        app.load_state();

        assert!(
            (app.ui_state.volume - 0.33).abs() < 1e-6,
            "unreadable state must not clobber the live value"
        );
    }

    // ── playlists.json ──

    #[test]
    fn playlists_round_trip_and_drop_songs_that_no_longer_exist() {
        let _guard = lock();
        let song = fixture("test.flac");
        let mut app = test_app();
        app.ui_state.playlist_state.playlists = vec![PlaylistData {
            name: "Keep".into(),
            songs: vec![song.clone(), PathBuf::from("/definitely/missing.flac")],
        }];
        app.save_playlists();

        let mut reloaded = test_app();
        reloaded.load_playlists();

        let loaded = &reloaded.ui_state.playlist_state.playlists;
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "Keep");
        assert_eq!(
            loaded[0].songs,
            vec![song],
            "a song that no longer exists must not be restored"
        );
    }

    #[test]
    fn a_corrupt_playlists_file_is_ignored() {
        let _guard = lock();
        write_state_file("playlists.json", "[[[not json");

        let mut app = test_app();
        app.load_playlists();

        assert!(app.ui_state.playlist_state.playlists.is_empty());
    }

    #[test]
    fn a_missing_playlists_file_is_a_no_op() {
        let _guard = lock();
        let _ = std::fs::remove_file(paths::state_dir().join("playlists.json"));

        let mut app = test_app();
        app.load_playlists();

        assert!(app.ui_state.playlist_state.playlists.is_empty());
    }

    // ── library.json ──

    #[test]
    fn library_paths_round_trip_and_skip_missing_entries() {
        let _guard = lock();
        let directory = std::env::temp_dir().join(format!("tmper-persist-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let song = fixture("test.flac");

        let mut app = test_app();
        app.ui_state.file_browser_state.library_paths = vec![
            directory.clone(),
            song.clone(),
            PathBuf::from("/definitely/missing"),
        ];
        app.save_library_paths();

        let mut reloaded = test_app();
        reloaded.load_library_paths();

        let loaded = &reloaded.ui_state.file_browser_state.library_paths;
        assert!(loaded.contains(&directory), "existing directory restored");
        assert!(loaded.contains(&song), "existing file restored");
        assert!(
            !loaded.iter().any(|p| p.ends_with("missing")),
            "a path that no longer exists is not restored"
        );
        // A file entry is also collected into the player queue.
        assert!(
            reloaded
                .ui_state
                .player
                .tracks
                .iter()
                .any(|t| t.path == song),
            "a restored file becomes a playable track"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The loader appends, so an entry already present must not be added twice.
    #[test]
    fn loading_library_paths_does_not_duplicate_entries() {
        let _guard = lock();
        let song = fixture("test.flac");

        let mut app = test_app();
        app.ui_state.file_browser_state.library_paths = vec![song.clone()];
        app.save_library_paths();
        // Present before the load, exactly as a running app would have it.
        app.load_library_paths();

        let occurrences = app
            .ui_state
            .file_browser_state
            .library_paths
            .iter()
            .filter(|p| **p == song)
            .count();
        assert_eq!(
            occurrences, 1,
            "the path is already known; do not add it again"
        );
    }

    #[test]
    fn a_corrupt_library_file_is_ignored() {
        let _guard = lock();
        write_state_file("library.json", "{\"not\": \"a list\"}");

        let mut app = test_app();
        app.load_library_paths();

        assert!(app.ui_state.file_browser_state.library_paths.is_empty());
    }

    #[test]
    fn saving_library_paths_writes_the_current_list() {
        let _guard = lock();
        let song = fixture("test.flac");
        let mut app = test_app();
        app.ui_state.file_browser_state.library_paths = vec![song.clone()];
        app.save_library_paths();

        let written = read_state_file("library.json");
        assert!(written.contains(song.to_string_lossy().as_ref()));
    }
}
