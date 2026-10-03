//! The playback core: one audio engine, one queue, and the policy that
//! decides what plays next.
//!
//! This is the half of the old `app` module that survives the TUI. Everything
//! here is **synchronous and testable** — [`Player::execute`] takes a
//! [`Request`] and hands back the [`Event`]s it produced. The socket daemon
//! and the client's in-process handle both drive this same function, so the
//! tested path and the shipped path are the same path.

pub mod fft;
pub mod library;
pub mod persistence;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::audio::engine::{AudioEngine, PlaybackEvent, PlaybackState};
use crate::config::Config;
use crate::error::AppResult;
use crate::ipc::proto::{Event, NoticeLevel, QueueTrack, RepeatMode, Request, StateSnapshot};
use crate::metadata::reader::read_metadata;

use self::library::{Library, ScanNotice};

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

impl NowPlaying {
    /// The four fields a queue row carries, and nothing else.
    ///
    /// For putting a restored session back on the deck: album, genre, year and
    /// codec live only in the tags, and re-reading them at startup would mean
    /// touching the file (and failing) on the way to showing a title. They
    /// fill in the first time the track is started.
    fn from_queue_track(track: &QueueTrack) -> Self {
        Self {
            path: track.path.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: String::new(),
            genre: String::new(),
            year: String::new(),
            codec: String::new(),
            duration_secs: track.duration_secs,
        }
    }
}

pub struct Player {
    engine: AudioEngine,
    /// The index of what is on disk, and the scanner that fills it. Moved
    /// here in phase 2: SQLite has one writer, and two TUIs are two clients.
    library: Library,
    /// The global queue. Insertion-ordered, unique by path.
    queue: Vec<QueueTrack>,
    playing_index: Option<usize>,
    /// The songs of the playlist the user last opened, if any. `Next`/`Prev`
    /// and auto-advance prefer this over the queue, exactly as the UI did
    /// when it owned the policy.
    active_list: Vec<PathBuf>,
    now_playing: Option<NowPlaying>,
    /// The track the last run ended on, from `state.json`.
    ///
    /// Startup deliberately does not resume it — a player that makes noise the
    /// moment it exists is a player you learn to fear — but it is what `Resume`
    /// falls back to when the deck is empty, so `play` after a restart means
    /// "carry on" instead of "nothing happened".
    last_track: Option<PathBuf>,
    /// Where a restored session's needle was, in seconds, until it is used.
    ///
    /// A session the daemon saved on its way out comes back parked: the deck
    /// has a track on it and this is the offset it will start from, but
    /// nothing is sounding. It is consumed by the first start — after that the
    /// engine's own clock is the truth again — so a `stop` after a restore
    /// still rewinds rather than resuming the old position.
    resume_at: Option<f64>,
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
        Ok(Self::with_engine(
            config,
            AudioEngine::new()?,
            Library::open(),
        ))
    }

    /// Device-free player for tests: the sink's queue receiver is dropped, so
    /// nothing opens ALSA/PulseAudio and nothing actually sounds. The library
    /// is in memory as well — a test that wrote to the real `library.db` would
    /// leave rows for the next test to find.
    #[cfg(test)]
    pub fn new_headless(config: &Config) -> Self {
        Self::with_engine(config, AudioEngine::new_headless(), Library::in_memory())
    }

    fn with_engine(config: &Config, engine: AudioEngine, library: Library) -> Self {
        Self {
            engine,
            library,
            queue: Vec::new(),
            playing_index: None,
            active_list: Vec::new(),
            now_playing: None,
            last_track: None,
            resume_at: None,
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
        // Scanner messages first: they are the index being written, and a
        // client that asked for a scan should hear about it before it hears
        // about the clock again.
        for notice in self.library.drain() {
            events.push(match notice {
                ScanNotice::Progress { scanned, changed } => {
                    Event::ScanProgress { scanned, changed }
                }
                ScanNotice::Finished(report) => Event::ScanFinished(report),
            });
        }
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
                    Vec::new()
                } else {
                    // Deliberately `resume` and not `engine.resume()`: the key
                    // that silences the player has to be the key that brings it
                    // back, whatever silence it was.
                    self.resume()
                }
            }
            Request::Pause => {
                self.engine.pause();
                Vec::new()
            }
            Request::Resume => self.resume(),
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
            Request::LibraryArtists => vec![Event::LibraryArtists {
                artists: self.library.artists(),
            }],
            Request::LibraryAlbums { artist } => vec![Event::LibraryAlbums {
                albums: self.library.albums_by(&artist),
                artist,
            }],
            Request::LibraryTracks { artist, album } => vec![Event::LibraryTracks {
                tracks: self.library.tracks_by(&artist, &album),
                artist,
                album,
            }],
            Request::SearchLibrary { query } => vec![Event::SearchResults {
                tracks: self.library.search(&query),
                query,
            }],
            Request::AddLibraryPath { path } => {
                let mut events = self.add_library_path(&path);
                events.push(self.library_paths_event());
                events
            }
            Request::IndexLibrary => {
                self.index_library();
                Vec::new()
            }
            Request::CancelScan => {
                self.library.cancel_scans();
                Vec::new()
            }
            Request::RemoveLibraryPath { root } => {
                // The queue is deliberately untouched: it is what is playing
                // now, not what the collection claims. A track that is
                // sounding keeps sounding, and `QueueRemove` is the verb for
                // taking a row out of the queue.
                self.library.remove_path(&root);
                vec![self.library_paths_event()]
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
                // Deliberately not silenced here. `Player::shutdown` does that,
                // and it runs *after* the daemon has written `state.json` —
                // stopping first would rewind the position out from under the
                // save and lose the user's place on every `tmper quit`.
                self.shutdown = true;
                Vec::new()
            }
            Request::Hello { .. } => Vec::new(),
        }
    }

    /// Whether a [`Request::Shutdown`] has arrived — `tmper quit`, `:quit!`,
    /// or eventually MPRIS `Quit`. The daemon reads it after every command and
    /// stops when it is set.
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
            position_secs: self.position_secs(),
            volume: self.volume,
            repeat: self.repeat,
            playing_index: self.playing_index,
            queue_rev: self.queue_rev,
            lyrics_offset_ms: self.lyrics_offset_ms,
        }
    }

    /// Where the needle is: the engine's clock while a session exists, and the
    /// parked offset of one that was restored but not started yet.
    ///
    /// Asked of the player rather than of the engine directly because the two
    /// differ exactly once — a restored session reports zero from an engine
    /// that has no session, and the place `Resume` will start from is the
    /// honest answer there. It is also what `save_state` writes, so a daemon
    /// that comes up and goes back down without being played keeps the
    /// position instead of forgetting it on the second exit.
    pub fn position_secs(&self) -> f64 {
        match self.engine.state() {
            state if state.is_active() || matches!(state, PlaybackState::Paused) => {
                self.engine.position_secs()
            }
            _ => self.resume_at.unwrap_or(0.0),
        }
    }

    /// The queue as the client mirrors it.
    pub fn queue(&self) -> Vec<QueueTrack> {
        self.queue.clone()
    }

    /// The collection's paths, as the browser panel lists them.
    pub fn library_paths(&self) -> Vec<PathBuf> {
        self.library.paths().to_vec()
    }

    // ── The collection ──

    /// Remember a path and make the collection true of it right away: a
    /// directory is walked, a single file is read and put on the deck.
    ///
    /// Called by [`Request::AddLibraryPath`] and by [`Player::attach_scanner`]
    /// for the paths that were already there — the same work either way, and
    /// deliberately idempotent, because the second call is what a re-add means.
    ///
    /// A single file lands in the queue as well, which is the one part of this
    /// a client has to hear about; the caller forwards what comes back.
    fn add_library_path(&mut self, path: &Path) -> Vec<Event> {
        self.library.add_path(path);
        if path.is_dir() {
            self.library.start_scan(path);
            Vec::new()
        } else {
            self.library.ensure_indexed(&[path.to_path_buf()]);
            self.push_to_queue(path)
        }
    }

    /// Read the tags of everything the daemon already knows about: the paths
    /// the user added, and the queue — a track that was played is part of the
    /// collection whether or not a scan has seen it.
    fn index_library(&mut self) {
        let mut paths = self.library.paths().to_vec();
        paths.extend(self.queue.iter().map(|track| track.path.clone()));
        self.library.ensure_indexed(&paths);
    }

    fn library_paths_event(&self) -> Event {
        Event::LibraryPaths {
            paths: self.library.paths().to_vec(),
        }
    }

    /// Hand the scanner a runtime to walk on. Called once, by the daemon: the
    /// client has no business spawning work in the player's process, and a
    /// scan requested without one is refused rather than attempted.
    ///
    /// The paths the collection already had are picked up here too, which is
    /// where the runtime becomes available and therefore the first moment a
    /// walk can be started. A daemon that comes up with a library re-reads it;
    /// a client that comes and goes does not restart anything.
    pub fn attach_scanner(&mut self, handle: tokio::runtime::Handle) {
        self.library.attach_scanner(handle);
        for path in self.library.paths().to_vec() {
            // Nobody is attached yet, so the events a restored file produces
            // have no one to go to; a fresh connection is told the queue.
            let _ = self.add_library_path(&path);
        }
    }

    /// The index, for tests that seed or assert on it directly.
    #[cfg(test)]
    pub fn library(&self) -> &Library {
        &self.library
    }

    #[cfg(test)]
    pub fn library_mut(&mut self) -> &mut Library {
        &mut self.library
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

    /// Load a track, queueing it first if it is new, and start it from the top.
    ///
    /// This is the one way a track starts, wherever the request came from: a
    /// keypress, the browser, the CLI, and (later) MPRIS.
    fn play(&mut self, path: &Path) -> Vec<Event> {
        self.start(path, 0.0)
    }

    /// [`Player::play`], but with the needle placed `offset_secs` in.
    ///
    /// Only a restored session asks for the offset; everything a user clicks
    /// on starts at the beginning, and this is the same code path so the two
    /// cannot drift apart.
    fn start(&mut self, path: &Path, offset_secs: f64) -> Vec<Event> {
        // Whatever a restore parked here is spent: this call decides where the
        // needle goes, and leaving it set would replay the offset on the next
        // start after a stop.
        self.resume_at = None;
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

        // A decoder seeked onto the very last sample finishes the moment it
        // starts, so a restored needle lands just short of the end —
        // `seek_relative` clamps to the same fraction for the same reason.
        let offset_secs = if duration_secs > 0.0 {
            offset_secs.clamp(0.0, duration_secs * 0.999)
        } else {
            offset_secs.max(0.0)
        };

        match self.engine.play_file_at(path, offset_secs) {
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
            // Unload rather than stop, or `play` would faithfully start the
            // track the user just deleted.
            self.unload();
        }
        events
    }

    /// Put sound back in the air: unpause, or start the loaded track over.
    ///
    /// "Resume" is deliberately wider than unpausing. [`Request::Stop`] does
    /// not unload the track — the deck still has something on it — and a queue
    /// that runs out is silenced the same way, so in both the only honest
    /// reading of *play* is to start that track again. Answering `Paused` alone
    /// (which is all this used to do through `engine.resume`) made `tmper stop`
    /// a dead end: `tmper play`, the TUI's space bar and every media key after
    /// it did nothing at all, and a stop was indistinguishable from a pause
    /// that could not be undone.
    fn resume(&mut self) -> Vec<Event> {
        match self.engine.state() {
            // Already on its way. Without this, a second press while the
            // decoder is still loading would restart the track from the top.
            state if state.is_active() => Vec::new(),
            PlaybackState::Paused => {
                self.engine.resume();
                Vec::new()
            }
            // Stopped, or a load that failed. With even the deck empty the
            // track the last run ended on is the answer — otherwise a
            // restarted daemon turns `play` into a key that does nothing.
            //
            // Where it starts from is the difference between a `stop` and a
            // restored session: a stop rewound the position and `resume_at` is
            // empty, so this plays from the top; a session that came back from
            // `state.json` parked its needle, so this carries on from there.
            _ => {
                let path = self
                    .now_playing
                    .as_ref()
                    .map(|now| now.path.clone())
                    .or_else(|| self.last_track.clone());
                let offset = self.resume_at.unwrap_or(0.0);
                match path {
                    Some(path) => self.start(&path, offset),
                    None => Vec::new(),
                }
            }
        }
    }

    /// Silence the player, keeping the track on the deck so [`Player::resume`]
    /// can start it again. This is what [`Request::Stop`] means.
    fn stop_playback(&mut self) {
        self.engine.stop();
        self.playing_index = None;
        // A stop rewinds, and a parked position is precisely what a rewind
        // undoes. Leaving it set would make a stop *after* a restore resume
        // from the old place instead of starting the track over — the one
        // thing `stop` is defined not to do.
        self.resume_at = None;
    }

    /// Silence the player *and* take the track off the deck.
    ///
    /// For the cases where the track is gone rather than merely quiet. After
    /// this `resume` has nothing to offer — which is the point: a track the
    /// user deleted from the queue must not come back on the next keypress.
    fn unload(&mut self) {
        self.stop_playback();
        self.now_playing = None;
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

    /// `stop` is not `pause`: it rewinds, so what `play` does afterwards is
    /// start the track over rather than continue it.
    #[tokio::test]
    async fn stop_then_play_starts_the_track_again() {
        let mut player = player();
        player.execute(Request::Play {
            path: fixture("test.wav"),
        });

        player.execute(Request::Stop);
        assert_eq!(player.state().status, PlaybackState::Stopped);
        // The deck still has the track on it — that is what makes `play` work.
        assert!(player.state().path.is_some());

        player.execute(Request::Resume);
        assert!(
            player.state().status.is_active(),
            "play after stop must bring the track back"
        );
        // From the top, not from where it was silenced.
        assert!(player.state().position_secs < 0.05);
    }

    /// The same wall the CLI hits, hit by the key that is supposed to be the
    /// transport: space after a stop has to bring the music back.
    #[tokio::test]
    async fn toggle_after_a_stop_plays_again() {
        let mut player = player();
        player.execute(Request::Play {
            path: fixture("test.wav"),
        });
        player.execute(Request::Stop);

        player.execute(Request::Toggle);
        assert!(
            player.state().status.is_active(),
            "space after a stop must not be a dead key"
        );
    }

    /// With nothing loaded there is nothing to bring back, and inventing a
    /// track would be worse than staying quiet.
    #[test]
    fn resume_with_an_empty_deck_does_nothing() {
        let mut player = player();
        player.execute(Request::Resume);
        player.execute(Request::Toggle);
        assert_eq!(player.state().status, PlaybackState::Stopped);
        assert_eq!(player.state().path, None);
    }

    /// Deleting the playing row takes the track off the deck, so `play` cannot
    /// resurrect what the user just removed.
    #[tokio::test]
    async fn a_deleted_track_is_not_resumable() {
        let mut player = player();
        player.execute(Request::Play {
            path: fixture("test.wav"),
        });
        player.execute(Request::QueueRemove {
            path: fixture("test.wav"),
        });

        assert_eq!(player.state().path, None, "the deck should be empty");
        player.execute(Request::Resume);
        assert_eq!(player.state().status, PlaybackState::Stopped);
    }

    /// A stop rewinds, which is what separates it from a pause — and why the
    /// `play` after it is a restart rather than a continuation.
    #[tokio::test]
    async fn stop_rewinds_the_position() {
        let mut player = player();
        player.execute(Request::Play {
            path: fixture("test.wav"),
        });
        // Let the clock run so there is a position to lose.
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        assert!(player.state().position_secs > 0.0);

        player.execute(Request::Stop);
        assert_eq!(player.state().position_secs, 0.0);
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

    // ── Library ──

    /// Seed one index row without touching disk: these tests are about the
    /// questions the daemon answers, not about the tag reader.
    fn index_track(player: &mut Player, path: &str, title: &str, artist: &str, album: &str) {
        player
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
                10_000,
                1_000,
            )
            .expect("upsert failed");
    }

    #[test]
    fn the_artist_list_comes_back_as_an_event() {
        let mut player = player();
        index_track(&mut player, "/music/a.flac", "A", "Artist", "Album");

        let events = player.execute(Request::LibraryArtists);
        let Some(Event::LibraryArtists { artists }) = events.first() else {
            panic!("expected an artist answer, got {events:?}");
        };
        assert_eq!(artists, &vec!["Artist".to_string()]);
    }

    /// Both answers carry the cursor key back, so the client can drop a reply
    /// that arrived after the user moved on.
    #[test]
    fn album_and_track_answers_echo_the_key_they_were_asked_for() {
        let mut player = player();
        index_track(&mut player, "/music/a.flac", "A", "Artist", "Album");

        let events = player.execute(Request::LibraryAlbums {
            artist: "Artist".into(),
        });
        assert!(
            matches!(
                events.first(),
                Some(Event::LibraryAlbums { artist, albums })
                    if artist == "Artist" && albums == &vec!["Album".to_string()]
            ),
            "got {events:?}"
        );

        let events = player.execute(Request::LibraryTracks {
            artist: "Artist".into(),
            album: "Album".into(),
        });
        let Some(Event::LibraryTracks {
            artist,
            album,
            tracks,
        }) = events.first()
        else {
            panic!("expected a track answer, got {events:?}");
        };
        assert_eq!((artist.as_str(), album.as_str()), ("Artist", "Album"));
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title, "A");
        assert_eq!(tracks[0].artist, "Artist");
    }

    #[test]
    fn a_search_reports_the_query_it_answered() {
        let mut player = player();
        index_track(&mut player, "/music/a.flac", "Nightfall", "Artist", "Album");

        let events = player.execute(Request::SearchLibrary {
            query: "night".into(),
        });
        let Some(Event::SearchResults { query, tracks }) = events.first() else {
            panic!("expected search results, got {events:?}");
        };
        assert_eq!(query, "night");
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].path, PathBuf::from("/music/a.flac"));
    }

    /// A track the user played belongs in the collection even if no scan ever
    /// walked its directory — and the client cannot say so, because it does not
    /// know the queue the daemon is holding.
    #[test]
    fn indexing_the_library_also_indexes_the_queue() {
        let mut player = player();
        player.set_queue(vec![QueueTrack {
            path: fixture("test.wav"),
            title: "test".into(),
            artist: "X".into(),
            duration_secs: 2.0,
        }]);

        player.execute(Request::IndexLibrary);

        let row = player
            .library()
            .db()
            .get_by_path(&fixture("test.wav").to_string_lossy())
            .expect("query")
            .expect("the queue's track should be indexed");
        assert_eq!(row.title, "test");
    }

    /// The paths the user added are the daemon's list now, and adding one is
    /// what makes the collection true of it: a directory is walked, a file is
    /// read and put on the deck.
    #[tokio::test]
    async fn adding_a_path_remembers_it_and_indexes_it() {
        let mut player = player();
        let events = player.execute(Request::AddLibraryPath {
            path: fixture("test.wav"),
        });
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::LibraryPaths { paths } if paths.len() == 1)),
            "the list comes back whole: {events:?}"
        );
        assert_eq!(player.library_paths(), vec![fixture("test.wav")]);
        assert_eq!(player.queue().len(), 1, "a picked file joins the queue");
        assert!(
            player
                .library()
                .db()
                .get_by_path(&fixture("test.wav").to_string_lossy())
                .expect("query")
                .is_some(),
            "…and the index"
        );

        // Adding it again does not duplicate it.
        player.execute(Request::AddLibraryPath {
            path: fixture("test.wav"),
        });
        assert_eq!(player.library_paths().len(), 1);
        assert_eq!(player.queue().len(), 1);
    }

    #[test]
    fn removing_a_path_drops_it_from_the_list_too() {
        let mut player = player();
        player.execute(Request::AddLibraryPath {
            path: PathBuf::from("/music"),
        });
        index_track(&mut player, "/music/a.flac", "A", "Artist", "Album");

        let events = player.execute(Request::RemoveLibraryPath {
            root: PathBuf::from("/music"),
        });

        assert!(matches!(
            events.first(),
            Some(Event::LibraryPaths { paths }) if paths.is_empty()
        ));
        assert!(player.library_paths().is_empty());
        assert!(player.library().artists().is_empty(), "rows are gone too");
    }

    /// Forgetting a path is an edit to the collection, not to what is playing:
    /// a track that is sounding keeps sounding.
    #[test]
    fn forgetting_a_path_drops_its_rows_and_leaves_the_queue_alone() {
        let mut player = player();
        index_track(&mut player, "/music/a.flac", "A", "Artist", "Album");
        player.set_queue(vec![QueueTrack {
            path: PathBuf::from("/music/a.flac"),
            title: "A".into(),
            artist: "Artist".into(),
            duration_secs: 1.0,
        }]);

        player.execute(Request::RemoveLibraryPath {
            root: PathBuf::from("/music"),
        });

        assert!(player.library().artists().is_empty(), "rows should be gone");
        assert_eq!(player.queue().len(), 1, "the queue is not the library");
    }

    /// Without a runtime there is nothing to walk on, and a scan is refused
    /// rather than allowed to panic inside `spawn_blocking`.
    #[test]
    fn a_scan_without_a_runtime_is_refused() {
        let mut player = player();
        player.execute(Request::AddLibraryPath {
            path: PathBuf::from("tests/fixtures"),
        });
        assert_eq!(player.library().scans_active(), 0);
        assert_eq!(player.library_paths(), [PathBuf::from("tests/fixtures")]);

        player.execute(Request::CancelScan); // and cancelling is a no-op
    }

    /// The tick is where a scanner thread's report becomes a client's event.
    #[tokio::test]
    async fn a_finished_scan_reaches_the_client_as_an_event() {
        let mut player = player();
        player.attach_scanner(tokio::runtime::Handle::current());
        player.execute(Request::AddLibraryPath {
            path: PathBuf::from("tests/fixtures"),
        });

        let report = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                for event in player.tick() {
                    if let Event::ScanFinished(report) = event {
                        return report;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the scan should finish");

        assert!(report.complete);
        assert_eq!(report.failed, 0);
        assert!(report.scanned >= 3, "three fixtures at least: {report:?}");
        assert_eq!(report.active, 0);
        assert_eq!(player.library().artists().len(), 1, "test files carry tags");
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
