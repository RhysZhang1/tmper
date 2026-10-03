//! The playback core: one audio engine, one queue, and the policy that
//! decides what plays next.
//!
//! This is the half of the old `app` module that survives the TUI. Everything
//! here is **synchronous and testable** — [`Player::execute`] takes a
//! [`Request`] and hands back the [`Event`]s it produced. The socket daemon
//! and the client's in-process handle both drive this same function, so the
//! tested path and the shipped path are the same path.

pub mod fft;
pub mod persistence;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::audio::engine::{AudioEngine, PlaybackEvent};
use crate::config::Config;
use crate::error::AppResult;
use crate::ipc::proto::{Event, NoticeLevel, QueueTrack, RepeatMode, Request, StateSnapshot};
use crate::metadata::reader::read_metadata;

/// Display metadata of the track that is loaded, whether or not it is still
/// sounding: stopping does not clear the title off the screen.
#[derive(Debug, Clone)]
struct NowPlaying {
    path: PathBuf,
    title: String,
    artist: String,
    album: String,
    genre: String,
    year: String,
    codec: String,
    duration_secs: f64,
}

pub struct Player {
    engine: AudioEngine,
    /// The global queue. Insertion-ordered, unique by path.
    queue: Vec<QueueTrack>,
    playing_index: Option<usize>,
    /// The songs of the playlist the user last opened, if any. `Next`/`Prev`
    /// and auto-advance prefer this over the queue, exactly as the UI did
    /// when it owned the policy.
    active_list: Vec<PathBuf>,
    now_playing: Option<NowPlaying>,
    repeat: RepeatMode,
    volume: f32,
    lyrics_offset_ms: i64,
    num_bars: usize,
    smoothing: f32,
    /// Spectrum bars, written by the FFT thread and read when a snapshot is
    /// built. Only produced while a client is subscribed.
    bars: Arc<Mutex<Vec<f32>>>,
    fft_cancel: Option<tokio::sync::watch::Sender<()>>,
    fft_subscribed: bool,
    /// Bumped on every queue change so a client can tell whether the copy it
    /// holds is the current one.
    queue_rev: u64,
    shutdown: bool,
}

impl Player {
    pub fn new(config: &Config) -> AppResult<Self> {
        Ok(Self::with_engine(config, AudioEngine::new()?))
    }

    /// Device-free player for tests: the sink's queue receiver is dropped, so
    /// nothing opens ALSA/PulseAudio and nothing actually sounds.
    #[cfg(test)]
    pub fn new_headless(config: &Config) -> Self {
        Self::with_engine(config, AudioEngine::new_headless())
    }

    fn with_engine(config: &Config, engine: AudioEngine) -> Self {
        Self {
            engine,
            queue: Vec::new(),
            playing_index: None,
            active_list: Vec::new(),
            now_playing: None,
            repeat: RepeatMode::Sequential,
            volume: config.playback.default_volume,
            lyrics_offset_ms: 0,
            num_bars: config.visualizer.num_bars as usize,
            smoothing: config.visualizer.smoothing,
            bars: Arc::new(Mutex::new(Vec::new())),
            fft_cancel: None,
            fft_subscribed: false,
            queue_rev: 0,
            shutdown: false,
        }
    }

    // ── Driving ──

    /// Apply one command, returning the events it produced. `execute` appends
    /// a snapshot to whatever the command produced, so a caller that applies
    /// the result in order always ends up current.
    pub fn execute(&mut self, request: Request) -> Vec<Event> {
        let mut events = self.apply(request);
        events.push(self.snapshot_event());
        events
    }

    /// Advance the clock: drain what the decoder reported, advance the queue
    /// if a track ended, and report the state as it now stands.
    pub fn tick(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        for event in self.engine.drain_events() {
            match event {
                // The duration it carries is already in the snapshot, read
                // from the engine directly.
                PlaybackEvent::Ready { .. } => {}
                PlaybackEvent::Finished => events.extend(self.after_track_ended()),
                PlaybackEvent::Failed(message) => {
                    tracing::error!("Playback failed: {message}");
                    events.push(Event::Notice {
                        level: NoticeLevel::Error,
                        message: format!("Playback failed: {message}"),
                    });
                }
            }
        }
        // The spectrum only exists for a client that asked for it: no
        // subscription, no FFT thread, no per-tick event.
        if self.fft_subscribed {
            events.push(Event::Visualizer { bars: self.bars() });
        }
        events.push(self.snapshot_event());
        events
    }

    fn apply(&mut self, request: Request) -> Vec<Event> {
        match request {
            Request::Play { path } => self.play(&path),
            Request::Toggle => {
                if self.engine.is_playing() {
                    self.engine.pause();
                } else {
                    self.engine.resume();
                }
                Vec::new()
            }
            Request::Pause => {
                self.engine.pause();
                Vec::new()
            }
            Request::Resume => {
                self.engine.resume();
                Vec::new()
            }
            Request::Stop => {
                self.stop_playback();
                Vec::new()
            }
            Request::Next => self.next(),
            Request::Prev => self.prev(),
            Request::SeekRelative { secs } => {
                if let Err(e) = self.engine.seek_relative(secs) {
                    tracing::error!("Seek error: {e}");
                }
                Vec::new()
            }
            Request::SetVolume { volume } => {
                self.set_volume(volume);
                Vec::new()
            }
            Request::VolumeStep { delta } => {
                self.set_volume(self.volume + delta);
                Vec::new()
            }
            Request::SetRepeat { mode } => {
                self.repeat = mode;
                Vec::new()
            }
            Request::QueuePush { path } => self.push_to_queue(&path),
            Request::QueueRemove { path } => self.remove_from_queue(&path),
            Request::SetActiveList { songs } => {
                self.active_list = songs;
                Vec::new()
            }
            Request::SubscribeVisualizer { on } => {
                self.fft_subscribed = on;
                if on {
                    self.start_fft();
                } else {
                    // Dropping the sender is what stops the thread.
                    self.fft_cancel = None;
                }
                Vec::new()
            }
            Request::SetFftParams {
                num_bars,
                smoothing,
            } => {
                self.num_bars = num_bars.max(1) as usize;
                self.smoothing = smoothing;
                Vec::new()
            }
            Request::SetLyricsOffset { ms } => {
                self.lyrics_offset_ms = ms;
                Vec::new()
            }
            Request::GetState => Vec::new(),
            Request::Shutdown => {
                self.shutdown = true;
                self.stop_playback();
                Vec::new()
            }
            Request::Hello { .. } => Vec::new(),
        }
    }

    /// Whether a [`Request::Shutdown`] has arrived. Read by the daemon's
    /// accept loop, which is the next commit; nothing else can act on it.
    #[allow(dead_code)]
    pub fn should_shutdown(&self) -> bool {
        self.shutdown
    }

    /// Tear down before the process exits: silence the device and let the FFT
    /// thread stop. Persisting is the caller's job, and it happens first.
    pub fn shutdown(&mut self) {
        self.fft_cancel = None;
        self.stop_playback();
    }

    // ── State ──

    pub fn state(&self) -> StateSnapshot {
        let now = self.now_playing.as_ref();
        StateSnapshot {
            status: self.engine.state().clone(),
            path: now.map(|n| n.path.clone()),
            title: now
                .map(|n| n.title.clone())
                .unwrap_or_else(|| "No track".into()),
            artist: now.map(|n| n.artist.clone()).unwrap_or_else(|| "—".into()),
            album: now.map(|n| n.album.clone()).unwrap_or_default(),
            genre: now.map(|n| n.genre.clone()).unwrap_or_default(),
            year: now.map(|n| n.year.clone()).unwrap_or_default(),
            codec: now.map(|n| n.codec.clone()).unwrap_or_default(),
            duration_secs: self
                .engine
                .duration_secs()
                .or_else(|| now.map(|n| n.duration_secs))
                .unwrap_or(0.0),
            position_secs: self.engine.position_secs(),
            volume: self.volume,
            repeat: self.repeat,
            playing_index: self.playing_index,
            queue_rev: self.queue_rev,
            lyrics_offset_ms: self.lyrics_offset_ms,
        }
    }

    /// The queue as the client mirrors it.
    pub fn queue(&self) -> Vec<QueueTrack> {
        self.queue.clone()
    }

    /// The most recent spectrum frame.
    pub fn bars(&self) -> Vec<f32> {
        crate::audio::engine::lock(&self.bars).clone()
    }

    /// The songs `Next`/`Prev` walk when a list is open, else empty.
    #[cfg(test)]
    pub fn active_list(&self) -> &[PathBuf] {
        &self.active_list
    }

    /// Install a queue without reading a single tag.
    ///
    /// Client tests need rows to render and remove, and the fixture set is
    /// three files; a queue has to be bigger than that to exercise scrolling.
    /// This is the one way a test can put a track in the queue that does not
    /// exist on disk.
    #[cfg(test)]
    pub fn set_queue(&mut self, tracks: Vec<QueueTrack>) {
        self.queue = tracks;
        self.queue_rev += 1;
    }

    /// The queue, as an event — sent when (and only when) it changes.
    fn queue_event(&self) -> Event {
        Event::Queue {
            rev: self.queue_rev,
            tracks: self.queue(),
        }
    }

    fn snapshot_event(&self) -> Event {
        Event::Snapshot(Box::new(self.state()))
    }

    // ── Playback ──

    /// Load a track, queueing it first if it is new, and start it.
    ///
    /// This is the one way a track starts, wherever the request came from: a
    /// keypress, the browser, the CLI, and (later) MPRIS.
    fn play(&mut self, path: &Path) -> Vec<Event> {
        let mut events = Vec::new();
        let info = match read_metadata(path) {
            Ok(info) => info,
            Err(e) => {
                tracing::error!("Failed to read metadata for {path:?}: {e}");
                events.push(Event::Notice {
                    level: NoticeLevel::Error,
                    message: format!("Cannot play {}: {e}", path.display()),
                });
                return events;
            }
        };
        let title = info.title.clone();
        let artist = info
            .artist
            .clone()
            .unwrap_or_else(|| "Unknown Artist".into());
        let duration_secs = info.duration.as_secs_f64();

        let before = self.queue.len();
        if !self.queue.iter().any(|t| t.path == info.path) {
            self.queue.push(QueueTrack {
                path: info.path.clone(),
                title: title.clone(),
                artist: artist.clone(),
                duration_secs,
            });
        }
        if self.queue.len() != before {
            self.queue_rev += 1;
            events.push(self.queue_event());
        }
        self.playing_index = self.queue.iter().position(|t| t.path == info.path);

        match self.engine.play_file_async(path) {
            Ok(()) => {
                self.now_playing = Some(NowPlaying {
                    path: info.path,
                    title,
                    artist,
                    album: info.album.unwrap_or_default(),
                    genre: info.genre.unwrap_or_default(),
                    year: info.year.map(|y| y.to_string()).unwrap_or_default(),
                    codec: info.codec,
                    duration_secs,
                });
                self.engine.set_volume(self.volume);
                tracing::info!("Now playing: {path:?}");
            }
            Err(e) => {
                tracing::error!("Failed to play file: {e}");
                events.push(Event::Notice {
                    level: NoticeLevel::Error,
                    message: format!("Playback failed: {e}"),
                });
            }
        }
        events
    }

    /// Add a file to the queue without playing it.
    ///
    /// Answers with the new queue when it changed. A file the tag reader
    /// cannot open is not a track, and saying nothing about it is the honest
    /// response — there is nothing the user could do with a queue row that
    /// will never start.
    fn push_to_queue(&mut self, path: &Path) -> Vec<Event> {
        let Ok(info) = read_metadata(path) else {
            return Vec::new();
        };
        if self.queue.iter().any(|t| t.path == info.path) {
            return Vec::new();
        }
        self.queue.push(QueueTrack {
            path: info.path.clone(),
            title: info.title.clone(),
            artist: info
                .artist
                .clone()
                .unwrap_or_else(|| "Unknown Artist".into()),
            duration_secs: info.duration.as_secs_f64(),
        });
        self.queue_rev += 1;
        vec![self.queue_event()]
    }

    fn remove_from_queue(&mut self, path: &Path) -> Vec<Event> {
        let Some(index) = self.queue.iter().position(|t| t.path == path) else {
            return Vec::new();
        };
        let was_playing = self.playing_index == Some(index);

        self.queue.remove(index);
        self.queue_rev += 1;
        self.playing_index = match self.playing_index {
            Some(i) if i == index => None,
            Some(i) if i > index => Some(i - 1),
            other => other,
        };

        let events = vec![self.queue_event()];
        if was_playing {
            // Deleting the track that is sounding stops it: the user asked for
            // this row to be gone, and continuing to play it would be a lie.
            self.stop_playback();
        }
        events
    }

    fn stop_playback(&mut self) {
        self.engine.stop();
        self.playing_index = None;
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        self.engine.set_volume(self.volume);
    }

    /// The position of the current track within the active list.
    ///
    /// Derived rather than stored, which is what makes the whole family of
    /// stale-cursor bugs impossible: a cursor that is recomputed from the list
    /// and the current track cannot point past the end of a list that shrank.
    fn active_cursor(&self) -> Option<usize> {
        let path = self.now_playing.as_ref()?.path.clone();
        self.active_list.iter().position(|song| *song == path)
    }

    fn next(&mut self) -> Vec<Event> {
        if let Some(path) = self.next_path() {
            return self.play(&path);
        }
        Vec::new()
    }

    fn prev(&mut self) -> Vec<Event> {
        if let Some(path) = self.prev_path() {
            return self.play(&path);
        }
        Vec::new()
    }

    fn next_path(&self) -> Option<PathBuf> {
        if !self.active_list.is_empty() {
            let len = self.active_list.len();
            // A track that is not in the active list counts as "before the
            // first", so `Next` starts at the top rather than at whatever
            // index a stale cursor happened to hold.
            let cursor = self.active_cursor().unwrap_or(len - 1);
            return Some(self.active_list[(cursor + 1) % len].clone());
        }
        let index = self.playing_index?;
        self.queue.get(index + 1).map(|t| t.path.clone())
    }

    fn prev_path(&self) -> Option<PathBuf> {
        if !self.active_list.is_empty() {
            let len = self.active_list.len();
            let cursor = self.active_cursor().unwrap_or(0);
            let prev = if cursor == 0 { len - 1 } else { cursor - 1 };
            return Some(self.active_list[prev].clone());
        }
        let index = self.playing_index?;
        index
            .checked_sub(1)
            .and_then(|i| self.queue.get(i))
            .map(|t| t.path.clone())
    }

    /// The track finished on its own: apply the repeat mode and advance.
    fn after_track_ended(&mut self) -> Vec<Event> {
        let repeat = self.repeat;
        let path = self.now_playing.as_ref().map(|n| n.path.clone());

        if let Some(current) = path {
            match repeat {
                RepeatMode::SingleTrack => return self.play(&current),
                RepeatMode::Sequential => {
                    if let Some(next) = self.next_path() {
                        return self.play(&next);
                    }
                }
                RepeatMode::Shuffle => {
                    let pick = self.random_path();
                    if let Some(next) = pick {
                        return self.play(&next);
                    }
                }
            }
        }
        // Nothing to advance to: stop rather than leave a finished track
        // looking like it is still playing.
        self.playing_index = None;
        self.engine.stop();
        Vec::new()
    }

    /// A uniformly random track from the active list, else from the queue.
    fn random_path(&self) -> Option<PathBuf> {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        if !self.active_list.is_empty() {
            let index = rng.gen_range(0..self.active_list.len());
            return Some(self.active_list[index].clone());
        }
        if self.queue.is_empty() {
            return None;
        }
        let index = rng.gen_range(0..self.queue.len());
        Some(self.queue[index].path.clone())
    }

    fn start_fft(&mut self) {
        self.fft_cancel = None; // drop the previous thread's sender first
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(());
        self.fft_cancel = Some(cancel_tx);
        fft::spawn(
            self.engine.pcm_buffer.clone(),
            self.engine.sample_rate_handle(),
            self.bars.clone(),
            self.num_bars,
            self.smoothing,
            cancel_rx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::engine::PlaybackState;

    fn player() -> Player {
        Player::new_headless(&Config::default())
    }

    /// A real file from the fixture set — the metadata reader has to be able
    /// to open it, or `play` reports instead of queueing.
    fn fixture(name: &str) -> PathBuf {
        std::fs::canonicalize(format!("tests/fixtures/{name}")).expect("fixture file")
    }

    fn queue_of(paths: &[&str]) -> Vec<QueueTrack> {
        paths
            .iter()
            .map(|p| QueueTrack {
                path: PathBuf::from(p),
                title: p.to_string(),
                artist: "X".into(),
                duration_secs: 1.0,
            })
            .collect()
    }

    /// Load a queue without touching the filesystem or the engine's decoder.
    fn seed_queue(player: &mut Player, paths: &[&str]) {
        player.queue = queue_of(paths);
        player.queue_rev += 1;
    }

    // ── Playing ──

    #[tokio::test]
    async fn play_queues_the_track_and_starts_it() {
        let mut player = player();
        player.execute(Request::Play {
            path: fixture("test.wav"),
        });

        let state = player.state();
        assert_eq!(state.title, "test"); // the fixture carries tags
        assert!(state.duration_secs > 0.0);
        assert_eq!(player.queue().len(), 1);
        assert_eq!(state.playing_index, Some(0));
        assert!(state.status.is_active());
    }

    /// A path that cannot be read is reported, not queued: a queue entry the
    /// player can never start is worse than a message saying why.
    #[tokio::test]
    async fn a_missing_file_reports_a_notice_and_queues_nothing() {
        let mut player = player();
        let events = player.execute(Request::Play {
            path: PathBuf::from("/definitely/not/here.flac"),
        });

        assert!(player.queue().is_empty());
        assert!(events.iter().any(|e| matches!(e, Event::Notice { .. })));
    }

    #[tokio::test]
    async fn playing_the_same_track_twice_does_not_duplicate_it() {
        let mut player = player();
        for _ in 0..2 {
            player.execute(Request::Play {
                path: fixture("test.wav"),
            });
        }
        assert_eq!(player.queue().len(), 1);
    }

    /// Re-queueing must not bump the revision — the client would refetch a
    /// queue that did not change.
    #[tokio::test]
    async fn the_queue_revision_only_moves_when_the_queue_does() {
        let mut player = player();
        player.execute(Request::Play {
            path: fixture("test.wav"),
        });
        let rev = player.state().queue_rev;

        player.execute(Request::Play {
            path: fixture("test.wav"),
        });
        assert_eq!(player.state().queue_rev, rev);

        player.execute(Request::QueuePush {
            path: fixture("test.flac"),
        });
        assert!(player.state().queue_rev > rev);
    }

    #[tokio::test]
    async fn the_queue_event_is_sent_only_when_the_queue_changes() {
        let mut player = player();
        let first = player.execute(Request::Play {
            path: fixture("test.wav"),
        });
        assert!(first.iter().any(|e| matches!(e, Event::Queue { .. })));

        let again = player.execute(Request::Play {
            path: fixture("test.wav"),
        });
        assert!(!again.iter().any(|e| matches!(e, Event::Queue { .. })));
    }

    // ── Transport ──

    #[tokio::test]
    async fn toggle_pauses_and_resumes() {
        let mut player = player();
        player.execute(Request::Play {
            path: fixture("test.wav"),
        });
        assert!(player.state().status.is_active());

        player.execute(Request::Toggle);
        assert_eq!(player.state().status, PlaybackState::Paused);

        player.execute(Request::Toggle);
        assert!(player.state().status.is_active());
    }

    #[test]
    fn volume_steps_are_clamped_and_reported() {
        let mut player = player();
        player.execute(Request::SetVolume { volume: 0.5 });
        assert!((player.state().volume - 0.5).abs() < 1e-6);

        for _ in 0..50 {
            player.execute(Request::VolumeStep { delta: 0.05 });
        }
        assert!((player.state().volume - 1.0).abs() < 1e-6);

        for _ in 0..50 {
            player.execute(Request::VolumeStep { delta: -0.05 });
        }
        assert!((player.state().volume - 0.0).abs() < 1e-6);
    }

    #[test]
    fn repeat_mode_is_reported_back() {
        let mut player = player();
        player.execute(Request::SetRepeat {
            mode: RepeatMode::SingleTrack,
        });
        assert_eq!(player.state().repeat, RepeatMode::SingleTrack);
    }

    // ── Queue removal ──

    #[test]
    fn removing_a_track_shifts_the_playing_index() {
        let mut player = player();
        seed_queue(&mut player, &["/a.flac", "/b.flac", "/c.flac"]);
        player.playing_index = Some(2);

        player.execute(Request::QueueRemove {
            path: PathBuf::from("/a.flac"),
        });

        assert_eq!(player.state().playing_index, Some(1));
        assert_eq!(player.queue().len(), 2);
    }

    /// Deleting the row that is sounding stops the sound — that is what the
    /// user asked for, and `dd` on the playing track has always meant it.
    #[test]
    fn removing_the_playing_track_stops_playback() {
        let mut player = player();
        seed_queue(&mut player, &["/a.flac", "/b.flac"]);
        player.playing_index = Some(0);

        player.execute(Request::QueueRemove {
            path: PathBuf::from("/a.flac"),
        });

        assert_eq!(player.state().playing_index, None);
        assert_eq!(player.state().status, PlaybackState::Stopped);
    }

    #[test]
    fn removing_an_unknown_path_is_a_noop() {
        let mut player = player();
        seed_queue(&mut player, &["/a.flac"]);
        let rev = player.state().queue_rev;

        player.execute(Request::QueueRemove {
            path: PathBuf::from("/nope.flac"),
        });

        assert_eq!(player.queue().len(), 1);
        assert_eq!(player.state().queue_rev, rev);
    }

    // ── Next / prev ──

    #[test]
    fn next_stops_at_the_end_of_the_queue_and_prev_at_the_start() {
        let mut player = player();
        seed_queue(&mut player, &["/a.flac", "/b.flac", "/c.flac"]);

        player.playing_index = Some(2);
        assert_eq!(player.next_path(), None);

        player.playing_index = Some(0);
        assert_eq!(player.prev_path(), None);
    }

    #[test]
    fn next_and_prev_walk_the_queue() {
        let mut player = player();
        seed_queue(&mut player, &["/a.flac", "/b.flac", "/c.flac"]);

        player.playing_index = Some(1);
        assert_eq!(player.next_path(), Some(PathBuf::from("/c.flac")));
        assert_eq!(player.prev_path(), Some(PathBuf::from("/a.flac")));
    }

    #[test]
    fn next_and_prev_without_a_current_track_do_nothing() {
        let mut player = player();
        seed_queue(&mut player, &["/a.flac"]);
        player.playing_index = None;

        assert_eq!(player.next_path(), None);
        assert_eq!(player.prev_path(), None);
    }

    // ── The active list ──

    /// With a list open the cursor is derived from the current track, so a
    /// list that shrank under a stale index cannot be run off the end — the
    /// bug class the old stored cursor kept producing.
    #[test]
    fn the_active_list_takes_priority_over_the_queue() {
        let mut player = player();
        seed_queue(&mut player, &["/a.flac", "/b.flac", "/c.flac", "/d.flac"]);
        player.active_list = vec![PathBuf::from("/x.flac"), PathBuf::from("/y.flac")];
        player.now_playing = Some(NowPlaying {
            path: PathBuf::from("/x.flac"),
            ..now_playing_dummy()
        });

        assert_eq!(player.next_path(), Some(PathBuf::from("/y.flac")));
        assert_eq!(player.prev_path(), Some(PathBuf::from("/y.flac")));
    }

    #[test]
    fn the_active_list_wraps_in_both_directions() {
        let mut player = player();
        player.active_list = vec![PathBuf::from("/x.flac"), PathBuf::from("/y.flac")];
        player.now_playing = Some(NowPlaying {
            path: PathBuf::from("/y.flac"),
            ..now_playing_dummy()
        });

        assert_eq!(player.next_path(), Some(PathBuf::from("/x.flac")));
        assert_eq!(player.prev_path(), Some(PathBuf::from("/x.flac")));
    }

    /// A track that is not in the open list is "before the first": `Next`
    /// starts at the top instead of resuming from wherever the last one left
    /// off, which is what the stored cursor used to do.
    #[test]
    fn next_from_outside_the_active_list_starts_at_the_top() {
        let mut player = player();
        player.active_list = vec![PathBuf::from("/x.flac"), PathBuf::from("/y.flac")];
        player.now_playing = Some(NowPlaying {
            path: PathBuf::from("/somewhere_else.flac"),
            ..now_playing_dummy()
        });

        assert_eq!(player.next_path(), Some(PathBuf::from("/x.flac")));
        assert_eq!(player.prev_path(), Some(PathBuf::from("/y.flac")));
    }

    #[test]
    fn an_empty_active_list_falls_back_to_the_queue() {
        let mut player = player();
        seed_queue(&mut player, &["/a.flac", "/b.flac"]);
        player.playing_index = Some(0);
        player.active_list = Vec::new();

        assert_eq!(player.next_path(), Some(PathBuf::from("/b.flac")));
    }

    fn now_playing_dummy() -> NowPlaying {
        NowPlaying {
            path: PathBuf::from("/dummy.flac"),
            title: "Dummy".into(),
            artist: "X".into(),
            album: String::new(),
            genre: String::new(),
            year: String::new(),
            codec: "wav".into(),
            duration_secs: 1.0,
        }
    }

    // ── Auto-advance ──

    #[tokio::test]
    async fn a_finished_track_advances_in_sequential_mode() {
        let mut player = player();
        seed_queue(&mut player, &["/a.flac", "/b.flac"]);
        player.now_playing = Some(NowPlaying {
            path: PathBuf::from("/a.flac"),
            ..now_playing_dummy()
        });
        player.playing_index = Some(0);

        // The next path does not exist, so `play` reports and stops — what
        // matters is which track it tried.
        let events = player.after_track_ended();
        assert!(events.iter().any(|e| matches!(e, Event::Notice { .. })));
    }

    #[test]
    fn shuffle_picks_a_track_inside_the_active_list() {
        let mut player = player();
        player.active_list = vec![PathBuf::from("/x.flac"), PathBuf::from("/y.flac")];
        for _ in 0..20 {
            let pick = player.random_path().expect("a pick");
            assert!(player.active_list.contains(&pick));
        }
    }

    #[test]
    fn shuffle_with_nothing_to_pick_returns_none() {
        let player = player();
        assert_eq!(player.random_path(), None);
    }

    // ── FFT subscription ──

    #[tokio::test]
    async fn the_spectrum_is_only_produced_while_subscribed() {
        let mut player = player();
        assert!(!player.fft_subscribed);

        player.execute(Request::SubscribeVisualizer { on: true });
        assert!(player.fft_subscribed);

        player.execute(Request::SubscribeVisualizer { on: false });
        assert!(!player.fft_subscribed);
        assert!(player.fft_cancel.is_none(), "the thread must be stopped");
    }

    // ── Shutdown ──

    #[test]
    fn shutdown_is_reported_once_requested() {
        let mut player = player();
        assert!(!player.should_shutdown());
        player.execute(Request::Shutdown);
        assert!(player.should_shutdown());
        assert_eq!(player.state().status, PlaybackState::Stopped);
    }
}
