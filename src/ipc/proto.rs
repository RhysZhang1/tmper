//! The wire contract between the daemon and its clients.
//!
//! One JSON object per line, UTF-8, `\n`-terminated — [`super`] owns the
//! framing, this module owns the vocabulary. Clients send [`Request`]s;
//! everything the daemon has to say comes back as an [`Event`], *including the
//! answer to a question*. That asymmetry is deliberate: a client is usually a
//! TUI sitting in a `select!` loop, and it must never block waiting for a
//! reply.
//!
//! The state the daemon owns is pushed whole ([`StateSnapshot`]) rather than as
//! deltas. A snapshot is a few hundred bytes, it removes an entire class of
//! desync bugs, and it means a client that reconnects after a gap needs no
//! catch-up protocol — the next snapshot is the catch-up.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::audio::engine::PlaybackState;

/// Bumped whenever an existing field changes meaning or is removed. Additive
/// changes (a new `Request` variant, a new optional field) do not need a bump;
/// a client that does not know a tag simply never sends it.
pub const PROTOCOL_VERSION: u32 = 1;

/// Playback order. Daemon policy — it survives the TUI that set it, and it is
/// what decides the next track when nobody is attached — so it is declared
/// here rather than in the UI. `ui` re-exports it for the ~45 call sites that
/// still spell it `crate::ui::RepeatMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatMode {
    Sequential,
    Shuffle,
    SingleTrack,
}

/// One entry of the playback queue.
///
/// Deliberately *not* `ui::TrackDisplay`, which is the same four fields but is
/// a rendering type: the library views use it for rows that never enter a
/// queue, and the queue crosses the socket. The client maps one into the other
/// in a single place (`App::apply_event`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueueTrack {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub duration_secs: f64,
}

/// What one finished scan did, as counted by the walker and the index.
///
/// Sent whole rather than as a sentence: the numbers are the daemon's, the
/// sentence about them is the view's — the daemon has no keys to advertise and
/// no status bar to fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanReport {
    pub scanned: usize,
    pub changed: usize,
    /// Rows dropped because the files are gone from disk. Zero unless the
    /// walk was complete and was not cancelled.
    pub removed: usize,
    pub failed: usize,
    pub cancelled: bool,
    /// The walk could read everything it saw. `false` means the listing was
    /// partial, and a partial listing is not evidence that anything is gone.
    pub complete: bool,
    /// Scans still running after this one ended.
    pub active: usize,
}

/// One row of a library answer.
///
/// Deliberately not [`QueueTrack`]: a row in the index carries no duration,
/// and its artist may be missing from the tags entirely. What the row should
/// *look* like — `title`, or `title — artist` for a search that mixes artists
/// together — is the view's decision, so both labels cross and neither is
/// pre-formatted here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackLine {
    pub path: PathBuf,
    pub title: String,
    /// Empty when the tags carry none.
    pub artist: String,
}

/// Everything a client can ask for.
///
/// Fire-and-forget: the daemon answers with events, never with a return value.
/// A command that fails produces [`Event::Notice`] (and no state change), not
/// an error reply — the client's keypress handler has nothing to do with a
/// `Result` anyway.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Request {
    /// First message on every connection. The daemon refuses a client whose
    /// [`PROTOCOL_VERSION`] it does not speak.
    Hello {
        proto: u32,
        version: String,
    },

    /// Play a file: add it to the queue if absent, then start it. This is the
    /// old `load_and_play`, and it is the only way a track starts.
    Play {
        path: PathBuf,
    },
    /// Space: pause if it is sounding, otherwise play.
    Toggle,
    /// Hold the position: `Resume` continues from here.
    Pause,
    /// Make sound come out again — unpause, or start the loaded track over if
    /// it was stopped. Distinct from [`Request::Play`], which names a file.
    Resume,
    /// Silence the player, releasing the audio device, and rewind. The track
    /// stays loaded, so `Resume` starts it again from the top; distinct from
    /// [`Request::Shutdown`], which ends the daemon.
    Stop,
    Next,
    Prev,
    /// `←`/`→`: relative seek, negative goes back.
    SeekRelative {
        secs: f64,
    },
    /// Absolute volume, used by `:volume` and by MPRIS.
    SetVolume {
        volume: f32,
    },
    /// Relative volume, used by the `-`/`=` keys. Relative matters: the keys
    /// auto-repeat, and a client computing an absolute value from its own
    /// (slightly stale) mirror loses steps under fast repeat.
    VolumeStep {
        delta: f32,
    },
    SetRepeat {
        mode: RepeatMode,
    },
    /// Append a track to the queue without playing it (browser `Enter`, and
    /// the rest of a directory when a file in it is played).
    QueuePush {
        path: PathBuf,
    },
    /// Remove by path, not index: the index is a cursor into the client's
    /// mirror and can be one push stale, while a queue has unique paths.
    QueueRemove {
        path: PathBuf,
    },
    /// The list that `Next`/`Prev`/auto-advance walk, when the user has one
    /// open. The client owns the playlist store for now and pushes a copy
    /// here whenever the active one changes or is edited; an empty list means
    /// "no active list, use the queue". Phase 2 moves the store itself into
    /// the daemon and deletes this.
    SetActiveList {
        songs: Vec<PathBuf>,
    },

    /// The artist list — the library panel's first question.
    LibraryArtists,
    /// The albums of one artist. The artist comes back with the answer so a
    /// client can drop a reply that lost a race with the cursor: two `j`
    /// presses put two of these on the wire, and only the last one describes
    /// where the user actually is.
    LibraryAlbums {
        artist: String,
    },
    /// The tracks of one album of one artist, for the same reason.
    LibraryTracks {
        artist: String,
        album: String,
    },
    /// FTS5 prefix search over title, artist, album and genre. The query is
    /// echoed back so a reply cannot land under a newer one.
    SearchLibrary {
        query: String,
    },
    /// Add a directory or a file to the collection: remembered in
    /// `library.json`, indexed now, and — for a directory — walked
    /// incrementally. Sent by the browser's `a` key and by picking a file,
    /// which are the same act with a different root.
    AddLibraryPath {
        path: PathBuf,
    },
    /// Read the tags of anything the daemon already knows about — its library
    /// paths and its queue — that is not in the index yet. A track that was
    /// played belongs in the collection whether or not a scan ever saw it.
    IndexLibrary,
    /// Stop every running scan at its next file boundary.
    CancelScan,
    /// Drop a path from the collection, rows and all: the rows describe files
    /// the library no longer claims.
    RemoveLibraryPath {
        root: PathBuf,
    },

    /// Start or stop the spectrum stream. The daemon runs the FFT thread only
    /// while at least one client is subscribed.
    SubscribeVisualizer {
        on: bool,
    },
    /// `num_bars`/`smoothing` live in the client's `config.toml` but are
    /// consumed by the daemon's FFT thread, so the client pushes them.
    SetFftParams {
        num_bars: u32,
        smoothing: f32,
    },
    /// The lyric offset is the client's to choose and the daemon's to persist
    /// (`state.json` has one writer).
    SetLyricsOffset {
        ms: i64,
    },

    /// Ask for a [`Event::Snapshot`] now — used by `tmper status` and after a
    /// reconnect, where the client needs a state before the next tick.
    GetState,
    /// Stop playback, save state, and exit the daemon. `tmper quit`,
    /// `:quit!`, and MPRIS `Quit`.
    Shutdown,
}

/// Everything the daemon pushes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Event {
    /// First event on a connection, in reply to [`Request::Hello`].
    Welcome {
        proto: u32,
        version: String,
        pid: u32,
    },
    /// Full player state: on every discrete change, and on every tick so the
    /// position advances. Boxed because it is an order of magnitude larger
    /// than any other variant and every `Event` would otherwise carry its size.
    Snapshot(Box<StateSnapshot>),
    /// The whole queue, whenever it changes ([`StateSnapshot::queue_rev`]
    /// tells the client whether it has missed one).
    Queue {
        rev: u64,
        tracks: Vec<QueueTrack>,
    },
    /// Spectrum bars, only while subscribed.
    Visualizer {
        bars: Vec<f32>,
    },
    /// The answers to the library questions. Each carries the key it was asked
    /// for, so a client showing a cursor that has since moved can tell.
    LibraryArtists {
        artists: Vec<String>,
    },
    LibraryAlbums {
        artist: String,
        albums: Vec<String>,
    },
    LibraryTracks {
        artist: String,
        album: String,
        /// In the index's own order, which is by track number.
        tracks: Vec<TrackLine>,
    },
    SearchResults {
        query: String,
        tracks: Vec<TrackLine>,
    },
    /// The collection's paths, whole, whenever they change — and once to every
    /// new client, like the queue. `library.json` has one writer, so this is
    /// what a client renders instead of reading the file.
    LibraryPaths {
        paths: Vec<PathBuf>,
    },
    /// A scan is still walking.
    ScanProgress {
        scanned: usize,
        changed: usize,
    },
    /// A scan ended. The index has already been written and pruned — the
    /// report describes work that is done, not work that is about to start.
    ScanFinished(ScanReport),
    /// A one-line message for the client's notification toast.
    Notice {
        level: NoticeLevel,
        message: String,
    },
    /// The daemon is exiting; clients should leave too.
    Bye,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warn,
    Error,
}

/// The daemon's truth about playback, at one instant.
///
/// What is *not* here is as much a part of the contract as what is: the
/// selected row, the scroll offset, the search query, the open view — every
/// cursor is per-client, and two attached TUIs each own their own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub status: PlaybackState,
    pub path: Option<PathBuf>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub year: String,
    pub codec: String,
    pub duration_secs: f64,
    pub position_secs: f64,
    pub volume: f32,
    pub repeat: RepeatMode,
    /// Index into the queue, if the current track is in it.
    pub playing_index: Option<usize>,
    pub queue_rev: u64,
    pub lyrics_offset_ms: i64,
}

impl Default for StateSnapshot {
    fn default() -> Self {
        Self {
            status: PlaybackState::Stopped,
            path: None,
            title: "No track".into(),
            artist: "—".into(),
            album: String::new(),
            genre: String::new(),
            year: String::new(),
            codec: String::new(),
            duration_secs: 0.0,
            position_secs: 0.0,
            volume: 1.0,
            repeat: RepeatMode::Sequential,
            playing_index: None,
            queue_rev: 0,
            lyrics_offset_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_request(request: Request) {
        let json = serde_json::to_string(&request).expect("serialize");
        let back: Request = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, request, "round trip changed the request: {json}");
    }

    fn round_trip_event(event: Event) {
        let json = serde_json::to_string(&event).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, event, "round trip changed the event: {json}");
    }

    /// Every variant, so adding one without a working serde shape is caught
    /// here rather than at the far end of a socket.
    #[test]
    fn every_request_round_trips() {
        for request in [
            Request::Hello {
                proto: PROTOCOL_VERSION,
                version: "0.1.0".into(),
            },
            Request::Play {
                path: PathBuf::from("/music/一首歌.flac"),
            },
            Request::Toggle,
            Request::Pause,
            Request::Resume,
            Request::Stop,
            Request::Next,
            Request::Prev,
            Request::SeekRelative { secs: -5.0 },
            Request::SetVolume { volume: 0.8 },
            Request::VolumeStep { delta: 0.05 },
            Request::SetRepeat {
                mode: RepeatMode::SingleTrack,
            },
            Request::QueuePush {
                path: PathBuf::from("/music/b.flac"),
            },
            Request::QueueRemove {
                path: PathBuf::from("/music/b.flac"),
            },
            Request::SetActiveList {
                songs: vec![
                    PathBuf::from("/music/a.flac"),
                    PathBuf::from("/music/b.flac"),
                ],
            },
            Request::LibraryArtists,
            Request::LibraryAlbums {
                artist: "一首歌的歌手".into(),
            },
            Request::LibraryTracks {
                artist: "Artist".into(),
                album: "Album".into(),
            },
            Request::SearchLibrary {
                query: "night".into(),
            },
            Request::AddLibraryPath {
                path: PathBuf::from("/music"),
            },
            Request::IndexLibrary,
            Request::CancelScan,
            Request::RemoveLibraryPath {
                root: PathBuf::from("/music"),
            },
            Request::SubscribeVisualizer { on: true },
            Request::SetFftParams {
                num_bars: 32,
                smoothing: 0.35,
            },
            Request::SetLyricsOffset { ms: -500 },
            Request::GetState,
            Request::Shutdown,
        ] {
            round_trip_request(request);
        }
    }

    #[test]
    fn every_event_round_trips() {
        for event in [
            Event::Welcome {
                proto: PROTOCOL_VERSION,
                version: "0.1.0".into(),
                pid: 4242,
            },
            Event::Snapshot(Box::default()),
            Event::Snapshot(Box::new(StateSnapshot {
                status: PlaybackState::Failed("no such file".into()),
                path: Some(PathBuf::from("/music/一首歌.flac")),
                title: "Title".into(),
                artist: "Artist".into(),
                album: "Album".into(),
                genre: "Genre".into(),
                year: "2026".into(),
                codec: "flac".into(),
                duration_secs: 245.5,
                position_secs: 12.25,
                volume: 0.8,
                repeat: RepeatMode::Shuffle,
                playing_index: Some(3),
                queue_rev: 7,
                lyrics_offset_ms: -500,
            })),
            Event::Queue {
                rev: 7,
                tracks: vec![QueueTrack {
                    path: PathBuf::from("/music/a.flac"),
                    title: "A".into(),
                    artist: "Artist".into(),
                    duration_secs: 12.5,
                }],
            },
            Event::Visualizer {
                bars: vec![0.0, 0.25, 1.0],
            },
            Event::LibraryArtists {
                artists: vec!["Artist".into(), "另一个".into()],
            },
            Event::LibraryAlbums {
                artist: "Artist".into(),
                albums: vec!["Album".into()],
            },
            Event::LibraryTracks {
                artist: "Artist".into(),
                album: "Album".into(),
                tracks: vec![TrackLine {
                    path: PathBuf::from("/music/a.flac"),
                    title: "A".into(),
                    artist: "Artist".into(),
                }],
            },
            Event::SearchResults {
                query: "night".into(),
                tracks: vec![TrackLine {
                    path: PathBuf::from("/music/一首歌.flac"),
                    title: "A".into(),
                    // A row whose tags carry no artist: empty, not a stand-in.
                    artist: String::new(),
                }],
            },
            Event::LibraryPaths {
                paths: vec![PathBuf::from("/music"), PathBuf::from("/more/一首歌.flac")],
            },
            Event::ScanProgress {
                scanned: 12,
                changed: 3,
            },
            Event::ScanFinished(ScanReport {
                scanned: 12,
                changed: 3,
                removed: 1,
                failed: 0,
                cancelled: false,
                complete: true,
                active: 0,
            }),
            Event::Notice {
                level: NoticeLevel::Warn,
                message: "daemon busy".into(),
            },
            Event::Bye,
        ] {
            round_trip_event(event);
        }
    }

    /// Floats must survive the trip exactly. A position or a volume that comes
    /// back a hair different is not a display problem — it is the client and
    /// the daemon disagreeing about what was asked for.
    #[test]
    fn floats_survive_the_wire_exactly() {
        for value in [0.0f32, 0.1, 0.8, 0.35, 0.3, 1.0] {
            let request = Request::SetVolume { volume: value };
            let json = serde_json::to_string(&request).expect("serialize");
            let back: Request = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, request, "{value} did not survive: {json}");
        }
        for value in [0.0f64, 1.0 / 3.0, 245.5, -5.0] {
            let request = Request::SeekRelative { secs: value };
            let json = serde_json::to_string(&request).expect("serialize");
            let back: Request = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, request, "{value} did not survive: {json}");
        }
    }

    /// The tag is the whole contract; a rename would break every deployed
    /// client silently, so pin a few spellings.
    #[test]
    fn variants_are_tagged_in_snake_case() {
        let json = serde_json::to_string(&Request::SetRepeat {
            mode: RepeatMode::Shuffle,
        })
        .expect("serialize");
        assert_eq!(json, r#"{"t":"set_repeat","mode":"Shuffle"}"#);

        let json = serde_json::to_string(&Request::Toggle).expect("serialize");
        assert_eq!(json, r#"{"t":"toggle"}"#);
    }

    /// A client one version ahead may send a field this build has never heard
    /// of; that must be ignored, not fatal, or every mixed install breaks.
    #[test]
    fn unknown_fields_are_ignored() {
        let back: Request =
            serde_json::from_str(r#"{"t":"toggle","from_the_future":42}"#).expect("deserialize");
        assert_eq!(back, Request::Toggle);
    }

    /// An unknown *tag* is an error, not a guess: silently dropping a command
    /// the user typed is worse than reporting it.
    #[test]
    fn unknown_tags_are_an_error() {
        assert!(serde_json::from_str::<Request>(r#"{"t":"teleport"}"#).is_err());
    }
}
