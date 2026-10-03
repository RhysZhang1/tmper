//! `state.json`: what survives a restart.
//!
//! The daemon is the only writer. Volume and repeat mode are its truth to
//! begin with; the lyric offset is the client's to *choose* and the daemon's to
//! *write*, which is why it arrives as [`crate::ipc::proto::Request::SetLyricsOffset`]
//! instead of in a second file. One file, one writer.

use serde::{Deserialize, Serialize};

use super::{NowPlaying, Player};
use crate::ipc::proto::{QueueTrack, RepeatMode};
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
    /// The queue as it stood, so reopening tmper finds the songs still lined
    /// up rather than an empty panel.
    queue: Option<Vec<QueueTrack>>,
    /// Where in that queue the needle was.
    queue_index: Option<usize>,
    /// How far into `last_track_path` the needle was, in seconds.
    ///
    /// Restored as a *parked* position — the track comes back loaded and the
    /// progress bar shows where it stopped, but nothing sounds until something
    /// asks for it. Rounding is deliberate: the value is a JSON float, and the
    /// engine's clock is second-granular to the ear anyway.
    position_secs: Option<f64>,
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
            queue: Some(self.queue.clone()),
            queue_index: self.playing_index,
            position_secs: Some(self.position_secs()),
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

        // The queue comes back whole — it is the part a client cannot
        // reconstruct, and the daemon is the only thing that has it.
        if let Some(queue) = saved.queue {
            self.queue = queue;
        }
        // Guarded rather than trusted: a cursor is only meaningful against the
        // queue it was written with, and an out-of-range index would reach the
        // client through the snapshot.
        self.playing_index = saved.queue_index.filter(|index| *index < self.queue.len());

        // The deck, in the shape the player expects: the queue row already
        // carries the three fields the strip shows, and the rest (album,
        // genre, year, codec) comes back the first time the track is started,
        // when the tag reader runs anyway.
        if let Some(track) = self
            .last_track
            .as_ref()
            .and_then(|path| self.queue.iter().find(|track| &track.path == path))
        {
            self.now_playing = Some(NowPlaying::from_queue_track(track));
        }
        // A NaN or an infinity in a hand-edited file would reach the decoder's
        // seek and turn every start into a failure; a position is a finite
        // number of seconds or it is nothing.
        self.resume_at = saved.position_secs.filter(|secs| secs.is_finite());

        tracing::info!(
            "Restored saved state (volume={:.2}, {} queued)",
            self.volume,
            self.queue.len()
        );
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

    /// A session that was saved comes back *parked*: the track is on the deck
    /// and the progress bar shows where it stopped, but nothing is sounding.
    /// Making noise the moment a daemon starts is a thing users learn to fear.
    #[tokio::test]
    async fn a_saved_session_comes_back_parked_on_the_same_track() {
        let _guard = lock();
        let track = std::fs::canonicalize("tests/fixtures/test.wav").expect("fixture file");
        let other = std::fs::canonicalize("tests/fixtures/test.flac").expect("fixture file");
        write_state_file(
            "state.json",
            &format!(
                r#"{{
                    "last_track_path": {:?},
                    "position_secs": 1.2,
                    "queue_index": 1,
                    "queue": [
                        {{"path": {:?}, "title": "First", "artist": "A", "duration_secs": 2.0}},
                        {{"path": {:?}, "title": "Second", "artist": "B", "duration_secs": 2.0}},
                        {{"path": {:?}, "title": "Third", "artist": "C", "duration_secs": 2.0}}
                    ]
                }}"#,
                track.to_string_lossy().as_ref(),
                other.to_string_lossy().as_ref(),
                track.to_string_lossy().as_ref(),
                other.to_string_lossy().as_ref(),
            ),
        );

        let mut player = new_player();
        player.load_state();
        let state = player.state();

        assert_eq!(state.status, crate::audio::engine::PlaybackState::Stopped);
        assert_eq!(
            state.path.as_deref(),
            Some(track.as_path()),
            "the deck has the track the last run ended on"
        );
        assert_eq!(state.title, "Second", "and shows it, from the queue row");
        assert!(
            state.position_secs > 1.0 && state.position_secs < 1.5,
            "the needle is parked where it stopped, got {}",
            state.position_secs
        );
        assert_eq!(state.playing_index, Some(1));
        assert_eq!(player.queue().len(), 3, "the queue came back whole");
    }

    /// A parked needle is not a rewind: the next `play` carries on from it,
    /// which is the difference between a restored session and a stop.
    #[tokio::test]
    async fn resuming_a_restored_session_continues_where_it_left_off() {
        let _guard = lock();
        let track = std::fs::canonicalize("tests/fixtures/test.wav").expect("fixture file");
        write_state_file(
            "state.json",
            &format!(
                r#"{{
                    "last_track_path": {:?},
                    "position_secs": 1.2,
                    "queue_index": 0,
                    "queue": [{{"path": {:?}, "title": "T", "artist": "A", "duration_secs": 2.0}}]
                }}"#,
                track.to_string_lossy().as_ref(),
                track.to_string_lossy().as_ref(),
            ),
        );

        let mut player = new_player();
        player.load_state();
        player.execute(crate::ipc::proto::Request::Resume);

        let position = player.state().position_secs;
        assert!(player.state().status.is_active(), "resume makes sound");
        assert!(
            position > 1.0 && position < 1.5,
            "it starts a fifth of a second in, not at the top: {position}"
        );
    }

    /// A saved position is spent by the start it paid for: stopping afterwards
    /// rewinds, exactly as it does for a session that was never restored.
    #[tokio::test]
    async fn a_restored_position_does_not_survive_a_stop() {
        let _guard = lock();
        let track = std::fs::canonicalize("tests/fixtures/test.wav").expect("fixture file");
        write_state_file(
            "state.json",
            &format!(
                r#"{{
                    "last_track_path": {:?},
                    "position_secs": 1.2,
                    "queue_index": 0,
                    "queue": [{{"path": {:?}, "title": "T", "artist": "A", "duration_secs": 2.0}}]
                }}"#,
                track.to_string_lossy().as_ref(),
                track.to_string_lossy().as_ref(),
            ),
        );

        let mut player = new_player();
        player.load_state();
        player.execute(crate::ipc::proto::Request::Stop);
        player.execute(crate::ipc::proto::Request::Resume);

        assert!(
            player.state().position_secs < 0.05,
            "a stop is still a rewind, even on a restored session"
        );
    }

    /// What `save_state` writes is the live position — and it is still live at
    /// the moment it is written. The daemon silences the device *after*
    /// persisting for exactly this reason: a stop first would rewind the clock
    /// out from under the save.
    #[tokio::test]
    async fn saving_a_session_records_the_queue_and_the_live_position() {
        let _guard = lock();
        let track = std::fs::canonicalize("tests/fixtures/test.wav").expect("fixture file");
        let other = std::fs::canonicalize("tests/fixtures/test.flac").expect("fixture file");

        let mut player = new_player();
        player.execute(crate::ipc::proto::Request::Play {
            path: track.clone(),
        });
        player.execute(crate::ipc::proto::Request::QueuePush { path: other });
        std::thread::sleep(std::time::Duration::from_millis(80));
        player.save_state();

        // Read back through a fresh player, which is the path that matters.
        let mut reloaded = new_player();
        reloaded.load_state();
        assert_eq!(reloaded.queue().len(), 2);
        assert_eq!(reloaded.state().playing_index, Some(0));
        assert!(
            reloaded.state().position_secs > 0.05,
            "the position was written while the clock was running, got {}",
            reloaded.state().position_secs
        );
        assert_eq!(reloaded.state().path.as_deref(), Some(track.as_path()));
    }

    /// Every clean exit — `tmper quit`, and the idle timeout — runs
    /// `save_state` and *then* silences the device. The order is the whole
    /// test: silencing is a `stop`, a stop rewinds, and a save after it would
    /// faithfully record 0:00 on every exit and lose the user's place in the
    /// one case the state file exists for.
    #[tokio::test]
    async fn a_shutdown_saves_the_position_before_it_silences_the_player() {
        let _guard = lock();
        let track = std::fs::canonicalize("tests/fixtures/test.wav").expect("fixture file");
        let mut player = new_player();
        player.execute(crate::ipc::proto::Request::Play {
            path: track.clone(),
        });
        std::thread::sleep(std::time::Duration::from_millis(80));

        // The daemon's exit sequence, in its order.
        player.execute(crate::ipc::proto::Request::Shutdown);
        player.save_state();
        player.shutdown();

        let mut reloaded = new_player();
        reloaded.load_state();
        assert_eq!(
            reloaded.state().path.as_deref(),
            Some(track.as_path()),
            "the deck survives the exit"
        );
        assert!(
            reloaded.state().position_secs > 0.05,
            "and so does the place in the track, got {}",
            reloaded.state().position_secs
        );
    }

    /// A cursor is only meaningful against the queue it was written with; a
    /// hand-edited file must not push an index past the end of it.
    #[tokio::test]
    async fn an_out_of_range_queue_index_is_dropped() {
        let _guard = lock();
        write_state_file(
            "state.json",
            r#"{"queue_index": 7, "queue": [{"path": "/nowhere/a.flac", "title": "A", "artist": "B", "duration_secs": 1.0}]}"#,
        );

        let mut player = new_player();
        player.load_state();

        assert_eq!(player.state().playing_index, None);
    }

    /// Restoring a track is not starting it — but it is what makes `play` mean
    /// something on a daemon that has just come up with an empty deck. A state
    /// file from a build that saved only the path (no queue) still works.
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
