//! The player on the desktop bus.
//!
//! MPRIS2 is how a desktop finds a media player at all: the media widget in
//! the panel, the play/pause key on the keyboard and `playerctl` all look for
//! a well-known name on the session bus and talk to whatever answers to it.
//! The daemon is the only process that knows what is playing, so the daemon is
//! the one that answers — which is the whole reason the player was split out
//! of the TUI in the first place.
//!
//! Three rules shape this module.
//!
//! **MPRIS commands are [`Request`]s.** A `PlayPause`, a `Next` or a
//! `SetVolume` arriving over D-Bus goes down the same channel a socket
//! client's commands arrive on and is applied by the same `Player::execute`.
//! There is no second implementation of what *next* means to keep in step.
//!
//! **Nothing here can block the player.** The bus is another process: it can
//! be slow, wedged or gone. The daemon's loop hands an update over with
//! `try_send`-style channels and moves on, and the D-Bus-facing work happens
//! on a task of its own.
//!
//! **A missing bus is not an error.** A machine with no desktop — a bare ssh
//! login, a service manager, a container — has no media widget to appear in,
//! and the player's job is to play music either way. Failing to join the bus
//! is logged and shrugged off.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mpris_server::{
    LoopStatus, Metadata, PlaybackStatus, PlayerInterface, Property, RootInterface, Server, Signal,
    Time, TrackId, Volume,
};
use tokio::sync::mpsc;

use crate::audio::engine::PlaybackState;
use crate::ipc::proto::{RepeatMode, Request, StateSnapshot};

use super::cover;

/// The name the desktop looks for. `playerctl -p tmper`, and the media widget
/// lists it as "tmper".
const BUS_NAME: &str = "tmper";

/// How long the bus gets to answer before the player gives up on it.
///
/// A wedged `dbus-daemon` is a real thing, and it must not be able to keep the
/// player from starting: the music is the point, the widget is the bonus.
const BUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// What the daemon reports through, and the channel it takes orders from.
pub struct Handle {
    updates: mpsc::UnboundedSender<Command>,
    state: Arc<Mutex<State>>,
}

/// A change the bus has to be told about.
enum Command {
    /// Properties whose values are now different.
    Changed(Vec<Property>),
    /// The needle moved somewhere it did not walk to.
    Seeked(Time),
}

/// Join the session bus, if there is one.
///
/// Returns the handle the daemon reports through and the receiver it takes
/// commands from, or `None` when the bus could not be joined — which is not a
/// failure, and must not stop the daemon.
pub async fn start() -> Option<(Handle, mpsc::UnboundedReceiver<Request>)> {
    let (commands_tx, commands_rx) = mpsc::unbounded_channel::<Request>();
    let (updates_tx, updates_rx) = mpsc::unbounded_channel::<Command>();
    let state = Arc::new(Mutex::new(State::new()));

    let player = Player {
        commands: commands_tx,
        state: Arc::clone(&state),
    };

    let server = match tokio::time::timeout(BUS_TIMEOUT, Server::new(BUS_NAME, player)).await {
        Ok(Ok(server)) => server,
        Ok(Err(error)) => {
            tracing::warn!("MPRIS unavailable ({error}); the desktop will not see the player");
            return None;
        }
        Err(_) => {
            tracing::warn!("MPRIS unavailable (the session bus did not answer)");
            return None;
        }
    };

    tracing::info!("MPRIS: answering as org.mpris.MediaPlayer2.{BUS_NAME}");
    tokio::spawn(announce(server, updates_rx));

    Some((
        Handle {
            updates: updates_tx,
            state,
        },
        commands_rx,
    ))
}

impl Handle {
    /// Fold a snapshot in and tell the desktop whatever changed.
    ///
    /// The daemon pushes a snapshot every tick, and consecutive ones differ in
    /// exactly one field most of the time: the position. Position is not an
    /// MPRIS property — a client asks for it with `Position` — so a report
    /// that announced the whole state would be thirty `PropertiesChanged` a
    /// second and a media widget repainting forever.
    pub fn apply(&self, snapshot: &StateSnapshot) {
        let changed = match self.state.lock() {
            Ok(mut state) => state.apply(snapshot),
            // A poisoned lock means a D-Bus thread panicked mid-read. The
            // player is fine; the widget can go stale.
            Err(_) => return,
        };
        if !changed.is_empty() {
            let _ = self.updates.send(Command::Changed(changed));
        }
    }

    /// Say where a seek landed.
    ///
    /// Separate from [`Handle::apply`] because it is not a change of state: a
    /// client that was not watching cannot tell a jump from a fast-forward
    /// otherwise, and the spec's whole purpose for the signal is to tell it.
    pub fn seeked(&self, position_secs: f64) {
        let _ = self.updates.send(Command::Seeked(to_time(position_secs)));
    }
}

/// Own the server and forward what the daemon reports.
///
/// The server has to be owned by something that lives as long as the
/// registration does: dropping it releases the bus name, and the widget
/// disappears.
async fn announce(server: Server<Player>, mut updates: mpsc::UnboundedReceiver<Command>) {
    while let Some(command) = updates.recv().await {
        let result = match command {
            Command::Changed(properties) => server.properties_changed(properties).await,
            Command::Seeked(position) => server.emit(Signal::Seeked { position }).await,
        };
        if let Err(error) = result {
            // The bus is gone. The registration goes with it, so there is
            // nothing left to report to and no reason to keep the channel
            // alive: ending the task drops the receiver, and the daemon's
            // `send`s start failing instead of piling up.
            tracing::warn!("MPRIS: {error}; the desktop will stop seeing the player");
            return;
        }
    }
}

// ── The state the bus sees ──

/// What the desktop is told, as plain data.
///
/// Deliberately not the snapshot: MPRIS has its own vocabulary — a track id
/// has to be a D-Bus object path, a length is in microseconds, and one repeat
/// mode in tmper is *two* flags here — and doing that translation once, on the
/// way in, is what keeps the getters below free of it.
#[derive(Debug, Clone, PartialEq)]
struct State {
    status: PlaybackStatus,
    loop_status: LoopStatus,
    shuffle: bool,
    volume: f64,
    track: Option<Track>,
    can_seek: bool,
    /// Not a property: MPRIS clients ask for `Position` when they want it, and
    /// announcing it every tick is exactly the storm the diff exists to avoid.
    /// It is kept anyway because `Position` still has to be answered, and
    /// because a snapshot is the only moment the daemon says where the needle
    /// is.
    position_secs: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct Track {
    path: PathBuf,
    title: String,
    artist: String,
    album: String,
    length_secs: f64,
    /// A `file://` URL of the cached cover, when the tags carried one.
    art_url: Option<String>,
}

impl State {
    fn new() -> Self {
        Self {
            status: PlaybackStatus::Stopped,
            loop_status: LoopStatus::None,
            shuffle: false,
            volume: 1.0,
            track: None,
            can_seek: false,
            position_secs: 0.0,
        }
    }

    /// Fold a snapshot in, and answer with the properties whose values moved.
    ///
    /// The empty vector is the common case and the point of the exercise.
    fn apply(&mut self, snapshot: &StateSnapshot) -> Vec<Property> {
        let mut changed = Vec::new();

        let status = status_of(&snapshot.status);
        if status != self.status {
            self.status = status;
            changed.push(Property::PlaybackStatus(status));
        }

        // `RepeatMode::Shuffle` is both of MPRIS's repeat flags at once: tmper
        // shuffles by picking forever, which is a playlist loop that is not in
        // order. The two properties are therefore two views of one state, and
        // both move together.
        let loop_status = loop_status_of(snapshot.repeat);
        if loop_status != self.loop_status {
            self.loop_status = loop_status;
            changed.push(Property::LoopStatus(loop_status));
        }
        let shuffle = snapshot.repeat == RepeatMode::Shuffle;
        if shuffle != self.shuffle {
            self.shuffle = shuffle;
            changed.push(Property::Shuffle(shuffle));
        }

        let volume = to_volume(snapshot.volume);
        if volume != self.volume {
            self.volume = volume;
            changed.push(Property::Volume(volume));
        }

        let track = snapshot.path.as_ref().map(|path| Track::of(path, snapshot));
        if track != self.track {
            let announced = track.clone();
            self.track = track;
            changed.push(Property::Metadata(metadata_of(announced.as_ref())));
        }

        // A stopped player has nowhere to seek to, and a client that sees
        // `CanSeek` is one that draws a scrub bar.
        let can_seek = snapshot.duration_secs > 0.0;
        if can_seek != self.can_seek {
            self.can_seek = can_seek;
            changed.push(Property::CanSeek(can_seek));
        }

        // Position is not announced, but it is remembered — see the field.
        self.position_secs = snapshot.position_secs;

        changed
    }
}

impl Track {
    /// The track an MPRIS client is shown.
    ///
    /// Three of the seven fields come from the snapshot's *display* values,
    /// which is not the same thing as the tags: a file with no title tag is
    /// shown as its file name, and the widget should say what the player says.
    fn of(path: &Path, snapshot: &StateSnapshot) -> Self {
        Self {
            path: path.to_path_buf(),
            title: snapshot.title.clone(),
            artist: snapshot.artist.clone(),
            album: snapshot.album.clone(),
            length_secs: snapshot.duration_secs,
            art_url: snapshot.cover_path.as_deref().map(file_url),
        }
    }
}

/// The `PlaybackStatus` tmper's state is closest to.
///
/// Loading and seeking are the player's business, not the desktop's: a widget
/// told "loading" when a track was picked would flash a stopped transport at
/// the user. There is no separate `Loading` in MPRIS — the state either makes
/// sound, holds a place, or is not there.
fn status_of(state: &PlaybackState) -> PlaybackStatus {
    match state {
        PlaybackState::Paused => PlaybackStatus::Paused,
        PlaybackState::Loading | PlaybackState::Playing | PlaybackState::Seeking => {
            PlaybackStatus::Playing
        }
        PlaybackState::Stopped | PlaybackState::Finished | PlaybackState::Failed(_) => {
            PlaybackStatus::Stopped
        }
    }
}

fn metadata_of(track: Option<&Track>) -> Metadata {
    let Some(track) = track else {
        return Metadata::new();
    };
    let mut builder = Metadata::builder()
        .trackid(id_of(&track.path))
        .title(track.title.clone())
        .artist([track.artist.clone()])
        .length(to_time(track.length_secs));
    if !track.album.is_empty() {
        builder = builder.album(track.album.clone());
    }
    if let Some(url) = &track.art_url {
        builder = builder.art_url(url.clone());
    }
    builder.build()
}

/// The MPRIS identity of a track: a D-Bus object path, derived from its file.
///
/// Derived rather than counted, so the same track is the same id across
/// restarts — a client that remembers "the track that was playing" must not
/// find the id pointing at something else tomorrow. The `/org/mpris` namespace
/// is reserved by the spec for paths it defines, and `NoTrack` is the one id
/// that already has a meaning there, so ours lives under `/tmper`.
fn id_of(path: &Path) -> TrackId {
    TrackId::try_from(format!("/tmper/track/{:016x}", cover::fingerprint(path)))
        .expect("a hex string is a valid object path")
}

/// A `file://` URL for a path on this machine.
///
/// Percent-encoded by hand, and it has to be: the paths come from the user's
/// own filesystem, and a `file://` URL with a raw space, `#` or `%` in it is a
/// URL the desktop resolves to a *different* path than the one we meant — or
/// refuses.
fn file_url(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    let mut url = String::from("file://");
    for byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                url.push(char::from(*byte));
            }
            _ => url.push_str(&format!("%{byte:02X}")),
        }
    }
    url
}

/// tmper's gain in the double MPRIS counts in.
///
/// The cast is not the whole story: tmper's volume is an `f32`, so widening it
/// plain would publish `0.800000011920929` — the same 80% with eleven digits of
/// noise on it, and a value that no longer round-trips, since a client that set
/// 0.8 and read it back would find it had moved. Three decimals is finer than
/// any volume slider and leaves the round trip exact.
fn to_volume(volume: f32) -> f64 {
    (f64::from(volume) * 1000.0).round() / 1000.0
}

/// Seconds to the microseconds MPRIS counts in.
fn to_time(secs: f64) -> Time {
    Time::from_micros((secs.max(0.0) * 1e6).round() as i64)
}

fn from_time(time: Time) -> f64 {
    time.as_micros() as f64 / 1e6
}

// ── The interfaces ──

/// The `org.mpris.MediaPlayer2` object.
///
/// It holds a channel and a shared state rather than the player itself: the
/// player is `!Send` (there is a sound card inside it), and D-Bus methods are
/// called from whichever thread zbus likes. Everything a method does is either
/// a message into the daemon's loop or a read of the state the daemon just
/// published.
struct Player {
    commands: mpsc::UnboundedSender<Request>,
    state: Arc<Mutex<State>>,
}

impl Player {
    /// Send a command to the daemon.
    ///
    /// A closed channel means the daemon is on its way out, which is not
    /// something a media key can do anything about — and answering the bus
    /// with an error would only make the widget flash.
    fn send(&self, request: Request) {
        let _ = self.commands.send(request);
    }

    /// Move a property in the mirror the getters read.
    ///
    /// Needed because zbus answers a property `Set` with a `PropertiesChanged`
    /// **of its own**, computed from the getter the moment the setter returns
    /// — and our setter returns before the daemon has applied anything. Left
    /// alone, every volume a widget set would be announced as the volume it
    /// was replacing, a millisecond before the real change arrived: the slider
    /// would snap back and then forward again on every drag.
    ///
    /// So a setter for a value we intend to honour records it here first. The
    /// value has to be spelled the way the snapshot will spell it (`[to_volume]`
    /// and friends), or the snapshot that follows would find a difference to
    /// announce and we would be back to two signals per `Set`.
    ///
    /// Only for values the player actually honours: `Rate` is accepted and
    /// ignored, so recording it would have the getter report a speed that is
    /// not being played.
    fn record(&self, edit: impl FnOnce(&mut State)) {
        if let Ok(mut state) = self.state.lock() {
            edit(&mut state);
        }
    }

    fn current(&self) -> Option<Track> {
        self.state.lock().ok()?.track.clone()
    }
}

impl RootInterface for Player {
    /// Nothing to raise: tmper is a TUI, and it has no window of its own to
    /// bring forward. [`RootInterface::can_raise`] says so, so a client that
    /// respects the property never calls this.
    async fn raise(&self) -> mpris_server::zbus::fdo::Result<()> {
        Ok(())
    }

    /// `CanQuit` is true, so this is how the desktop's "quit" button stops the
    /// player — the same [`Request::Shutdown`] as `tmper quit` and `:quit!`.
    async fn quit(&self) -> mpris_server::zbus::fdo::Result<()> {
        self.send(Request::Shutdown);
        Ok(())
    }

    async fn can_quit(&self) -> mpris_server::zbus::fdo::Result<bool> {
        Ok(true)
    }

    async fn fullscreen(&self) -> mpris_server::zbus::fdo::Result<bool> {
        Ok(false)
    }

    async fn set_fullscreen(&self, _fullscreen: bool) -> mpris_server::zbus::Result<()> {
        Ok(())
    }

    async fn can_set_fullscreen(&self) -> mpris_server::zbus::fdo::Result<bool> {
        Ok(false)
    }

    async fn can_raise(&self) -> mpris_server::zbus::fdo::Result<bool> {
        Ok(false)
    }

    async fn has_track_list(&self) -> mpris_server::zbus::fdo::Result<bool> {
        Ok(false)
    }

    async fn identity(&self) -> mpris_server::zbus::fdo::Result<String> {
        Ok("tmper".into())
    }

    async fn desktop_entry(&self) -> mpris_server::zbus::fdo::Result<String> {
        Ok("tmper".into())
    }

    async fn supported_uri_schemes(&self) -> mpris_server::zbus::fdo::Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn supported_mime_types(&self) -> mpris_server::zbus::fdo::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

impl PlayerInterface for Player {
    async fn next(&self) -> mpris_server::zbus::fdo::Result<()> {
        self.send(Request::Next);
        Ok(())
    }

    async fn previous(&self) -> mpris_server::zbus::fdo::Result<()> {
        self.send(Request::Prev);
        Ok(())
    }

    async fn pause(&self) -> mpris_server::zbus::fdo::Result<()> {
        self.send(Request::Pause);
        Ok(())
    }

    /// One key, one command: the same `Toggle` the space bar sends, so the
    /// media key and the keyboard key cannot drift apart.
    async fn play_pause(&self) -> mpris_server::zbus::fdo::Result<()> {
        self.send(Request::Toggle);
        Ok(())
    }

    async fn stop(&self) -> mpris_server::zbus::fdo::Result<()> {
        self.send(Request::Stop);
        Ok(())
    }

    /// Not [`Request::Toggle`]: MPRIS separates play from pause, and a client
    /// that says "play" to a player that is already making sound means carry
    /// on, not stop.
    async fn play(&self) -> mpris_server::zbus::fdo::Result<()> {
        self.send(Request::Resume);
        Ok(())
    }

    async fn seek(&self, offset: Time) -> mpris_server::zbus::fdo::Result<()> {
        self.send(Request::SeekRelative {
            secs: from_time(offset),
        });
        Ok(())
    }

    /// Jump to a position, if it is a position in the track that is loaded.
    ///
    /// The track id is checked rather than assumed: the spec says a client
    /// naming a track other than the current one gets nothing done, and the id
    /// is the only thing that tells us which track the client meant.
    async fn set_position(
        &self,
        track_id: TrackId,
        position: Time,
    ) -> mpris_server::zbus::fdo::Result<()> {
        let Some(track) = self.current() else {
            return Ok(());
        };
        if track_id != id_of(&track.path) {
            return Ok(());
        }
        let now = self
            .state
            .lock()
            .map(|state| state.position_secs)
            .unwrap_or(0.0);
        self.send(Request::SeekRelative {
            secs: from_time(position) - now,
        });
        Ok(())
    }

    /// Not supported: the daemon plays files it was pointed at, and there is
    /// no URL handler behind it.
    async fn open_uri(&self, _uri: String) -> mpris_server::zbus::fdo::Result<()> {
        Err(mpris_server::zbus::fdo::Error::NotSupported(
            "tmper opens local files, not URIs".into(),
        ))
    }

    async fn playback_status(&self) -> mpris_server::zbus::fdo::Result<PlaybackStatus> {
        Ok(self
            .state
            .lock()
            .map(|s| s.status)
            .unwrap_or(PlaybackStatus::Stopped))
    }

    async fn loop_status(&self) -> mpris_server::zbus::fdo::Result<LoopStatus> {
        Ok(self
            .state
            .lock()
            .map(|s| s.loop_status)
            .unwrap_or(LoopStatus::None))
    }

    async fn set_loop_status(&self, loop_status: LoopStatus) -> mpris_server::zbus::Result<()> {
        // tmper has one three-way mode where MPRIS has two flags, so this is
        // the whole mapping in both directions: see `State::apply`.
        //
        // Only the flag the client named is recorded. Its twin is left alone
        // deliberately: when this `Set` really does move it — `Track` on a
        // shuffled player is also "not shuffled" — the snapshot that follows
        // announces it, and that announcement is news the desktop needs rather
        // than a duplicate. What the mirror is for is the *other* case: a flag
        // zbus would otherwise announce from a stale getter, at the old value,
        // a millisecond before the real change arrived.
        self.record(|state| state.loop_status = loop_status);
        self.send(Request::SetRepeat {
            mode: repeat_of(loop_status),
        });
        Ok(())
    }

    /// tmper plays at 1×; there is no other speed to report or to accept.
    async fn rate(&self) -> mpris_server::zbus::fdo::Result<f64> {
        Ok(1.0)
    }

    /// Accepted and ignored, which is why nothing is recorded here: the
    /// announcement zbus makes for this `Set` reads `rate` back as 1×, and
    /// that is the truth — there is no faster player to tell the client about.
    async fn set_rate(&self, _rate: f64) -> mpris_server::zbus::Result<()> {
        Ok(())
    }

    async fn shuffle(&self) -> mpris_server::zbus::fdo::Result<bool> {
        Ok(self.state.lock().map(|s| s.shuffle).unwrap_or(false))
    }

    async fn set_shuffle(&self, shuffle: bool) -> mpris_server::zbus::Result<()> {
        // `false` does not mean "and stop looping". The two flags are separate
        // axes to a client, so clearing a shuffle the player never had — which
        // is what a widget re-asserting its own state does on every refresh —
        // must not take a track loop with it. Which of the two unshuffled modes
        // was meant is only answerable by reading the loop axis back.
        let mode = if shuffle {
            RepeatMode::Shuffle
        } else if matches!(self.loop_status().await, Ok(LoopStatus::Track)) {
            RepeatMode::SingleTrack
        } else {
            RepeatMode::Sequential
        };
        // As in `set_loop_status`: the flag the client named, and nothing else.
        // `set_shuffle(true)` really does move the loop to `Playlist`, and the
        // snapshot says so; recording that here would swallow it.
        self.record(|state| state.shuffle = shuffle);
        self.send(Request::SetRepeat { mode });
        Ok(())
    }

    async fn metadata(&self) -> mpris_server::zbus::fdo::Result<Metadata> {
        Ok(metadata_of(self.current().as_ref()))
    }

    async fn volume(&self) -> mpris_server::zbus::fdo::Result<Volume> {
        Ok(self.state.lock().map(|s| s.volume).unwrap_or(1.0))
    }

    async fn set_volume(&self, volume: Volume) -> mpris_server::zbus::Result<()> {
        // tmper's own volume is already 0.0–1.0, which is what MPRIS counts in
        // — the one scale in the spec that needs no conversion. The daemon
        // clamps anyway.
        let volume = volume.clamp(0.0, 1.0) as f32;
        self.record(|state| state.volume = to_volume(volume));
        self.send(Request::SetVolume { volume });
        Ok(())
    }

    /// Where the needle is, as of the last snapshot.
    ///
    /// Up to one tick stale — the daemon is the one with the clock, and the
    /// position it publishes thirty times a second is a better answer than
    /// anything this side of the socket could derive.
    async fn position(&self) -> mpris_server::zbus::fdo::Result<Time> {
        let secs = self.state.lock().map(|s| s.position_secs).unwrap_or(0.0);
        Ok(to_time(secs))
    }

    async fn minimum_rate(&self) -> mpris_server::zbus::fdo::Result<f64> {
        Ok(1.0)
    }

    async fn maximum_rate(&self) -> mpris_server::zbus::fdo::Result<f64> {
        Ok(1.0)
    }

    /// Unknown, and therefore true: whether there is a next track depends on
    /// the queue and the playlist the daemon is walking, and this side of the
    /// socket can only guess. The spec's instruction for exactly this case is
    /// to report `true` rather than to disable the button on a guess.
    async fn can_go_next(&self) -> mpris_server::zbus::fdo::Result<bool> {
        Ok(true)
    }

    /// Same rule as [`PlayerInterface::can_go_next`].
    async fn can_go_previous(&self) -> mpris_server::zbus::fdo::Result<bool> {
        Ok(true)
    }

    async fn can_play(&self) -> mpris_server::zbus::fdo::Result<bool> {
        Ok(true)
    }

    async fn can_pause(&self) -> mpris_server::zbus::fdo::Result<bool> {
        Ok(true)
    }

    async fn can_seek(&self) -> mpris_server::zbus::fdo::Result<bool> {
        Ok(self.state.lock().map(|s| s.can_seek).unwrap_or(false))
    }

    async fn can_control(&self) -> mpris_server::zbus::fdo::Result<bool> {
        Ok(true)
    }
}

/// A `LoopStatus` back to the one mode it names.
///
/// `Playlist` is `Shuffle`: see `State::apply` for why the two MPRIS flags are
/// one mode here.
fn repeat_of(loop_status: LoopStatus) -> RepeatMode {
    match loop_status {
        LoopStatus::None => RepeatMode::Sequential,
        LoopStatus::Track => RepeatMode::SingleTrack,
        LoopStatus::Playlist => RepeatMode::Shuffle,
    }
}

/// The other direction: the loop half of a mode's two flags. The shuffle half
/// is `repeat == RepeatMode::Shuffle`, spelled that way wherever a mode is
/// turned into flags — here in the `State` mirror, and in the setters below.
fn loop_status_of(repeat: RepeatMode) -> LoopStatus {
    match repeat {
        RepeatMode::Sequential => LoopStatus::None,
        RepeatMode::SingleTrack => LoopStatus::Track,
        RepeatMode::Shuffle => LoopStatus::Playlist,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> StateSnapshot {
        StateSnapshot {
            status: PlaybackState::Playing,
            path: Some(PathBuf::from("/music/a.flac")),
            title: "A Song".into(),
            artist: "An Artist".into(),
            album: "An Album".into(),
            duration_secs: 245.5,
            position_secs: 12.25,
            volume: 0.8,
            ..StateSnapshot::default()
        }
    }

    /// A player with nothing attached to a bus, which is the whole point: the
    /// interfaces are a translation layer, and the translation is testable
    /// without a session bus, a desktop, or an audio device.
    fn player() -> (Player, mpsc::UnboundedReceiver<Request>, Arc<Mutex<State>>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(State::new()));
        (
            Player {
                commands: tx,
                state: Arc::clone(&state),
            },
            rx,
            state,
        )
    }

    async fn sent(rx: &mut mpsc::UnboundedReceiver<Request>) -> Request {
        rx.try_recv().expect("a command reached the daemon")
    }

    /// The first snapshot a daemon ever routes announces everything the
    /// desktop needs to draw a widget, because before it there was nothing.
    #[test]
    fn the_first_snapshot_announces_the_whole_state() {
        let mut state = State::new();
        let changed = state.apply(&snapshot());

        assert!(changed.contains(&Property::PlaybackStatus(PlaybackStatus::Playing)));
        assert!(changed.contains(&Property::Volume(0.8)));
        assert!(changed.contains(&Property::CanSeek(true)));
        assert!(changed.iter().any(|p| matches!(p, Property::Metadata(_))));
    }

    /// The reason the diff exists at all: the snapshot arrives thirty times a
    /// second and the position is the only thing that usually moved.
    #[test]
    fn a_position_that_moved_is_not_a_change() {
        let mut state = State::new();
        state.apply(&snapshot());

        let mut later = snapshot();
        later.position_secs = 13.9;
        assert!(
            state.apply(&later).is_empty(),
            "the position is not a property, and announcing it would be a \
             PropertiesChanged storm"
        );
        assert_eq!(state.position_secs, 13.9, "but it is still remembered");
    }

    #[test]
    fn only_the_properties_that_moved_are_announced() {
        let mut state = State::new();
        state.apply(&snapshot());

        let mut later = snapshot();
        later.volume = 0.4;
        later.position_secs = 30.0;
        assert_eq!(state.apply(&later), vec![Property::Volume(0.4)]);
    }

    /// Every state that is making sound reads as Playing: a widget that
    /// flickered to "stopped" while a track was loading would be lying.
    #[test]
    fn loading_and_seeking_are_playing_to_the_desktop() {
        for loading in [
            PlaybackState::Loading,
            PlaybackState::Playing,
            PlaybackState::Seeking,
        ] {
            assert_eq!(status_of(&loading), PlaybackStatus::Playing);
        }
        assert_eq!(status_of(&PlaybackState::Paused), PlaybackStatus::Paused);
        for stopped in [
            PlaybackState::Stopped,
            PlaybackState::Finished,
            PlaybackState::Failed("no decoder".into()),
        ] {
            assert_eq!(status_of(&stopped), PlaybackStatus::Stopped);
        }
    }

    /// One mode here, two flags there — and both flags have to move together,
    /// or a widget would show "loop the playlist" and "not shuffled" at once.
    #[test]
    fn the_repeat_mode_moves_both_mpris_flags() {
        let mut state = State::new();
        state.apply(&snapshot());

        let mut shuffled = snapshot();
        shuffled.repeat = RepeatMode::Shuffle;
        let changed = state.apply(&shuffled);
        assert!(changed.contains(&Property::LoopStatus(LoopStatus::Playlist)));
        assert!(changed.contains(&Property::Shuffle(true)));

        let mut single = snapshot();
        single.repeat = RepeatMode::SingleTrack;
        let changed = state.apply(&single);
        assert!(changed.contains(&Property::LoopStatus(LoopStatus::Track)));
        assert!(changed.contains(&Property::Shuffle(false)));

        // And back, so the mapping is not one-way.
        assert_eq!(repeat_of(LoopStatus::None), RepeatMode::Sequential);
        assert_eq!(repeat_of(LoopStatus::Track), RepeatMode::SingleTrack);
        assert_eq!(repeat_of(LoopStatus::Playlist), RepeatMode::Shuffle);
    }

    /// A track change is one `Metadata`, and the values in it come from the
    /// snapshot's display fields — what the player shows is what the widget
    /// shows, file name and all.
    #[test]
    fn a_track_change_announces_its_metadata() {
        let mut state = State::new();
        state.apply(&snapshot());

        let mut next = snapshot();
        next.path = Some(PathBuf::from("/music/b.flac"));
        next.title = "B Song".into();
        let changed = state.apply(&next);

        let Some(Property::Metadata(metadata)) = changed
            .iter()
            .find(|property| matches!(property, Property::Metadata(_)))
        else {
            panic!("a new track has to announce its metadata: {changed:?}");
        };
        assert_eq!(metadata.title(), Some("B Song"));
        assert_eq!(metadata.artist(), Some(vec!["An Artist".to_string()]));
        assert_eq!(metadata.album(), Some("An Album"));
        assert_eq!(metadata.length(), Some(to_time(245.5)));
        assert_eq!(metadata.trackid(), Some(id_of(Path::new("/music/b.flac"))));
    }

    /// Nothing on the deck is not an empty track: a widget told about a track
    /// with no title draws a blank row instead of its "nothing playing" face.
    #[test]
    fn an_empty_deck_announces_empty_metadata() {
        let mut state = State::new();
        state.apply(&snapshot());

        let mut empty = snapshot();
        empty.path = None;
        let changed = state.apply(&empty);

        let Some(Property::Metadata(metadata)) = changed
            .iter()
            .find(|property| matches!(property, Property::Metadata(_)))
        else {
            panic!("the deck emptying is a change: {changed:?}");
        };
        assert_eq!(metadata.title(), None);
        assert_eq!(metadata.trackid(), None);
        assert_eq!(metadata.length(), None);
    }

    /// A scrub bar is only drawn for something that has a length to scrub
    /// through — and the flag has to move both ways, since a track can be
    /// replaced by a stream that does not end.
    #[test]
    fn a_track_with_no_duration_cannot_be_seeked() {
        let mut state = State::new();
        // From a track, not from bare `State::new()`: an empty deck is already
        // unseekable, so only a *change* of the flag proves anything.
        assert!(state.apply(&snapshot()).contains(&Property::CanSeek(true)));

        let mut endless = snapshot();
        endless.duration_secs = 0.0;
        assert!(state.apply(&endless).contains(&Property::CanSeek(false)));
    }

    // ── The interfaces ──

    /// Every transport method is the same `Request` the key of the same name
    /// sends, which is what keeps the media key and the keyboard in step.
    #[tokio::test]
    async fn the_transport_methods_are_the_requests_the_keys_send() {
        let (player, mut rx, _state) = player();

        player.play_pause().await.expect("call");
        assert_eq!(sent(&mut rx).await, Request::Toggle);
        player.next().await.expect("call");
        assert_eq!(sent(&mut rx).await, Request::Next);
        player.previous().await.expect("call");
        assert_eq!(sent(&mut rx).await, Request::Prev);
        player.pause().await.expect("call");
        assert_eq!(sent(&mut rx).await, Request::Pause);
        player.stop().await.expect("call");
        assert_eq!(sent(&mut rx).await, Request::Stop);
        player.play().await.expect("call");
        assert_eq!(sent(&mut rx).await, Request::Resume);
        player.quit().await.expect("call");
        assert_eq!(sent(&mut rx).await, Request::Shutdown);
    }

    #[tokio::test]
    async fn seeking_and_volume_cross_the_wire_in_seconds_and_fractions() {
        let (player, mut rx, _state) = player();

        player.seek(Time::from_secs(-5)).await.expect("call");
        assert_eq!(sent(&mut rx).await, Request::SeekRelative { secs: -5.0 });

        player.seek(Time::from_millis(1500)).await.expect("call");
        assert_eq!(sent(&mut rx).await, Request::SeekRelative { secs: 1.5 });

        player.set_volume(0.35).await.expect("call");
        assert_eq!(sent(&mut rx).await, Request::SetVolume { volume: 0.35 });
    }

    /// MPRIS volume is a double in the same 0.0–1.0 tmper already uses, but a
    /// client that sends something else must not be able to hand the engine a
    /// gain it would then have to refuse.
    #[tokio::test]
    async fn a_volume_outside_the_scale_is_clamped() {
        let (player, mut rx, _state) = player();

        player.set_volume(4.0).await.expect("call");
        assert_eq!(sent(&mut rx).await, Request::SetVolume { volume: 1.0 });
        player.set_volume(-1.0).await.expect("call");
        assert_eq!(sent(&mut rx).await, Request::SetVolume { volume: 0.0 });
    }

    /// A `SetPosition` is relative on the wire, because that is the only seek
    /// the daemon has: the offset is worked out from the last position it
    /// published.
    #[tokio::test]
    async fn set_position_becomes_a_relative_seek() {
        let (player, mut rx, state) = player();
        state.lock().unwrap().apply(&snapshot());

        player
            .set_position(id_of(Path::new("/music/a.flac")), Time::from_secs(90))
            .await
            .expect("call");

        assert_eq!(
            sent(&mut rx).await,
            Request::SeekRelative { secs: 90.0 - 12.25 }
        );
    }

    /// The spec's rule: naming a track that is not the one loaded gets nothing
    /// done. The id is the only thing that says which track the client meant,
    /// so this is the check that makes a stale widget harmless.
    #[tokio::test]
    async fn set_position_for_a_track_that_is_not_loaded_does_nothing() {
        let (player, mut rx, state) = player();
        state.lock().unwrap().apply(&snapshot());

        player
            .set_position(
                id_of(Path::new("/music/somewhere_else.flac")),
                Time::from_secs(90),
            )
            .await
            .expect("call");

        assert!(rx.try_recv().is_err(), "no command should have been sent");
    }

    #[tokio::test]
    async fn set_position_with_an_empty_deck_does_nothing() {
        let (player, mut rx, _state) = player();
        player
            .set_position(id_of(Path::new("/music/a.flac")), Time::from_secs(90))
            .await
            .expect("call");
        assert!(rx.try_recv().is_err());
    }

    /// The getters answer from the state the daemon published, so a client
    /// reading two properties in a row cannot see two different players.
    #[tokio::test]
    async fn the_getters_read_what_the_daemon_published() {
        let (player, _rx, state) = player();
        state.lock().unwrap().apply(&snapshot());

        assert_eq!(
            player.playback_status().await.expect("call"),
            PlaybackStatus::Playing
        );
        assert_eq!(player.volume().await.expect("call"), 0.8);
        assert_eq!(player.position().await.expect("call"), to_time(12.25));
        assert!(player.can_seek().await.expect("call"));

        let metadata = player.metadata().await.expect("call");
        assert_eq!(metadata.title(), Some("A Song"));
    }

    /// A widget with nothing to show must be told so, not left holding the
    /// last track's title.
    #[tokio::test]
    async fn the_getters_answer_sensibly_with_nothing_loaded() {
        let (player, _rx, _state) = player();

        assert_eq!(
            player.playback_status().await.expect("call"),
            PlaybackStatus::Stopped
        );
        assert_eq!(player.loop_status().await.expect("call"), LoopStatus::None);
        assert!(!player.shuffle().await.expect("call"));
        assert!(!player.can_seek().await.expect("call"));
        assert_eq!(player.metadata().await.expect("call").title(), None);
    }

    /// `Playlist` loop is tmper's shuffle, in both directions.
    ///
    /// Each `Set` is followed by the snapshot the daemon sends to confirm it,
    /// because that is what the next `Set` is read against: the mirror is the
    /// desktop's view, and a `Set` arrives at whatever the desktop currently
    /// believes — which a widget has from the properties it was last told.
    #[tokio::test]
    async fn the_repeat_mode_round_trips_through_the_bus() {
        let (player, mut rx, state) = player();
        let confirmed = |mode| {
            let mut snap = snapshot();
            snap.repeat = mode;
            state.lock().unwrap().apply(&snap);
        };

        player
            .set_loop_status(LoopStatus::Track)
            .await
            .expect("call");
        assert_eq!(
            sent(&mut rx).await,
            Request::SetRepeat {
                mode: RepeatMode::SingleTrack
            }
        );
        confirmed(RepeatMode::SingleTrack);

        player.set_shuffle(true).await.expect("call");
        assert_eq!(
            sent(&mut rx).await,
            Request::SetRepeat {
                mode: RepeatMode::Shuffle
            }
        );
        confirmed(RepeatMode::Shuffle);

        player.set_shuffle(false).await.expect("call");
        assert_eq!(
            sent(&mut rx).await,
            Request::SetRepeat {
                mode: RepeatMode::Sequential
            },
            "the loop was shuffled, not looped on one track"
        );
    }

    /// The loop and the shuffle are separate axes to a client, and the widget
    /// re-asserts its own values on every refresh. Clearing a shuffle the
    /// player never had must not take a track loop with it.
    #[tokio::test]
    async fn clearing_a_shuffle_that_was_never_set_keeps_the_track_loop() {
        let (player, mut rx, state) = player();
        state.lock().unwrap().apply(&StateSnapshot {
            repeat: RepeatMode::SingleTrack,
            ..snapshot()
        });

        player.set_shuffle(false).await.expect("call");

        assert_eq!(
            sent(&mut rx).await,
            Request::SetRepeat {
                mode: RepeatMode::SingleTrack
            },
            "the loop was the only thing set, and it is still set"
        );
    }

    /// The name and the cover are separate concerns: a track with no art still
    /// gets the rest of its metadata.
    #[tokio::test]
    async fn a_track_without_a_cover_has_no_art_url() {
        let (player, _rx, state) = player();
        state.lock().unwrap().apply(&snapshot());
        let metadata = player.metadata().await.expect("call");
        assert_eq!(metadata.art_url(), None);

        let mut with_art = snapshot();
        with_art.cover_path = Some(PathBuf::from("/home/u/.cache/tmper/deadbeef.png"));
        state.lock().unwrap().apply(&with_art);
        let metadata = player.metadata().await.expect("call");
        assert_eq!(
            metadata.art_url().as_deref(),
            Some("file:///home/u/.cache/tmper/deadbeef.png")
        );
    }

    /// A track the daemon has not read yet — a restored session, parked on the
    /// deck — is still a track: the widget shows its title before anything is
    /// decoded.
    #[test]
    fn a_restored_session_is_a_track_with_no_cover_yet() {
        let track = Track::of(Path::new("/music/a.flac"), &snapshot());
        assert_eq!(track.art_url, None);
        assert_eq!(track.title, "A Song");
    }

    // ── The wire format MPRIS sees ──

    /// The track id has to be a D-Bus object path, and it must not live under
    /// `/org/mpris` — the spec reserves that namespace for paths it defines,
    /// `NoTrack` among them.
    #[test]
    fn a_track_id_is_a_path_and_is_spelled_from_the_file() {
        let id = id_of(Path::new("/music/a.flac"));
        let path = id.as_str();
        assert!(path.starts_with("/tmper/track/"), "{path}");
        assert!(!path.starts_with("/org/mpris"), "{path}");
        assert_eq!(id, id_of(Path::new("/music/a.flac")));
        assert_ne!(id, id_of(Path::new("/music/b.flac")));
    }

    /// A path with a space, a `#` or a non-ASCII character in it is the
    /// normal case on a real filesystem, and an unencoded `file://` URL for
    /// one resolves to a different path — or to nothing.
    #[test]
    fn a_file_url_encodes_what_a_url_cannot_carry_raw() {
        assert_eq!(
            file_url(Path::new("/home/u/.cache/tmper/cover.png")),
            "file:///home/u/.cache/tmper/cover.png"
        );
        assert_eq!(
            file_url(Path::new("/home/u/My Music/01 #1.png")),
            "file:///home/u/My%20Music/01%20%231.png"
        );
        assert_eq!(
            file_url(Path::new("/home/u/一首歌.png")),
            "file:///home/u/%E4%B8%80%E9%A6%96%E6%AD%8C.png"
        );
        // A literal `%` must not be mistaken for the start of an escape.
        assert_eq!(
            file_url(Path::new("/home/u/100%.png")),
            "file:///home/u/100%25.png"
        );
    }

    #[test]
    fn times_are_microseconds() {
        assert_eq!(to_time(1.5).as_micros(), 1_500_000);
        assert_eq!(to_time(245.5).as_micros(), 245_500_000);
        // A position can never be negative, and a negative `Time` on the wire
        // is a position a client would draw off the left edge of its bar.
        assert_eq!(to_time(-3.0).as_micros(), 0);
        assert!((from_time(Time::from_millis(1500)) - 1.5).abs() < 1e-9);
    }

    /// The reason the setters move the mirror: zbus announces the property
    /// itself when a `Set` returns, reading the getter before the daemon has
    /// heard the request. Without the recording, that announcement carries the
    /// *old* value and is followed by ours with the new one — two signals, the
    /// first of them wrong, on every volume a widget sets.
    ///
    /// Both halves of that: the getter answers with what was asked for as soon
    /// as the setter returns, and the snapshot that lands a moment later has
    /// nothing left to announce.
    #[tokio::test]
    async fn a_set_is_not_announced_twice() {
        let (player, _rx, state) = player();
        state.lock().unwrap().apply(&snapshot());

        player.set_volume(0.42).await.expect("call");
        assert_eq!(
            player.volume().await.expect("call"),
            0.42,
            "zbus's own PropertiesChanged reads this the moment the setter returns"
        );
        // From here the daemon's snapshots carry what it was told, so the
        // volume is not what the rest of this is about.
        let settled = || {
            let mut snap = snapshot();
            snap.volume = 0.42;
            snap
        };
        assert!(
            state.lock().unwrap().apply(&settled()).is_empty(),
            "the snapshot that confirms the set must not be a second announcement"
        );

        // The other half: tmper's one repeat mode is *two* MPRIS properties, so
        // a `Set` of either of them still owes the desktop a signal about the
        // other one — and only about that one, or a widget showing "shuffled"
        // next to a loop that still reads `None` is drawing the wrong thing
        // until the next unrelated change comes along.
        player.set_shuffle(true).await.expect("call");
        let mut shuffled = settled();
        shuffled.repeat = RepeatMode::Shuffle;
        assert_eq!(
            state.lock().unwrap().apply(&shuffled),
            vec![Property::LoopStatus(LoopStatus::Playlist)],
            "the shuffle flag was recorded; the loop status that follows it was not"
        );

        player
            .set_loop_status(LoopStatus::Track)
            .await
            .expect("call");
        let mut single = settled();
        single.repeat = RepeatMode::SingleTrack;
        assert_eq!(
            state.lock().unwrap().apply(&single),
            vec![Property::Shuffle(false)],
            "and the same the other way round"
        );
    }

    /// The published volume is the one a client set, not the one the `f32`
    /// rounding turned it into — otherwise every slider drag would end with the
    /// widget snapping a hair away from where the user let go.
    #[test]
    fn a_volume_survives_the_widening() {
        assert_eq!(to_volume(0.8), 0.8);
        assert_eq!(to_volume(0.4), 0.4);
        assert_eq!(to_volume(0.35), 0.35);
        assert_eq!(to_volume(0.0), 0.0);
        assert_eq!(to_volume(1.0), 1.0);
        // Not a fake identity: the raw widening really does differ, and three
        // decimals is what makes it not.
        assert_ne!(f64::from(0.8f32), 0.8);
        assert_eq!(f64::from(0.35f32), 0.3499999940395355);
    }
}
