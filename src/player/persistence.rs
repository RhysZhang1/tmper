//! `state.json`: what survives a restart.
//!
//! The daemon is the only writer. Volume and repeat mode are its truth to
//! begin with; the lyric offset is the client's to *choose* and the daemon's to
//! *write*, which is why it arrives as [`crate::ipc::proto::Request::SetLyricsOffset`]
//! instead of in a second file. One file, one writer.

use serde::{Deserialize, Serialize};

use super::Player;
use crate::ipc::proto::RepeatMode;
use crate::paths;

/// Persisted on exit, restored at startup.
///
/// Every setting is optional: a `state.json` written by an older build (or
/// truncated mid-write) used to fail to deserialize as a whole, and the app
/// silently dropped *all* of it — volume, repeat mode and lyric offset
/// together. Missing fields now fall back to whatever the config already
/// provided, and a present field is still restored.
#[derive(Serialize, Deserialize)]
struct SavedState {
    volume: Option<f32>,
    repeat_mode: Option<RepeatMode>,
    lyrics_offset_ms: Option<i64>,
    /// Remembered so `Resume` has somewhere to fall back to. Startup itself
    /// deliberately does not resume playback (see [`Player::load_state`]).
    last_track_path: Option<String>,
}

fn state_path() -> std::path::PathBuf {
    paths::state_dir().join("state.json")
}

impl Player {
    pub fn save_state(&self) {
        let path = state_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let saved = SavedState {
            volume: Some(self.volume),
            repeat_mode: Some(self.repeat),
            lyrics_offset_ms: Some(self.lyrics_offset_ms),
            last_track_path: self
                .now_playing
                .as_ref()
                .map(|n| n.path.to_string_lossy().to_string()),
        };

        if let Ok(json) = serde_json::to_string_pretty(&saved) {
            if let Err(e) = std::fs::write(&path, json) {
                tracing::warn!("Failed to save state: {e}");
            }
        }
    }

    /// Restore persisted playback settings (volume, repeat mode, lyrics
    /// offset) from `state.json` on startup. Silently ignores missing or
    /// corrupt state — a fresh install must not error out.
    ///
    /// Called once, by whoever is starting the player: the daemon at its own
    /// startup, or [`crate::app::handle::LocalHandle`] when the player and the
    /// TUI are the same process.
    pub fn load_state(&mut self) {
        let Ok(content) = std::fs::read_to_string(state_path()) else {
            return;
        };
        let Ok(saved) = serde_json::from_str::<SavedState>(&content) else {
            tracing::warn!("Ignoring unreadable state.json");
            return;
        };
        // Apply each setting independently so a partial file still restores
        // what it has; anything absent keeps the config-derived value.
        if let Some(volume) = saved.volume {
            self.set_volume(volume);
        }
        if let Some(mode) = saved.repeat_mode {
            self.repeat = mode;
        }
        if let Some(offset) = saved.lyrics_offset_ms {
            self.lyrics_offset_ms = offset;
        }
        // Remembered, not played: the daemon stays silent until something asks
        // for sound, and this is what `Resume` asks with.
        self.last_track = saved.last_track_path.map(std::path::PathBuf::from);
        tracing::info!("Restored saved state (volume={:.2})", self.volume);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::sync::Mutex;

    /// The runtime directories are per-process, not per-test, so tests that
    /// write the same file must not run concurrently.
    static FILE_LOCK: Mutex<()> = Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        FILE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn new_player() -> Player {
        Player::new_headless(&Config::default())
    }

    fn write_state_file(name: &str, contents: &str) {
        let dir = paths::state_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), contents).unwrap();
    }

    #[test]
    fn partial_state_file_restores_what_it_contains() {
        let _guard = lock();
        let mut player = new_player();
        // Values that must survive an absent field.
        player.volume = 0.5;
        player.lyrics_offset_ms = 0;
        player.repeat = RepeatMode::Sequential;

        write_state_file("state.json", r#"{"volume": 0.25, "lyrics_offset_ms": 700}"#);
        player.load_state();

        assert!(
            (player.volume - 0.25).abs() < 1e-6,
            "present field must be restored, got {}",
            player.volume
        );
        assert_eq!(player.lyrics_offset_ms, 700);
        assert_eq!(
            player.repeat,
            RepeatMode::Sequential,
            "absent field keeps the value the app already had"
        );
    }

    #[test]
    fn state_round_trips_through_the_file() {
        let _guard = lock();
        let mut player = new_player();
        player.volume = 0.42;
        player.repeat = RepeatMode::Shuffle;
        player.lyrics_offset_ms = -1500;
        player.save_state();

        // A fresh player starts from config defaults; loading must overwrite them.
        let mut reloaded = new_player();
        reloaded.volume = 1.0;
        reloaded.repeat = RepeatMode::Sequential;
        reloaded.lyrics_offset_ms = 0;
        reloaded.load_state();

        assert!((reloaded.volume - 0.42).abs() < 1e-6);
        assert_eq!(reloaded.repeat, RepeatMode::Shuffle);
        assert_eq!(reloaded.lyrics_offset_ms, -1500);
    }

    /// Reading a state file that is not there is the normal first-run case.
    #[test]
    fn a_missing_state_file_leaves_the_app_untouched() {
        let _guard = lock();
        let mut player = new_player();
        player.volume = 0.33;
        let _ = std::fs::remove_file(paths::state_dir().join("state.json"));

        player.load_state();

        assert!((player.volume - 0.33).abs() < 1e-6);
    }

    #[test]
    fn a_corrupt_state_file_is_ignored_rather_than_fatal() {
        let _guard = lock();
        let mut player = new_player();
        player.volume = 0.33;
        write_state_file("state.json", "{ not json at all");

        player.load_state();

        assert!(
            (player.volume - 0.33).abs() < 1e-6,
            "unreadable state must not clobber the live value"
        );
    }

    /// Restoring a track is not starting it — but it is what makes `play` mean
    /// something on a daemon that has just come up with an empty deck.
    #[tokio::test]
    async fn a_restored_track_is_what_resume_falls_back_to() {
        let _guard = lock();
        let track = std::fs::canonicalize("tests/fixtures/test.wav").expect("fixture file");
        write_state_file(
            "state.json",
            &format!(
                r#"{{"last_track_path": {:?}}}"#,
                track.to_string_lossy().as_ref()
            ),
        );

        let mut player = new_player();
        player.load_state();
        assert_eq!(
            player.state().path,
            None,
            "loading state must not start playback"
        );

        player.execute(crate::ipc::proto::Request::Resume);
        assert!(
            player.state().status.is_active(),
            "play must fall back to the track the last run ended on"
        );
        assert_eq!(player.state().path.as_deref(), Some(track.as_path()));
    }
}
