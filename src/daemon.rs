//! The daemon: the one process that owns the player.
//!
//! Everything that makes sound lives here — the engine, the queue, the policy
//! that decides what plays next, and (from phase 2 on) the library. Clients
//! attach over a unix socket, ask for things, and watch [`Event`]s arrive; the
//! TUI is only the most elaborate of them. Closing the TUI is not stopping the
//! music, which is the entire point of the split.
//!
//! Two rules shape the code below.
//!
//! **The player never waits for a client.** A client that stops reading is
//! dropped, never blocked on. A stalled peer must not be able to stall
//! playback, so every write to a client is a `try_send` into a bounded mailbox
//! and every read happens on a task that owns nothing the player needs.
//!
//! **One writer per process.** [`Player`] is `!Send` — the cpal stream inside
//! it can only be driven from the thread that built it — so this module's loop
//! owns it outright and clients reach it through a channel. That is a
//! constraint the audio stack imposes, and it is the same single-writer
//! discipline the rest of the program already follows.

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use tokio::io::BufReader;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::constants::runtime;
use crate::error::{AppError, AppResult};
use crate::ipc::proto::{Event, NoticeLevel, Request, PROTOCOL_VERSION};
use crate::ipc::{read_message_async, write_message_async};
use crate::paths;
use crate::player::mpris;
use crate::player::Player;

/// What a connection task tells the loop.
///
/// The loop itself is not spawned and owns the [`Player`]; the tasks are what
/// crosses the thread boundary, so they carry plain data only.
enum FromClient {
    /// The handshake succeeded. Until this arrives the connection is not a
    /// client and holds nothing the daemon will write to.
    Joined {
        id: u64,
        tx: mpsc::Sender<Event>,
    },
    Request {
        id: u64,
        request: Request,
    },
    /// The socket closed, for any reason at all.
    Gone {
        id: u64,
    },
}

/// One attached client, as the daemon sees it.
struct Client {
    tx: mpsc::Sender<Event>,
    /// Whether *this* client asked for the spectrum. Two attached TUIs need
    /// not both show a visualizer, so the subscription is per connection while
    /// the FFT thread it drives is not.
    visualizer: bool,
    /// Set the first time a message is dropped for this client, so the log
    /// says so once instead of thirty times a second.
    warned: bool,
}

impl Client {
    /// Hand a message to the connection task. `false` means this client is
    /// finished and should be forgotten.
    fn deliver(&mut self, event: &Event, id: u64) -> bool {
        match self.tx.try_send(event.clone()) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Closed(_)) => false,
            Err(mpsc::error::TrySendError::Full(_)) => {
                // A snapshot or a spectrum frame is superseded by the next one
                // within a frame's time, so losing it costs a flicker. Losing a
                // queue change or a notice costs a fact the client will never
                // be told again — a client that far behind is not coming back,
                // and dropping it is the only honest answer.
                if matches!(event, Event::Snapshot(_) | Event::Visualizer { .. }) {
                    if !self.warned {
                        self.warned = true;
                        tracing::warn!("Client {id} is behind; dropping frames");
                    }
                    true
                } else {
                    tracing::warn!("Client {id} missed a message it cannot recover; dropping it");
                    false
                }
            }
        }
    }
}

/// The daemon's state, minus the socket.
///
/// Split from [`run`] so that everything deciding *what a client is told* is
/// reachable from a test without a listener, an audio device, or a clock — the
/// idle timeout takes its `now` as an argument for exactly that reason.
pub struct Daemon {
    player: Player,
    clients: HashMap<u64, Client>,
    next_id: u64,
    /// When the daemon first found itself with nothing to do. Cleared the
    /// moment there is a reason to stay.
    idle_since: Option<Instant>,
    /// The desktop's view of the player, when there is a session bus to tell.
    /// Absent on a machine with no desktop, in tests, and whenever the bus
    /// refused the name — none of which is a reason to stop playing music.
    mpris: Option<mpris::Handle>,
    last_checkpoint: Instant,
    state_dirty: bool,
}

/// Give the player's scanner a runtime to walk on.
///
/// Only the daemon has one: scans are spawned tasks, and a synchronous test
/// has no reactor to spawn them on. There, a scan request is refused and
/// logged — the same thing a client sees when a scan of an empty directory
/// turns up nothing.
fn attach_scanner(player: &mut Player) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        player.attach_scanner(handle);
    }
}

impl Daemon {
    pub fn new(config: &Config) -> AppResult<Self> {
        let mut player = Player::new(config)?;
        // The daemon is the program's persistent half, so it is the one that
        // restores what the last run left behind.
        player.load_state();
        attach_scanner(&mut player);
        Ok(Self {
            player,
            clients: HashMap::new(),
            next_id: 0,
            idle_since: None,
            mpris: None,
            last_checkpoint: Instant::now(),
            state_dirty: false,
        })
    }

    #[cfg(test)]
    pub fn new_headless(config: &Config) -> Self {
        let mut player = Player::new_headless(config);
        attach_scanner(&mut player);
        Self {
            player,
            clients: HashMap::new(),
            next_id: 0,
            idle_since: None,
            mpris: None,
            last_checkpoint: Instant::now(),
            state_dirty: false,
        }
    }

    /// Report to the desktop from here on. Called by [`run`] once the bus has
    /// answered; a test attaches nothing, and the routing below is then a
    /// no-op rather than a branch.
    pub fn attach_mpris(&mut self, handle: mpris::Handle) {
        self.mpris = Some(handle);
    }

    /// Claim the id for a connection that has not finished its handshake yet.
    /// Ids are never reused, so a message from a departed client can never be
    /// mistaken for one from its replacement.
    pub fn reserve_client_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// A handshaken client, which is from here on told everything that
    /// happens. It also needs to hear what already happened: a snapshot, the
    /// queue and the collection's paths are the whole of what a fresh
    /// connection has missed, which is why reconnecting needs no catch-up
    /// protocol.
    pub fn client_joined(&mut self, id: u64, tx: mpsc::Sender<Event>) {
        let snapshot = self.player.state();
        let queue_rev = snapshot.queue_rev;
        self.clients.insert(
            id,
            Client {
                tx,
                visualizer: false,
                warned: false,
            },
        );
        self.send_to(id, Event::Snapshot(Box::new(snapshot)));
        self.send_to(
            id,
            Event::Queue {
                rev: queue_rev,
                tracks: self.player.queue(),
            },
        );
        self.send_to(
            id,
            Event::LibraryPaths {
                paths: self.player.library_paths(),
            },
        );
        self.send_to(
            id,
            Event::Playlists {
                playlists: self.player.playlists().to_vec(),
            },
        );
    }

    pub fn client_left(&mut self, id: u64) {
        // Dropping the sender is what ends the connection task's writer.
        let was_watched = self.any_visualizer();
        self.clients.remove(&id);
        self.stop_unobserved_fft(was_watched);
    }

    fn stop_unobserved_fft(&mut self, was_watched: bool) {
        if was_watched && !self.any_visualizer() {
            let _ = self
                .player
                .execute(Request::SubscribeVisualizer { on: false });
        }
    }

    /// Apply one command from one client, and tell everyone what changed.
    pub fn handle(&mut self, id: u64, request: Request) {
        if !self.clients.contains_key(&id) {
            return; // it left while this was in flight
        }

        if let Request::SubscribeVisualizer { on } = request {
            let before = self.any_visualizer();
            if let Some(client) = self.clients.get_mut(&id) {
                client.visualizer = on;
            }
            // The FFT thread belongs to the player, not to a connection: it
            // runs while *any* client is watching and stops when the last one
            // looks away. Forwarding every subscribe would start it twice, and
            // forwarding every unsubscribe would stop it for the client that
            // still wants it.
            if self.any_visualizer() != before {
                self.apply_to_player(Request::SubscribeVisualizer { on: !before }, Some(id));
            }
            return;
        }

        self.apply_to_player(request, Some(id));

        if self.player.should_shutdown() {
            self.broadcast(Event::Bye);
        }
    }

    /// Apply one command from the desktop bus.
    ///
    /// The same path a client's command takes, minus the client: MPRIS is not
    /// attached, so it has no mailbox to fill and no id — everything it causes
    /// is routed to whoever *is* attached, exactly as if a key had been
    /// pressed.
    pub fn handle_mpris(&mut self, request: Request) {
        self.apply_to_player(request, None);

        if self.player.should_shutdown() {
            self.broadcast(Event::Bye);
        }
    }

    /// Advance the player's clock and pass on what it produced.
    pub fn tick(&mut self) {
        for event in self.player.tick() {
            self.route(event);
        }
    }

    /// Whether the daemon should exit now.
    ///
    /// Idle means exactly two things: nobody is attached, and no sound is
    /// being produced. A paused player counts as idle — it is holding a place,
    /// not playing, and a daemon that stayed alive for a pause would never
    /// exit at all, since a pause can be left standing for days. The place is
    /// not lost: `state.json` carries the queue and the position, and the next
    /// run comes back parked on the same track at the same second.
    ///
    /// `now` is a parameter because the idle timeout is the one piece of
    /// daemon policy with a clock in it, and a test should not have to wait
    /// five minutes to see it fire.
    pub fn should_exit(&mut self, now: Instant) -> bool {
        if self.player.should_shutdown() {
            return true;
        }
        if !self.clients.is_empty()
            || self.player.state().status.is_active()
            || self.player.is_scanning()
        {
            self.idle_since = None;
            return false;
        }
        let since = *self.idle_since.get_or_insert(now);
        now.duration_since(since) >= Duration::from_secs(runtime::DAEMON_IDLE_EXIT_SECS)
    }

    /// Persist, say goodbye, and let the device go.
    pub fn shutdown(&mut self) {
        self.player.save_state();
        self.broadcast(Event::Bye);
        self.clients.clear();
        self.player.shutdown();
    }

    /// Debounce edits to one write per second and checkpoint active playback
    /// every 30 seconds. Normal shutdown always saves immediately.
    fn checkpoint(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.last_checkpoint);
        if (self.state_dirty && elapsed >= Duration::from_secs(1))
            || (elapsed >= Duration::from_secs(30) && self.player.state().status.is_active())
        {
            self.state_dirty = !self.player.save_state();
            self.last_checkpoint = now;
        }
    }

    // ── Plumbing ──

    /// Hand a command to the player and route whatever it produced.
    ///
    /// The seek is the one command whose *result* is invisible to a watcher:
    /// a track change is in the metadata and a pause is in the status, but a
    /// jump leaves every property exactly as it was. MPRIS has a separate
    /// signal for precisely that, so it is sent after the state it applies to
    /// — a client that then re-read `Position` gets the new one.
    fn apply_to_player(&mut self, request: Request, reply_to: Option<u64>) {
        if matches!(
            request,
            Request::Play { .. }
                | Request::Toggle
                | Request::Pause
                | Request::Resume
                | Request::Stop
                | Request::Next
                | Request::Prev
                | Request::SeekRelative { .. }
                | Request::SetVolume { .. }
                | Request::VolumeStep { .. }
                | Request::SetRepeat { .. }
                | Request::QueuePush { .. }
                | Request::QueueRemove { .. }
                | Request::SetLyricsOffset { .. }
                | Request::SetActivePlaylist { .. }
                | Request::PlaylistDelete { .. }
        ) {
            self.state_dirty = true;
        }
        let seek = matches!(request, Request::SeekRelative { .. });
        for event in self.player.execute(request) {
            let reply = matches!(
                event,
                Event::LibraryArtists { .. }
                    | Event::LibraryAlbums { .. }
                    | Event::LibraryTracks { .. }
                    | Event::SearchResults { .. }
                    | Event::PlaylistAdded { .. }
                    | Event::PlaylistImported { .. }
                    | Event::PlaylistsExported { .. }
                    | Event::Notice { .. }
                    | Event::Synced { .. }
            );
            if let (true, Some(id)) = (reply, reply_to) {
                self.send_to(id, event);
            } else {
                self.route(event);
            }
        }
        if seek {
            if let Some(mpris) = self.mpris.as_ref() {
                mpris.seeked(self.player.position_secs());
            }
        }
    }

    fn any_visualizer(&self) -> bool {
        self.clients.values().any(|c| c.visualizer)
    }

    fn route(&mut self, event: Event) {
        // The desktop hears about a snapshot the same way a client does, and
        // from the same place: the state the widget shows and the state the
        // TUI shows are one state, published once.
        if let (Event::Snapshot(snapshot), Some(mpris)) = (&event, self.mpris.as_ref()) {
            mpris.apply(snapshot);
        }
        let spectrum = matches!(event, Event::Visualizer { .. });
        let was_watched = self.any_visualizer();
        self.clients.retain(|id, client| {
            if spectrum && !client.visualizer {
                return true;
            }
            client.deliver(&event, *id)
        });
        self.stop_unobserved_fft(was_watched);
    }

    fn broadcast(&mut self, event: Event) {
        let was_watched = self.any_visualizer();
        self.clients
            .retain(|id, client| client.deliver(&event, *id));
        self.stop_unobserved_fft(was_watched);
    }

    fn send_to(&mut self, id: u64, event: Event) {
        let was_watched = self.any_visualizer();
        if let Some(client) = self.clients.get_mut(&id) {
            if !client.deliver(&event, id) {
                self.clients.remove(&id);
            }
        }
        self.stop_unobserved_fft(was_watched);
    }
}

/// Run the daemon until it is told to stop or has been idle long enough.
pub async fn run() -> AppResult<()> {
    paths::ensure_runtime_dir()?;
    let socket = paths::socket_path();

    // A socket that answers belongs to a daemon that is already running, and
    // two players on one sound card is not a state this program has an answer
    // for — the second one leaves. A socket that *refuses* was left behind by
    // a daemon that died, and replacing it is ours to do.
    if socket.exists() {
        if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
            tracing::info!("A daemon is already listening on {socket:?}");
            return Ok(());
        }
        let _ = std::fs::remove_file(&socket);
    }

    // The player is built before the socket is advertised: a machine with no
    // sound card should fail to start a daemon, not accept clients that will
    // never hear anything.
    let config = Config::load_or_default();
    let mut daemon = Daemon::new(&config)?;

    let listener = UnixListener::bind(&socket)
        .map_err(|e| AppError::Ipc(format!("failed to bind {socket:?}: {e}")))?;
    // The runtime directory is already 0700, but the socket's own mode comes
    // from the process umask, which is not necessarily anything.
    let _ = std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600));

    // After the socket, and never fatally: the desktop is a bonus, and a
    // session bus that will not answer must not be able to keep the player
    // from starting.
    let mut from_mpris = match mpris::start().await {
        Some((handle, requests)) => {
            daemon.attach_mpris(handle);
            Some(requests)
        }
        None => None,
    };

    tracing::info!(
        "tmper daemon listening on {socket:?} (pid {})",
        std::process::id()
    );

    let (to_daemon, mut from_client) = mpsc::channel::<FromClient>(runtime::DAEMON_CLIENT_QUEUE);
    let mut connections = tokio::task::JoinSet::new();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let frame_rate = config.visualizer.frame_rate.max(1);
    let mut ticker = tokio::time::interval(Duration::from_millis((1000 / frame_rate) as u64));

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let id = daemon.reserve_client_id();
                        // The connection task owns the socket and nothing
                        // else; the player is untouched by anything it does.
                        connections.spawn(serve_client(id, stream, to_daemon.clone()));
                    }
                    // Losing one connection is not losing the daemon. A
                    // listener that fails every time will simply be logged
                    // every time.
                    Err(e) => tracing::warn!("Failed to accept a client: {e}"),
                }
            }
            Some(message) = from_client.recv() => match message {
                FromClient::Joined { id, tx } => daemon.client_joined(id, tx),
                FromClient::Request { id, request } => daemon.handle(id, request),
                FromClient::Gone { id } => daemon.client_left(id),
            },
            Some(request) = desktop_command(&mut from_mpris) => daemon.handle_mpris(request),
            _ = ticker.tick() => daemon.tick(),
            _ = tokio::signal::ctrl_c() => break,
            _ = terminate.recv() => break,
            _ = connections.join_next(), if !connections.is_empty() => {},
        }

        daemon.checkpoint(Instant::now());
        if daemon.should_exit(Instant::now()) {
            break;
        }
    }

    daemon.shutdown();
    drop(from_client);
    // Flush the final state/Bye, with a deadline for peers that stopped reading.
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    connections.abort_all();
    // The file outlives the process that made it, and a stale socket turns the
    // next start into a connection-refused retry loop. Removing it is the
    // daemon's job because the daemon is what put it there.
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

/// The next command from the desktop, or a future that never resolves.
///
/// `select!` needs an arm that can be disarmed, and an `Option<Receiver>` is
/// how that is spelled: with no bus there is nothing to receive from, and
/// `pending` is a branch that simply never fires. A `Receiver` closed by its
/// sender ends the stream, which is the same thing as having no bus — the arm
/// stops firing for good.
async fn desktop_command(rx: &mut Option<mpsc::UnboundedReceiver<Request>>) -> Option<Request> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// Start a daemon as a process of its own.
///
/// This is the only place the program re-executes itself, and it is what keeps
/// the daemon in the same single binary as the TUI rather than in a second
/// executable to install, document and keep in step.
///
/// Under `cfg(test)` it refuses, and the refusal is not a formality: the test
/// binary *is* `current_exe()`, so a spawn there would re-run the whole suite —
/// detached, with its output sent to `/dev/null`, past the end of the test that
/// caused it. Nothing in the suite has a daemon to start; a test that drives
/// the reconnect path past a failed dial must get an error it can ignore, and
/// the real `spawn_detached` is exercised where it can be: by hand, on a
/// desktop, through `tmper` itself.
pub fn spawn_detached() -> AppResult<()> {
    #[cfg(test)]
    {
        Err(AppError::Ipc("not started from a test binary".into()).into())
    }

    #[cfg(not(test))]
    {
        spawn_process()
    }
}

#[cfg(not(test))]
fn spawn_process() -> AppResult<()> {
    use std::os::unix::process::CommandExt;

    let exe = std::env::current_exe()
        .map_err(|e| AppError::Ipc(format!("cannot find my own binary: {e}")))?;
    let mut command = std::process::Command::new(exe);
    command
        .arg("daemon")
        // The daemon has no terminal, and saying so explicitly means it cannot
        // inherit one and be killed by its hangup.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // A session of its own, so closing the terminal the client was started
    // from does not hang the player up. This is the whole point of the split:
    // the music outlives the window.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command
        .spawn()
        .map_err(|e| AppError::Ipc(format!("failed to start the player: {e}")))?;
    Ok(())
}

/// One client's connection, from handshake to hang-up.
async fn serve_client(id: u64, stream: UnixStream, to_daemon: mpsc::Sender<FromClient>) {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    // The handshake comes first, so a peer that connects and says nothing
    // costs a task — never a client slot, and never a message it would have to
    // have its mailbox drained for.
    let hello = tokio::time::timeout(
        Duration::from_millis(runtime::DAEMON_HELLO_TIMEOUT_MS),
        read_message_async::<_, Request>(&mut reader),
    )
    .await;

    if !matches!(&hello, Ok(Ok(Some(Request::Hello { proto, .. }))) if *proto == PROTOCOL_VERSION) {
        let _ = write_message_async(
            &mut write_half,
            &Event::Notice {
                level: NoticeLevel::Error,
                message: format!(
                    "this daemon speaks protocol {PROTOCOL_VERSION}; restart the client"
                ),
            },
        )
        .await;
        return;
    }

    let welcome = Event::Welcome {
        proto: PROTOCOL_VERSION,
        version: env!("CARGO_PKG_VERSION").to_string(),
        pid: std::process::id(),
    };
    if write_message_async(&mut write_half, &welcome)
        .await
        .is_err()
    {
        return;
    }

    let (mail_tx, mut mail_rx) = mpsc::channel(runtime::DAEMON_CLIENT_QUEUE);
    if to_daemon
        .send(FromClient::Joined { id, tx: mail_tx })
        .await
        .is_err()
    {
        return;
    }

    let mut writer = tokio::spawn(async move {
        while let Some(event) = mail_rx.recv().await {
            if write_message_async(&mut write_half, &event).await.is_err() {
                break;
            }
        }
    });

    loop {
        let result = tokio::select! {
            result = read_message_async::<_, Request>(&mut reader) => result,
            _ = &mut writer => break,
        };
        match result {
            Ok(Some(request)) => {
                if to_daemon
                    .send(FromClient::Request { id, request })
                    .await
                    .is_err()
                {
                    break;
                }
            }
            // A client that hangs up has said goodbye; that is not an error.
            Ok(None) => break,
            Err(error) => {
                tracing::debug!("Client {id} dropped: {error}");
                break;
            }
        }
    }

    let _ = to_daemon.send(FromClient::Gone { id }).await;
    // A peer that stopped reading must not retain a task or a socket forever.
    // The read future is cancelled only when this whole connection is ending.
    writer.abort();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::proto::QueueTrack;

    #[tokio::test]
    async fn disconnecting_the_last_subscriber_stops_the_fft() {
        let mut daemon = daemon();
        let (watcher, _rx) = attach(&mut daemon);
        daemon.handle(watcher, Request::SubscribeVisualizer { on: true });
        assert!(daemon
            .player
            .tick()
            .iter()
            .any(|e| matches!(e, Event::Visualizer { .. })));
        daemon.client_left(watcher);
        assert!(!daemon
            .player
            .tick()
            .iter()
            .any(|e| matches!(e, Event::Visualizer { .. })));
    }

    #[tokio::test]
    async fn operation_replies_belong_to_the_requester_while_state_is_shared() {
        let mut daemon = daemon();
        let (a, mut rx_a) = attach(&mut daemon);
        let (_, mut rx_b) = attach(&mut daemon);
        daemon.handle(
            a,
            Request::PlaylistCreate {
                name: "A's playlist".into(),
            },
        );
        let a_events = drain(&mut rx_a);
        let b_events = drain(&mut rx_b);
        assert!(a_events
            .iter()
            .any(|e| matches!(e, Event::PlaylistAdded { .. })));
        assert!(!b_events
            .iter()
            .any(|e| matches!(e, Event::PlaylistAdded { .. })));
        assert!(b_events
            .iter()
            .any(|e| matches!(e, Event::Playlists { .. })));
        daemon.handle(
            a,
            Request::SearchLibrary {
                query: "private query".into(),
            },
        );
        assert!(drain(&mut rx_a)
            .iter()
            .any(|e| matches!(e, Event::SearchResults { .. })));
        assert!(!drain(&mut rx_b)
            .iter()
            .any(|e| matches!(e, Event::SearchResults { .. })));
        daemon.handle(a, Request::SetVolume { volume: 0.3 });
        daemon.handle(a, Request::Sync { id: 42 });
        let a_events = drain(&mut rx_a);
        let b_events = drain(&mut rx_b);
        assert!(a_events
            .iter()
            .any(|e| matches!(e, Event::Synced { id: 42 })));
        assert!(!b_events.iter().any(|e| matches!(e, Event::Synced { .. })));
        assert!(snapshots(&b_events).iter().all(|s| s.volume == 0.3));
    }

    #[tokio::test]
    async fn a_large_queue_crosses_a_real_socket_during_the_greeting() {
        let (mut daemon, mut from_client, (mut writer, mut reader)) = connected();
        let tracks: Vec<_> = (0..12000)
            .map(|i| QueueTrack {
                path: format!("/music/{i}.flac").into(),
                title: "长标题音乐".repeat(10),
                artist: "Artist".into(),
                duration_secs: 3.0,
            })
            .collect();
        daemon.player.set_queue(tracks.clone());
        say_hello(&mut writer).await;
        let _: Event = read_message_async(&mut reader).await.unwrap().unwrap();
        apply(&mut daemon, next(&mut from_client).await);
        let _: Event = read_message_async(&mut reader).await.unwrap().unwrap();
        let event: Event =
            tokio::time::timeout(Duration::from_secs(5), read_message_async(&mut reader))
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        assert!(matches!(event, Event::Queue { tracks: received, .. } if received == tracks));
    }

    fn daemon() -> Daemon {
        Daemon::new_headless(&Config::default())
    }

    /// A client's mailbox, and the receiver the connection task would hold.
    fn mailbox() -> (mpsc::Sender<Event>, mpsc::Receiver<Event>) {
        mpsc::channel(runtime::DAEMON_CLIENT_QUEUE)
    }

    /// Attach a client and throw away the greeting — snapshot, queue and
    /// library paths — so a test only sees what happens next.
    fn attach(daemon: &mut Daemon) -> (u64, mpsc::Receiver<Event>) {
        let (tx, mut rx) = mailbox();
        let id = daemon.reserve_client_id();
        daemon.client_joined(id, tx);
        assert!(matches!(rx.try_recv(), Ok(Event::Snapshot(_))));
        assert!(matches!(rx.try_recv(), Ok(Event::Queue { .. })));
        assert!(matches!(rx.try_recv(), Ok(Event::LibraryPaths { .. })));
        assert!(matches!(rx.try_recv(), Ok(Event::Playlists { .. })));
        (id, rx)
    }

    fn drain(rx: &mut mpsc::Receiver<Event>) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    fn snapshots(events: &[Event]) -> Vec<&crate::ipc::proto::StateSnapshot> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Snapshot(s) => Some(&**s),
                _ => None,
            })
            .collect()
    }

    /// The whole reason a client needs no catch-up protocol: connecting is
    /// itself the catch-up.
    #[test]
    fn a_new_client_is_told_the_state_the_queue_the_paths_and_the_playlists() {
        let mut daemon = daemon();
        let (tx, mut rx) = mailbox();

        let id = daemon.reserve_client_id();
        daemon.client_joined(id, tx);

        let events = drain(&mut rx);
        assert!(matches!(events.first(), Some(Event::Snapshot(_))));
        assert!(matches!(events.get(1), Some(Event::Queue { .. })));
        assert!(matches!(events.get(2), Some(Event::LibraryPaths { .. })));
        assert!(matches!(events.get(3), Some(Event::Playlists { .. })));
        assert_eq!(events.len(), 4, "and nothing else: {events:?}");
    }

    /// Ids are handed out before the handshake finishes and never reused, so a
    /// message from a client that has already gone cannot be delivered to its
    /// successor.
    #[test]
    fn client_ids_are_never_reused() {
        let mut daemon = daemon();
        let first = daemon.reserve_client_id();
        let (tx, _rx) = mailbox();
        daemon.client_joined(first, tx);
        daemon.client_left(first);
        assert_ne!(daemon.reserve_client_id(), first);
    }

    /// Every command produces a snapshot, and every client gets it — the
    /// daemon has no notion of a "current" client to favour.
    #[tokio::test]
    async fn a_command_reaches_every_client() {
        let mut daemon = daemon();
        let (a, mut rx_a) = attach(&mut daemon);
        let (_b, mut rx_b) = attach(&mut daemon);

        daemon.handle(a, Request::SetVolume { volume: 0.25 });

        for rx in [&mut rx_a, &mut rx_b] {
            let events = drain(rx);
            let snapshots = snapshots(&events);
            assert_eq!(snapshots.len(), 1, "one command, one snapshot");
            assert_eq!(snapshots[0].volume, 0.25);
        }
    }

    /// A command from a client that has already left is not a panic and not a
    /// delivery to whoever holds that id now.
    #[test]
    fn a_command_from_a_departed_client_is_ignored() {
        let mut daemon = daemon();
        let (id, mut rx) = attach(&mut daemon);
        daemon.client_left(id);
        // Nothing new: the greeting was drained by `attach`.

        daemon.handle(id, Request::SetVolume { volume: 0.1 });

        assert!(drain(&mut rx).is_empty());
    }

    /// The spectrum is the one thing not everybody wants; sending it to a
    /// client that never asked would be 30 messages a second of pure waste.
    #[tokio::test]
    async fn the_spectrum_only_goes_to_clients_that_asked_for_it() {
        let mut daemon = daemon();
        let (watcher, mut rx_watcher) = attach(&mut daemon);
        let (_quiet, mut rx_quiet) = attach(&mut daemon);

        daemon.handle(watcher, Request::SubscribeVisualizer { on: true });
        drain(&mut rx_watcher);
        drain(&mut rx_quiet);
        daemon.tick();

        assert!(
            drain(&mut rx_watcher)
                .iter()
                .any(|e| matches!(e, Event::Visualizer { .. })),
            "a subscriber gets the spectrum"
        );
        assert!(
            !drain(&mut rx_quiet)
                .iter()
                .any(|e| matches!(e, Event::Visualizer { .. })),
            "a non-subscriber does not"
        );
    }

    /// The FFT thread is the player's, and it follows the *aggregate* of the
    /// subscriptions: starting it once per subscriber would leak a thread, and
    /// stopping it when one of two clients looks away would starve the other.
    #[tokio::test]
    async fn the_fft_follows_the_last_subscriber() {
        let mut daemon = daemon();
        let (first, mut rx_first) = attach(&mut daemon);
        let (second, _rx_second) = attach(&mut daemon);

        daemon.handle(first, Request::SubscribeVisualizer { on: true });
        drain(&mut rx_first);
        daemon.handle(second, Request::SubscribeVisualizer { on: true });
        // The second subscription changed nothing, so it must not have reached
        // the player: a second FFT thread would be running now.
        assert!(drain(&mut rx_first).is_empty());
        daemon.tick();
        assert!(drain(&mut rx_first)
            .iter()
            .any(|e| matches!(e, Event::Visualizer { .. })));

        // One of two leaving is not the last one leaving.
        daemon.handle(second, Request::SubscribeVisualizer { on: false });
        daemon.tick();
        assert!(
            drain(&mut rx_first)
                .iter()
                .any(|e| matches!(e, Event::Visualizer { .. })),
            "the remaining subscriber keeps its spectrum"
        );

        daemon.handle(first, Request::SubscribeVisualizer { on: false });
        daemon.tick();
        assert!(
            !drain(&mut rx_first)
                .iter()
                .any(|e| matches!(e, Event::Visualizer { .. })),
            "and the last one turning away stops it"
        );
    }

    /// A client that reads nothing must be able to pile messages up without
    /// the daemon so much as hesitating — but only up to a point, and the
    /// point is a bound on memory rather than on patience.
    #[tokio::test]
    async fn a_stalled_client_never_makes_the_daemon_wait() {
        let mut daemon = daemon();
        let (stalled, mut rx_stalled) = attach(&mut daemon);
        let (fast, mut rx_fast) = attach(&mut daemon);

        for _ in 0..runtime::DAEMON_CLIENT_QUEUE * 2 {
            daemon.tick();
        }

        assert_eq!(
            drain(&mut rx_stalled).len(),
            runtime::DAEMON_CLIENT_QUEUE,
            "the mailbox stops at its bound instead of growing"
        );
        assert!(
            daemon.clients.contains_key(&stalled),
            "being behind on frames is not a reason to be disconnected"
        );
        assert!(daemon.clients.contains_key(&fast));
        assert!(!drain(&mut rx_fast).is_empty());
    }

    /// Frames are superseded by the next tick, so dropping one costs a
    /// flicker. A notice is a fact the client will never be told again, and a
    /// mailbox that cannot hold one belongs to a client that cannot be served.
    #[tokio::test]
    async fn a_client_that_misses_a_fact_is_dropped() {
        let mut daemon = daemon();
        let (stalled, _rx) = attach(&mut daemon);

        // Exactly one more than the mailbox holds: the first `QUEUE` fit, and
        // the one after them is the one that does not.
        for _ in 0..runtime::DAEMON_CLIENT_QUEUE + 1 {
            daemon.broadcast(Event::Notice {
                level: NoticeLevel::Info,
                message: String::new(),
            });
        }

        assert!(
            !daemon.clients.contains_key(&stalled),
            "a client this far behind is not coming back"
        );
    }

    /// Losing a client is not losing the daemon.
    #[tokio::test]
    async fn a_client_hanging_up_leaves_the_rest_working() {
        let mut daemon = daemon();
        let (leaving, _rx_leaving) = attach(&mut daemon);
        let (staying, mut rx_staying) = attach(&mut daemon);

        daemon.client_left(leaving);
        daemon.handle(staying, Request::SetVolume { volume: 0.4 });

        let events = drain(&mut rx_staying);
        assert_eq!(snapshots(&events)[0].volume, 0.4);
    }

    // ── Lifecycle ──

    /// With nobody attached and nothing playing, the daemon waits out its
    /// idle timeout and then goes.
    #[test]
    fn an_idle_daemon_exits_after_the_timeout_and_not_before() {
        let mut daemon = daemon();
        let start = Instant::now();

        assert!(!daemon.should_exit(start));
        assert!(!daemon.should_exit(start + Duration::from_secs(1)));
        assert!(
            !daemon.should_exit(start + Duration::from_secs(runtime::DAEMON_IDLE_EXIT_SECS - 1)),
            "one second early is still early"
        );
        assert!(daemon.should_exit(start + Duration::from_secs(runtime::DAEMON_IDLE_EXIT_SECS)));
    }

    /// An attached client is a reason to live, whatever the player is doing.
    #[test]
    fn an_attached_client_keeps_the_daemon_alive() {
        let mut daemon = daemon();
        let (_id, _rx) = attach(&mut daemon);
        let start = Instant::now();

        assert!(!daemon.should_exit(start + Duration::from_secs(86_400)));
    }

    /// Work started in the meantime resets the clock: an idle timeout measured
    /// from "the last time there was nothing to do", not from process start.
    #[test]
    fn a_client_arriving_and_leaving_restarts_the_idle_clock() {
        let mut daemon = daemon();
        let start = Instant::now();
        assert!(!daemon.should_exit(start));

        let (id, _rx) = attach(&mut daemon);
        let later = start + Duration::from_secs(runtime::DAEMON_IDLE_EXIT_SECS);
        assert!(!daemon.should_exit(later), "a client is attached");

        daemon.client_left(id);
        assert!(
            !daemon.should_exit(later),
            "the clock restarts from the moment it went idle again"
        );
        assert!(daemon.should_exit(later + Duration::from_secs(runtime::DAEMON_IDLE_EXIT_SECS)));
    }

    /// Playing is the other reason to live. This is the whole point of the
    /// daemon: the music outlives the window.
    #[tokio::test]
    async fn playback_keeps_the_daemon_alive() {
        let mut daemon = daemon();
        let path = std::fs::canonicalize("tests/fixtures/test.wav").expect("fixture");
        daemon.apply_to_player(Request::Play { path }, None);

        let start = Instant::now();
        assert!(!daemon.should_exit(start + Duration::from_secs(86_400)));
    }

    /// A pause is not playback. It holds a place, and the daemon that held it
    /// forever would be a daemon nobody could get rid of short of `tmper
    /// quit` — the session it was holding comes back from `state.json` anyway.
    #[tokio::test]
    async fn a_paused_daemon_is_idle_like_any_other() {
        let mut daemon = daemon();
        let path = std::fs::canonicalize("tests/fixtures/test.wav").expect("fixture");
        daemon.apply_to_player(Request::Play { path }, None);
        daemon.apply_to_player(Request::Pause, None);
        assert!(
            matches!(
                daemon.player.state().status,
                crate::audio::engine::PlaybackState::Paused
            ),
            "the test needs a pause to be about a pause"
        );

        let start = Instant::now();
        assert!(!daemon.should_exit(start), "a pause is not playback");
        assert!(
            !daemon.should_exit(start + Duration::from_secs(runtime::DAEMON_IDLE_EXIT_SECS - 1)),
            "one second early is still early"
        );
        assert!(daemon.should_exit(start + Duration::from_secs(runtime::DAEMON_IDLE_EXIT_SECS)));
    }

    /// `:quit!` and `tmper quit` reach every attached client, so a TUI that is
    /// watching stops watching instead of painting a frozen position forever.
    #[tokio::test]
    async fn a_shutdown_request_says_bye_to_everyone_and_ends_the_daemon() {
        let mut daemon = daemon();
        let (a, mut rx_a) = attach(&mut daemon);
        let (_b, mut rx_b) = attach(&mut daemon);

        daemon.handle(a, Request::Shutdown);

        for rx in [&mut rx_a, &mut rx_b] {
            assert!(
                drain(rx).iter().any(|e| matches!(e, Event::Bye)),
                "every client hears about it"
            );
        }
        assert!(daemon.should_exit(Instant::now()));
    }

    /// Queue changes are the thing a client cannot reconstruct, so they reach
    /// everyone who is attached.
    #[tokio::test]
    async fn a_queue_change_is_pushed_to_every_client() {
        let mut daemon = daemon();
        let (a, mut rx_a) = attach(&mut daemon);
        let (_b, mut rx_b) = attach(&mut daemon);
        let fixture = std::fs::canonicalize("tests/fixtures/test.wav").expect("fixture");

        daemon.handle(
            a,
            Request::QueuePush {
                path: fixture.clone(),
            },
        );

        for rx in [&mut rx_a, &mut rx_b] {
            let events = drain(rx);
            let queue = events
                .iter()
                .find_map(|e| match e {
                    Event::Queue { tracks, .. } => Some(tracks),
                    _ => None,
                })
                .expect("a queue event");
            assert_eq!(
                queue,
                &vec![QueueTrack {
                    path: fixture.clone(),
                    title: "test".into(),
                    artist: "Unknown Artist".into(),
                    duration_secs: 2.0,
                }],
                "the row is built from the file's tags"
            );
        }
    }

    // ── The socket ──
    //
    // `run` itself needs a listener and a real audio device; what is left to
    // test here is the half that does not — the handshake, the framing, and
    // the translation between a byte stream and the daemon's vocabulary.

    fn version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    /// Both halves of the socket as the test client holds them.
    type ClientEnd = (
        tokio::net::unix::OwnedWriteHalf,
        BufReader<tokio::net::unix::OwnedReadHalf>,
    );

    /// Wire up a daemon, a connection task and the far end of a socket.
    ///
    /// `run` itself needs a listener and a real audio device; this is
    /// everything between "a socket connected" and "the daemon loop", which is
    /// where the handshake, the framing and the translation live.
    fn connected() -> (Daemon, mpsc::Receiver<FromClient>, ClientEnd) {
        let mut daemon = daemon();
        let (to_daemon, from_client) = mpsc::channel(64);
        let (server, client) = UnixStream::pair().expect("socketpair");
        let id = daemon.reserve_client_id();
        tokio::spawn(serve_client(id, server, to_daemon));
        let (read_half, write_half) = client.into_split();
        (daemon, from_client, (write_half, BufReader::new(read_half)))
    }

    /// The next thing the connection task handed the daemon. Waited for:
    /// "the task has not run yet" is a scheduling detail, not something a test
    /// should be asserting on.
    async fn next(from_client: &mut mpsc::Receiver<FromClient>) -> FromClient {
        tokio::time::timeout(Duration::from_secs(5), from_client.recv())
            .await
            .expect("the connection task reached the daemon")
            .expect("a message")
    }

    fn apply(daemon: &mut Daemon, message: FromClient) {
        match message {
            FromClient::Joined { id, tx } => daemon.client_joined(id, tx),
            FromClient::Request { id, request } => daemon.handle(id, request),
            FromClient::Gone { id } => daemon.client_left(id),
        }
    }

    async fn say_hello(writer: &mut tokio::net::unix::OwnedWriteHalf) {
        write_message_async(
            writer,
            &Request::Hello {
                proto: PROTOCOL_VERSION,
                version: version(),
            },
        )
        .await
        .expect("write hello");
    }

    /// The handshake and the greeting, over a real socket: connect, `Hello`,
    /// `Welcome`, then the snapshot and queue that make a fresh client current.
    #[tokio::test]
    async fn a_client_that_says_hello_is_greeted_and_brought_up_to_date() {
        let (mut daemon, mut from_client, (mut writer, mut reader)) = connected();
        say_hello(&mut writer).await;

        let welcome: Option<Event> = read_message_async(&mut reader).await.expect("read");
        match welcome {
            Some(Event::Welcome { proto, pid, .. }) => {
                assert_eq!(proto, PROTOCOL_VERSION);
                assert_eq!(pid, std::process::id());
            }
            other => panic!("expected a welcome, got {other:?}"),
        }

        // The daemon has to run for the greeting to follow.
        let joined = next(&mut from_client).await;
        apply(&mut daemon, joined);
        let first: Option<Event> = read_message_async(&mut reader).await.expect("read");
        let second: Option<Event> = read_message_async(&mut reader).await.expect("read");
        let third: Option<Event> = read_message_async(&mut reader).await.expect("read");
        let fourth: Option<Event> = read_message_async(&mut reader).await.expect("read");
        assert!(matches!(first, Some(Event::Snapshot(_))));
        assert!(matches!(second, Some(Event::Queue { .. })));
        assert!(matches!(third, Some(Event::LibraryPaths { .. })));
        assert!(matches!(fourth, Some(Event::Playlists { .. })));
    }

    /// A command written to the socket arrives as a `Request`, and the answer
    /// comes back as an `Event` — the whole asymmetry of the protocol in one
    /// test.
    #[tokio::test]
    async fn a_command_crosses_the_socket_and_its_answer_comes_back() {
        let (mut daemon, mut from_client, (mut writer, mut reader)) = connected();
        say_hello(&mut writer).await;
        let _welcome: Option<Event> = read_message_async(&mut reader).await.expect("read");
        let joined = next(&mut from_client).await;
        apply(&mut daemon, joined);
        // The greeting, so what follows is only the answer.
        let _: Option<Event> = read_message_async(&mut reader).await.expect("read");
        let _: Option<Event> = read_message_async(&mut reader).await.expect("read");
        let _: Option<Event> = read_message_async(&mut reader).await.expect("read");
        let _: Option<Event> = read_message_async(&mut reader).await.expect("read");

        write_message_async(&mut writer, &Request::SetVolume { volume: 0.25 })
            .await
            .expect("write command");

        let (id, request) = match next(&mut from_client).await {
            FromClient::Request { id, request } => (id, request),
            _ => panic!("expected a request"),
        };
        assert_eq!(request, Request::SetVolume { volume: 0.25 });

        daemon.handle(id, request);
        let answer: Option<Event> = read_message_async(&mut reader).await.expect("read");
        match answer {
            Some(Event::Snapshot(snapshot)) => assert_eq!(snapshot.volume, 0.25),
            other => panic!("expected a snapshot, got {other:?}"),
        }
    }

    /// Hanging up is how a client leaves, and the daemon is told.
    #[tokio::test]
    async fn hanging_up_reaches_the_daemon_as_a_departure() {
        let (mut daemon, mut from_client, (mut writer, _reader)) = connected();
        say_hello(&mut writer).await;
        let joined = next(&mut from_client).await;
        apply(&mut daemon, joined);
        assert_eq!(daemon.clients.len(), 1);

        drop(writer);
        let gone = next(&mut from_client).await;
        apply(&mut daemon, gone);
        assert!(daemon.clients.is_empty());
    }

    /// A peer that speaks a protocol this build does not is told so, in
    /// words, rather than left staring at a socket that never answers.
    #[tokio::test]
    async fn a_client_from_the_future_is_told_why_it_cannot_connect() {
        let (to_daemon, mut from_client) = mpsc::channel(64);
        let (server, client) = UnixStream::pair().expect("socketpair");
        tokio::spawn(serve_client(1, server, to_daemon));

        let (read_half, mut write_half) = client.into_split();
        let mut reader = BufReader::new(read_half);
        write_message_async(
            &mut write_half,
            &Request::Hello {
                proto: PROTOCOL_VERSION + 1,
                version: "9.9.9".into(),
            },
        )
        .await
        .expect("write hello");

        let reply: Option<Event> = read_message_async(&mut reader).await.expect("read");
        match reply {
            Some(Event::Notice {
                level: NoticeLevel::Error,
                message,
            }) => assert!(message.contains("protocol"), "unhelpful notice: {message}"),
            other => panic!("expected a notice, got {other:?}"),
        }
        // And it is never registered — nothing is waiting on it.
        assert!(from_client.try_recv().is_err());
    }

    /// A client that connects and says nothing holds nothing: no client slot,
    /// no mailbox, no snapshot.
    #[tokio::test]
    async fn a_silent_connection_is_never_registered() {
        let (to_daemon, mut from_client) = mpsc::channel(64);
        let (server, _client) = UnixStream::pair().expect("socketpair");
        tokio::spawn(serve_client(1, server, to_daemon));

        // The connection task's only move is to time out and leave, which
        // takes the configured handshake budget.
        tokio::time::sleep(Duration::from_millis(
            runtime::DAEMON_HELLO_TIMEOUT_MS + 200,
        ))
        .await;
        assert!(from_client.try_recv().is_err());
    }
}
