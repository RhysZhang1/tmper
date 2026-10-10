//! How the TUI reaches the player.
//!
//! One trait, two delivery timings. [`LocalHandle`] owns a
//! [`crate::player::Player`] outright and answers synchronously; `DaemonHandle`
//! (the socket client) writes a request and returns nothing, with the events
//! arriving over the socket on a later tick.
//!
//! The timing differs. **Nothing else may.** Both hand their events to the same
//! [`crate::app::App::apply_event`], so the ~150 existing tests that drive a
//! `LocalHandle` are exercising the code the daemon client runs, one tick's
//! latency aside. A second, "synchronous" path for tests would make green mean
//! nothing.

use std::time::Duration;

use tokio::io::BufReader;
use tokio::sync::{mpsc, watch};

use crate::constants::runtime;
use crate::error::{AppError, AppResult};
use crate::ipc::proto::{Event, NoticeLevel, Request, PROTOCOL_VERSION};
#[cfg(test)]
use crate::player::Player;

/// Whether the client can still reach its player.
///
/// A *state*, not an event, and the distinction is the reason it is a channel
/// the client reads rather than a message it is sent: a toast times out, and
/// "the player is gone" does not stop being true because ten seconds have
/// passed. The TUI draws this for as long as it says `Lost`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Connection {
    /// Either the player is in this process, or the socket to it is up.
    #[default]
    Live,
    /// The socket dropped. Something is already trying to get it back — the
    /// client carries on in the meantime, and may leave whenever it likes.
    Lost { reason: String },
}

/// A player the TUI can talk to, wherever it lives.
///
/// Deliberately *not* `Send`. The in-process implementation owns an
/// [`crate::audio::engine::AudioEngine`], whose cpal stream is `!Send` on some
/// platforms — the engine can only be driven from the thread that built it,
/// which is the same "single writer" rule the daemon runs on. A socket
/// implementation would be `Send`, but a trait cannot demand more than its
/// weakest implementor offers, and nothing needs to move an `App` between
/// tasks.
pub trait PlayerHandle {
    /// Send a command. Returns the events it produced *immediately*; a handle
    /// whose player is on the far side of a socket returns none here and
    /// delivers them from [`PlayerHandle::poll`] instead.
    fn dispatch(&mut self, request: Request) -> Vec<Event>;

    /// Whatever the player has produced since the last call: decoder results,
    /// a track that ended, state that moved on. Never blocks.
    fn poll(&mut self) -> Vec<Event>;

    /// Advance the player's clock — the daemon's per-tick work. A socket
    /// handle has no clock to advance and returns nothing.
    fn tick(&mut self) -> Vec<Event>;

    /// The client has arrived and the terminal is ready. An in-process player
    /// restores `state.json` here, because it is also the program's startup; a
    /// socket player has been running for hours and has nothing to restore.
    fn attach(&mut self) -> AppResult<()> {
        Ok(())
    }

    /// The client is leaving. An in-process player is the whole program, so
    /// leaving means persisting and silencing it; a player behind a socket is
    /// somebody else's process and carries on.
    fn detach(&mut self) -> AppResult<()> {
        Ok(())
    }

    /// Whether the player can still be reached, as of this moment.
    ///
    /// Always [`Connection::Live`] for a player in this process: a player that
    /// has gone takes the client with it, so it has no way to be lost and
    /// watched for.
    fn connection(&self) -> Connection {
        Connection::Live
    }

    /// The player itself, when it lives in this process.
    ///
    /// `None` across a socket: there is nothing to look at over there, and
    /// reaching for it anyway is how a client starts reading state the daemon
    /// owns. Tests use this to assert on what the wire deliberately does not
    /// carry (the active list, the queue revision).
    #[cfg(test)]
    fn local(&self) -> Option<&Player> {
        None
    }

    /// As [`PlayerHandle::local`], mutably, for tests that have to put the
    /// player into a state the wire cannot ask for.
    #[cfg(test)]
    fn local_mut(&mut self) -> Option<&mut Player> {
        None
    }
}

/// The in-process player: the whole engine, queue and policy, right here.
///
/// Only the tests build one. The shipped TUI uses the socket handle, because
/// the whole point of the split is that the music outlives the window — and an
/// in-process player cannot sound after the process that owns it is gone. It
/// stays because it is what keeps ~150 app tests synchronous, and because the
/// daemon's own tests would otherwise have no way to drive a player without a
/// socket.
#[cfg(test)]
pub struct LocalHandle {
    player: Player,
}

#[cfg(test)]
impl LocalHandle {
    pub fn new(player: Player) -> Self {
        Self { player }
    }
}

#[cfg(test)]
impl PlayerHandle for LocalHandle {
    fn dispatch(&mut self, request: Request) -> Vec<Event> {
        self.player.execute(request)
    }

    fn poll(&mut self) -> Vec<Event> {
        Vec::new()
    }

    fn tick(&mut self) -> Vec<Event> {
        self.player.tick()
    }

    /// There is nothing on the other side of this handle, so "connecting" is
    /// just restoring what the last run left behind.
    fn attach(&mut self) -> AppResult<()> {
        self.player.load_state();
        Ok(())
    }

    /// The process is ending, so the player ends with it: write `state.json`
    /// and let the device go. (Behind a socket this is a no-op — the daemon
    /// saves its own state when *it* exits, which may be hours later.)
    fn detach(&mut self) -> AppResult<()> {
        self.player.save_state();
        self.player.shutdown();
        Ok(())
    }

    fn local(&self) -> Option<&Player> {
        Some(&self.player)
    }

    fn local_mut(&mut self) -> Option<&mut Player> {
        Some(&mut self.player)
    }
}

/// The socket client: the player is in another process, and this is the end of
/// the wire that holds the TUI's end of it.
///
/// Neither half of the connection is ever touched from here. Reading is a task
/// that fills [`DaemonHandle::inbox`]; writing is a task fed by
/// [`DaemonHandle::outbox`]. That is what lets [`PlayerHandle::dispatch`] stay
/// synchronous — it is called from a key handler, which cannot wait, and a
/// socket write to a daemon that has stopped reading would be exactly the wait
/// this design exists to avoid.
pub struct DaemonHandle {
    outbox: mpsc::Sender<Request>,
    inbox: mpsc::Receiver<Event>,
    /// Written by the supervisor, read every tick by the client. A `watch`
    /// rather than a lock: this is a value that changes rarely and is read
    /// often, and the reader must never be able to hold anything up.
    connection: watch::Receiver<Connection>,
}

/// One greeted socket: the two halves of a connection the daemon has already
/// answered [`Request::Hello`] on.
struct Socket {
    write: tokio::net::unix::OwnedWriteHalf,
    read: BufReader<tokio::net::unix::OwnedReadHalf>,
}

impl DaemonHandle {
    /// Connect to a running daemon. Fails if there is none — use
    /// [`DaemonHandle::connect_or_spawn`] to start one.
    ///
    /// This is the one-shot verbs' connection: it does not heal, because
    /// `tmper pause` on a player that has crashed is a failure to report rather
    /// than a player to start again.
    pub async fn connect() -> AppResult<Self> {
        let socket = Self::dial().await?;
        let (outbox, outbox_rx) = mpsc::channel::<Request>(runtime::DAEMON_CLIENT_QUEUE);
        tokio::spawn(write_requests(socket.write, outbox_rx));

        // Bounded, and deliberately the same bound the daemon applies on its
        // side: a client that stops draining its inbox applies backpressure
        // all the way to the daemon's mailbox, which is what makes "this
        // client is too far behind" a judgement both ends agree on.
        let (inbox_tx, inbox) = mpsc::channel::<Event>(runtime::DAEMON_CLIENT_QUEUE);
        tokio::spawn(read_events(socket.read, inbox_tx));

        // A connection nobody supervises can never be lost, only ended. The
        // sender is dropped here rather than kept: `borrow` still answers with
        // the value it last held, and this handle's answer is always `Live`.
        let (_no_supervisor, connection) = watch::channel(Connection::Live);
        Ok(Self {
            outbox,
            inbox,
            connection,
        })
    }

    /// Connect, greet, and hand back the socket. The part both kinds of
    /// connection share.
    async fn dial() -> AppResult<Socket> {
        let socket = crate::paths::socket_path();
        let stream = tokio::net::UnixStream::connect(&socket)
            .await
            .map_err(|e| {
                AppError::Ipc(format!(
                    "no player is running at {} ({e})",
                    socket.display()
                ))
            })?;
        let (read_half, mut write_half) = stream.into_split();
        crate::ipc::write_message_async(
            &mut write_half,
            &Request::Hello {
                proto: PROTOCOL_VERSION,
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        )
        .await
        .map_err(ipc_error)?;

        // The handshake is the one exchange that blocks, and it happens before
        // there is a TUI to block: a peer that accepts a connection and then
        // says nothing must not turn into a TUI that never draws. Everything
        // after this point is a task.
        //
        // One reader, built here and then handed to the event loop — never a
        // second one over the same socket. A `BufReader` reads as much as the
        // socket will give it (up to 8 KiB) to answer a single `read_line`, so
        // a handshake that borrowed one and threw it away would take the
        // greeting with it: the daemon sends the snapshot and the queue the
        // moment it accepts the handshake, and a client that drops those bytes
        // reads the *middle* of the next message as its first event — a JSON
        // parse error, or a wait forever for a queue it already had.
        let mut reader = BufReader::new(read_half);
        let welcome = match tokio::time::timeout(
            Duration::from_millis(runtime::DAEMON_HELLO_TIMEOUT_MS),
            crate::ipc::read_message_async::<_, Event>(&mut reader),
        )
        .await
        {
            Ok(result) => result.map_err(ipc_error)?,
            Err(_) => {
                return Err(AppError::Ipc(format!(
                    "the player did not answer within {}ms",
                    runtime::DAEMON_HELLO_TIMEOUT_MS
                ))
                .into())
            }
        };

        match welcome {
            Some(Event::Welcome { proto, .. }) if proto == PROTOCOL_VERSION => {}
            Some(Event::Welcome { proto, .. }) => {
                return Err(AppError::Ipc(format!(
                    "the player speaks protocol {proto}; this client speaks {PROTOCOL_VERSION}"
                ))
                .into());
            }
            // A daemon that refused us sends the reason in words; passing it
            // on is the difference between a user who can fix it and one who
            // cannot.
            Some(Event::Notice { message, .. }) => return Err(AppError::Ipc(message).into()),
            Some(other) => {
                return Err(AppError::Ipc(format!("unexpected greeting: {other:?}")).into());
            }
            // The connect succeeded, so something was listening a moment ago
            // and hung up instead of answering: a daemon on its way out.
            None => {
                return Err(AppError::Ipc(
                    "the player closed the connection before saying hello".into(),
                )
                .into())
            }
        }

        Ok(Socket {
            write: write_half,
            read: reader,
        })
    }

    /// Connect to a running daemon, starting one if there is none — and keep it
    /// connected: this is the TUI's handle, and the TUI is meant to outlive any
    /// one socket.
    ///
    /// Bounded on purpose: a player that cannot come up — no sound card, a
    /// socket it cannot bind — has to surface as an error the user can read,
    /// not as a TUI that hangs on a blank screen. Only the *first* connection
    /// is bounded; once the TUI is up, losing the player puts a banner on
    /// screen and starts dialing again, forever.
    pub async fn connect_or_spawn() -> AppResult<Self> {
        let socket = Self::dial_starting_one().await?;

        let (outbox, outbox_rx) = mpsc::channel::<Request>(runtime::DAEMON_CLIENT_QUEUE);
        let (inbox_tx, inbox) = mpsc::channel::<Event>(runtime::DAEMON_CLIENT_QUEUE);
        let (state, connection) = watch::channel(Connection::Live);
        tokio::spawn(supervise(socket, outbox_rx, inbox_tx, state));
        Ok(Self {
            outbox,
            inbox,
            connection,
        })
    }

    /// Dial, and if nothing answers, start a player and keep dialing for as
    /// long as a startup is allowed to take.
    async fn dial_starting_one() -> AppResult<Socket> {
        if let Ok(socket) = Self::dial().await {
            return Ok(socket);
        }
        crate::daemon::spawn_detached()?;

        let deadline =
            std::time::Instant::now() + Duration::from_millis(runtime::DAEMON_START_TIMEOUT_MS);
        let mut last = None;
        while std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(runtime::DAEMON_CONNECT_RETRY_MS)).await;
            match Self::dial().await {
                Ok(socket) => return Ok(socket),
                Err(error) => last = Some(error),
            }
        }
        Err(AppError::Ipc(format!(
            "the player did not start within {}s ({}) — see {}",
            runtime::DAEMON_START_TIMEOUT_MS / 1000,
            last.map(|e| e.to_string()).unwrap_or_default(),
            crate::paths::state_dir().join("tmper-daemon.log").display()
        ))
        .into())
    }

    /// The next event, waiting for it. For the one-shot verbs, which have
    /// nothing else to do while the daemon answers.
    pub async fn next_event(&mut self) -> Option<Event> {
        self.inbox.recv().await
    }
}

impl PlayerHandle for DaemonHandle {
    fn dispatch(&mut self, request: Request) -> Vec<Event> {
        // A full outbox means the daemon has stopped reading. Dropping the
        // command is the honest outcome: the alternative is blocking the key
        // handler on a process that may never come back.
        if self.outbox.try_send(request).is_err() {
            tracing::warn!("The player is not taking commands");
        }
        Vec::new()
    }

    fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(event) = self.inbox.try_recv() {
            events.push(event);
        }
        events
    }

    /// The daemon has its own clock and runs it whether or not anyone is
    /// attached; a client ticking one would be a second clock to keep in step.
    fn tick(&mut self) -> Vec<Event> {
        Vec::new()
    }

    fn connection(&self) -> Connection {
        self.connection.borrow().clone()
    }
}

/// Write requests until the daemon stops listening.
async fn write_requests(
    mut writer: tokio::net::unix::OwnedWriteHalf,
    mut outbox: mpsc::Receiver<Request>,
) {
    while let Some(request) = outbox.recv().await {
        if crate::ipc::write_message_async(&mut writer, &request)
            .await
            .is_err()
        {
            break;
        }
    }
}

/// Read events until the daemon is gone, and end the client with a word about
/// why.
///
/// The one-shot verbs' reader. A `tmper pause` whose player died halfway
/// through has nothing left to do: the `Bye` tells [`crate::app::App`] to
/// leave, and the notice in front of it says why, which is the difference
/// between a TUI that vanished and one that explained itself. The TUI proper
/// wants [`supervise`] instead.
async fn read_events(
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    inbox: mpsc::Sender<Event>,
) {
    let Some(reason) = forward_events(reader, inbox.clone()).await else {
        return; // the client went away; nobody is waiting for a notice
    };
    tracing::warn!("Lost the player connection: {reason}");
    let _ = inbox
        .send(Event::Notice {
            level: NoticeLevel::Error,
            message: format!("Lost the player connection: {reason}"),
        })
        .await;
    let _ = inbox.send(Event::Bye).await;
}

/// Forward events until the socket dies. `None` means the *client* is gone —
/// the receiver was dropped, so nobody is listening any more.
///
/// The reader is the handshake's, passed on rather than rebuilt — see
/// [`DaemonHandle::dial`] for what a second `BufReader` over one socket costs.
///
/// Returned rather than acted on, because what a dead socket *means* is the
/// caller's business: the same reason is a `Bye` to a one-shot verb and a
/// banner to a TUI.
async fn forward_events(
    mut reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    inbox: mpsc::Sender<Event>,
) -> Option<String> {
    loop {
        match crate::ipc::read_message_async::<_, Event>(&mut reader).await {
            Ok(Some(event)) => {
                if inbox.send(event).await.is_err() {
                    return None;
                }
            }
            // A clean EOF at a line boundary: the daemon said goodbye by
            // leaving, which for a socket in the middle of a session is a crash
            // or a kill — a graceful exit sends `Bye` first.
            Ok(None) => return Some("the player closed the connection".into()),
            Err(error) => return Some(error.to_string()),
        }
    }
}

/// Hold a connection open, and get another one when it drops.
///
/// The client's end of a session outlives any single socket, which is what
/// makes a crashed player recoverable: this task owns the channels the
/// [`DaemonHandle`] holds, dials a replacement whenever the current socket
/// dies, and publishes what it is doing through `state` — which is what the
/// TUI's banner renders.
///
/// Requests sent while disconnected stay in the outbox and go out on the next
/// connection, in order. That is the right default for what a request *is*: a
/// key the user pressed, meaning what it meant when they pressed it.
///
/// This task ends when the client does — when the handle, and with it the
/// outbox, is dropped.
async fn supervise(
    first: Socket,
    mut outbox: mpsc::Receiver<Request>,
    inbox: mpsc::Sender<Event>,
    state: watch::Sender<Connection>,
) {
    let mut socket = first;
    let mut preferences = Vec::<Request>::new();
    loop {
        let Some(reason) = serve(socket, &mut outbox, &inbox, &mut preferences).await else {
            return; // the client is gone; so is there anything to reconnect for
        };
        let _ = state.send(Connection::Lost {
            reason: reason.clone(),
        });
        let _ = inbox
            .send(Event::Notice {
                level: NoticeLevel::Warn,
                message: format!("Lost the player: {reason} — reconnecting"),
            })
            .await;

        // A player that died is usually a player that has to be started again.
        // Once per outage: a daemon that comes up and dies again leaves its
        // socket answering for long enough to be dialed, so a genuine crash
        // loop stays a crash loop rather than becoming a fork loop.
        let mut started_one = false;
        loop {
            match DaemonHandle::dial().await {
                Ok(mut dialed) => {
                    let mut restored = true;
                    for preference in &preferences {
                        if crate::ipc::write_message_async(&mut dialed.write, preference)
                            .await
                            .is_err()
                        {
                            restored = false;
                            break;
                        }
                    }
                    if !restored {
                        continue;
                    }
                    socket = dialed;
                    let _ = state.send(Connection::Live);
                    break;
                }
                Err(error) => {
                    tracing::debug!("Still no player: {error}");
                    if !started_one {
                        started_one = true;
                        if let Err(error) = crate::daemon::spawn_detached() {
                            tracing::warn!("Could not start the player again: {error}");
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(runtime::DAEMON_RECONNECT_RETRY_MS))
                        .await;
                }
            }
        }
    }
}

/// Run one connection until it dies, forwarding requests and events.
///
/// `None` when the client itself is gone; `Some(reason)` when the socket is.
///
/// The reading is a task of its own, never a `select!` branch. A read future
/// dropped mid-line takes the buffered bytes with it — the hazard the handshake
/// had, and the reason it was rewritten — so it must be allowed to fail on its
/// own terms. The forwarder, meanwhile, *is* cancel-safe (`recv` either yields
/// a request or it does not), so it is what the supervisor may drop.
async fn serve(
    socket: Socket,
    outbox: &mut mpsc::Receiver<Request>,
    inbox: &mpsc::Sender<Event>,
    preferences: &mut Vec<Request>,
) -> Option<String> {
    let (live_tx, live_rx) = mpsc::channel::<Request>(runtime::DAEMON_CLIENT_QUEUE);
    tokio::spawn(write_requests(socket.write, live_rx));
    let mut reader = tokio::spawn(forward_events(socket.read, inbox.clone()));

    loop {
        tokio::select! {
            request = outbox.recv() => match request {
                // A send that fails means the writer task is done with, which
                // means this socket is. The reader will say so; dropping it
                // here just means the reason comes from the read side, which
                // is where a socket failure is actually observed.
                Some(request) => {
                    if matches!(request, Request::SubscribeVisualizer { .. } | Request::SetFftParams { .. }) {
                        preferences.retain(|old| std::mem::discriminant(old) != std::mem::discriminant(&request));
                        preferences.push(request.clone());
                    }
                    let _ = live_tx.send(request).await;
                }
                None => {
                    reader.abort();
                    return None;
                }
            },
            finished = &mut reader => {
                return match finished {
                    Ok(Some(reason)) => Some(reason),
                    // The client's inbox was dropped: it is not waiting for
                    // another player, it is leaving.
                    Ok(None) => None,
                    Err(error) => Some(format!("the reader task died: {error}")),
                };
            }
        }
    }
}

fn ipc_error(error: std::io::Error) -> crate::error::AppError {
    AppError::Ipc(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::engine::PlaybackState;
    use crate::config::Config;

    fn handle() -> LocalHandle {
        LocalHandle::new(Player::new_headless(&Config::default()))
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        std::fs::canonicalize(format!("tests/fixtures/{name}")).expect("fixture file")
    }

    /// The point of the local handle: a request is answered before it returns,
    /// so a test can assert on the very next line.
    #[tokio::test]
    async fn dispatch_answers_synchronously() {
        let mut handle = handle();
        let events = handle.dispatch(Request::Play {
            path: fixture("test.wav"),
        });

        let snapshot = events
            .iter()
            .find_map(|e| match e {
                Event::Snapshot(s) => Some(s.clone()),
                _ => None,
            })
            .expect("a snapshot accompanies every command");
        assert_eq!(snapshot.title, "test");
        assert!(snapshot.status.is_active());
    }

    /// A local player has no clock of its own to drain, and saying so is part
    /// of the contract: the client pumps `poll` every tick regardless of which
    /// handle it holds.
    #[test]
    fn poll_is_always_empty() {
        let mut handle = handle();
        assert!(handle.poll().is_empty());
        handle.dispatch(Request::SetVolume { volume: 0.5 });
        assert!(handle.poll().is_empty());
    }

    /// `tick` is what turns a finished track into the next one.
    #[tokio::test]
    async fn tick_reports_the_state_after_the_engine_moves() {
        let mut handle = handle();
        handle.dispatch(Request::Play {
            path: fixture("test.wav"),
        });

        let events = handle.tick();
        let snapshot = events
            .iter()
            .find_map(|e| match e {
                Event::Snapshot(s) => Some(s.clone()),
                _ => None,
            })
            .expect("a tick always reports the state");
        assert!(snapshot.status.is_active());
        assert!(snapshot.duration_secs > 0.0);
    }

    #[tokio::test]
    async fn detach_stops_the_player() {
        let mut handle = handle();
        handle.dispatch(Request::Play {
            path: fixture("test.wav"),
        });
        handle.detach().expect("detach");
        assert_eq!(
            handle.local().expect("in-process").state().status,
            PlaybackState::Stopped
        );
    }

    /// A socket handle has no player to hand out — the whole point of the
    /// trait. `LocalHandle` is the only implementation that answers `Some`.
    #[tokio::test]
    async fn only_a_local_handle_has_a_player_to_show() {
        let handle = handle();
        assert!(handle.local().is_some());
    }

    // ── The socket ──
    //
    // `DaemonHandle::connect` needs a socket and nothing else, so the daemon's
    // half of a connection is a few lines written straight onto the listener.
    // Under `cfg(test)` the socket path lives in a per-process temp directory,
    // never the developer's `$XDG_RUNTIME_DIR`.

    /// One socket path for the whole test binary, and several tests below bind
    /// it — so they take turns. Without this they race at the filesystem level:
    /// one test's `bind` after another's `remove_file` is an `ENOENT` for a
    /// client that was about to dial, and the failure reads as a reconnect bug.
    static SOCKET: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// A listener where [`DaemonHandle::connect`] will look for one. Hold the
    /// guard for as long as the socket is needed.
    async fn listener() -> (
        tokio::sync::MutexGuard<'static, ()>,
        tokio::net::UnixListener,
    ) {
        let guard = SOCKET.lock().await;
        let path = crate::paths::socket_path();
        crate::paths::ensure_runtime_dir().expect("runtime dir");
        let _ = std::fs::remove_file(&path);
        (guard, tokio::net::UnixListener::bind(&path).expect("bind"))
    }

    /// The next event, with a deadline: "nothing arrived" must fail the test
    /// rather than hang it.
    async fn event(handle: &mut DaemonHandle) -> Event {
        tokio::time::timeout(Duration::from_secs(5), handle.next_event())
            .await
            .expect("the client stopped reading")
            .expect("an event")
    }

    /// Answer a connection with a welcome and a greeting written as one chunk,
    /// the way the daemon does when it accepts and greets before the client has
    /// had its first read — and then hang up.
    async fn greet_in_one_chunk(listener: tokio::net::UnixListener) {
        use tokio::io::AsyncWriteExt;

        let (stream, _) = listener.accept().await.expect("accept");
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);
        let hello: Option<Request> = crate::ipc::read_message_async(&mut reader)
            .await
            .expect("read");
        assert!(matches!(hello, Some(Request::Hello { .. })), "{hello:?}");

        let mut chunk = Vec::new();
        for message in [
            Event::Welcome {
                proto: PROTOCOL_VERSION,
                version: "test".into(),
                pid: 1,
            },
            Event::Snapshot(Box::default()),
            Event::Queue {
                rev: 0,
                tracks: Vec::new(),
            },
        ] {
            crate::ipc::write_message_async(&mut chunk, &message)
                .await
                .expect("encode");
        }
        write_half.write_all(&chunk).await.expect("write");
        write_half.flush().await.expect("flush");

        // Stay connected just long enough for the client to have read the
        // chunk, so a client that swallowed the greeting is waiting on an
        // empty socket when this goes rather than racing the write.
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    /// The welcome and the greeting share the socket, and a `BufReader`
    /// answering one `read_line` takes everything the socket has — so a
    /// handshake that borrowed a reader and threw it away swallows the
    /// snapshot and the queue behind the welcome. They are not recoverable:
    /// the queue is only pushed when it *changes*, so a client that lost this
    /// one waits forever for a queue it was already sent.
    #[tokio::test]
    async fn the_greeting_behind_the_welcome_is_not_swallowed() {
        let (_socket, listener) = listener().await;
        let greeter = tokio::spawn(greet_in_one_chunk(listener));

        let mut handle = DaemonHandle::connect().await.expect("connect");
        assert!(
            matches!(event(&mut handle).await, Event::Snapshot(_)),
            "the snapshot the daemon sent with the welcome"
        );
        assert!(
            matches!(event(&mut handle).await, Event::Queue { .. }),
            "and the queue behind it"
        );

        greeter.await.expect("the fake daemon finished");
    }

    /// A daemon that hangs up between the connect and the welcome is not a
    /// protocol error to decode — it is a player that went away, and the
    /// client should say so rather than panic on an unexpected `None`.
    #[tokio::test]
    async fn a_daemon_that_leaves_before_saying_hello_is_reported() {
        let (_socket, listener) = listener().await;
        let greeter = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            // Read the hello before leaving. Closing on an unread hello is a
            // connection reset, which is a different complaint from the one
            // this test is about — and the one a daemon that exits mid-handshake
            // actually makes is this: it reads, then goes.
            let (read_half, _write_half) = stream.into_split();
            let mut reader = BufReader::new(read_half);
            let hello: Option<Request> = crate::ipc::read_message_async(&mut reader)
                .await
                .expect("read");
            assert!(matches!(hello, Some(Request::Hello { .. })), "{hello:?}");
        });

        let error = DaemonHandle::connect()
            .await
            .err()
            .expect("a daemon that hung up is not a connection");
        assert!(
            error.to_string().contains("closed the connection"),
            "unhelpful error: {error}"
        );
        greeter.await.expect("the fake daemon finished");
    }

    /// A daemon that dies *after* the greeting ends a one-shot verb's session
    /// with a `Bye` — the verb has nothing left to wait for, and saying so is
    /// what stops `tmper status` from hanging on a socket nobody will answer on.
    #[tokio::test]
    async fn a_socket_that_dies_mid_session_ends_a_one_shot_verb() {
        let (_socket, listener) = listener().await;
        let player = fake_player(listener);

        let mut handle = DaemonHandle::connect().await.expect("connect");
        assert!(matches!(event(&mut handle).await, Event::Snapshot(_)));
        player.kill();

        assert!(
            matches!(event(&mut handle).await, Event::Notice { .. }),
            "the reason, in words"
        );
        assert!(
            matches!(event(&mut handle).await, Event::Bye),
            "and then the end of the session"
        );
    }

    // ── Surviving the player ──
    //
    // The TUI's handle is the one that heals. Everything below drives the real
    // supervisor over a real socket; the far end is a stand-in player that can
    // be told to die and comes back when the client dials again.

    /// A stand-in player: greets every connection the way the daemon does, and
    /// then stays up until it is told to go.
    ///
    /// The far end has to be killable on cue and restartable, which a real
    /// daemon is neither — and starting one from a test is refused outright
    /// (see [`crate::daemon::spawn_detached`]).
    struct FakePlayer {
        /// One value per live connection: send to end it.
        deaths: mpsc::UnboundedSender<()>,
        /// `true` means the player is gone for good — no more connections.
        vanish: watch::Sender<bool>,
        /// Everything any connection asked for, `Hello` aside.
        requests: mpsc::UnboundedReceiver<Request>,
        /// How many times a client has connected.
        connections: watch::Receiver<usize>,
    }

    impl FakePlayer {
        /// End the current connection, as a crash would: no `Bye`, just a
        /// socket that stops answering. The next dial gets a new one.
        fn kill(&self) {
            self.deaths.send(()).expect("a connection to kill");
        }

        /// Take the player away for good: the current connection dies and
        /// nothing accepts the next dial.
        fn vanish(&self) {
            self.kill();
            let _ = self.vanish.send_replace(true);
        }

        /// Wait for a request to arrive on the wire.
        async fn next_request(&mut self) -> Request {
            tokio::time::timeout(Duration::from_secs(5), self.requests.recv())
                .await
                .expect("the client said something")
                .expect("a request")
        }
    }

    fn fake_player(listener: tokio::net::UnixListener) -> FakePlayer {
        let (deaths, mut dying) = mpsc::unbounded_channel::<()>();
        let (vanish, mut vanishing) = watch::channel(false);
        let (requests_tx, requests) = mpsc::unbounded_channel::<Request>();
        let (count_tx, connections) = watch::channel(0usize);

        tokio::spawn(async move {
            let mut count = 0usize;
            loop {
                let accepted = tokio::select! {
                    // Cancel-safe, and the only *listening* arm: `accept` is
                    // not, so it must never be the one a select drops — it is
                    // not dropped, it is simply not polled again.
                    _ = vanishing.changed() => return,
                    accepted = listener.accept() => accepted,
                };
                let Ok((stream, _)) = accepted else { return };

                let (read_half, mut write_half) = stream.into_split();
                let mut reader = BufReader::new(read_half);
                let hello: Option<Request> = match crate::ipc::read_message_async(&mut reader).await
                {
                    Ok(hello) => hello,
                    Err(_) => continue, // a client that left before saying hello
                };
                if !matches!(hello, Some(Request::Hello { .. })) {
                    continue;
                }
                count += 1;
                let _ = count_tx.send_replace(count);

                let mut greeted = true;
                for message in [
                    Event::Welcome {
                        proto: PROTOCOL_VERSION,
                        version: "test".into(),
                        pid: 1,
                    },
                    Event::Snapshot(Box::default()),
                ] {
                    if crate::ipc::write_message_async(&mut write_half, &message)
                        .await
                        .is_err()
                    {
                        greeted = false;
                        break;
                    }
                }
                if !greeted {
                    continue;
                }

                // Everything this connection asks for, until the test ends it.
                loop {
                    tokio::select! {
                        _ = dying.recv() => break,
                        _ = vanishing.changed() => return,
                        message = crate::ipc::read_message_async::<_, Request>(&mut reader) => {
                            match message {
                                Ok(Some(request)) => {
                                    if requests_tx.send(request).is_err() {
                                        return;
                                    }
                                }
                                // The client hung up, or the socket broke.
                                _ => break,
                            }
                        }
                    }
                }
                // Both halves, so the client sees a socket that is finished
                // rather than one that is merely quiet.
                drop(write_half);
                drop(reader);
            }
        });

        FakePlayer {
            deaths,
            vanish,
            requests,
            connections,
        }
    }

    /// Wait for the handle to report a state, with a deadline.
    async fn wait_for(handle: &DaemonHandle, want: impl Fn(&Connection) -> bool) -> Connection {
        let mut watch = handle.connection.clone();
        let current = handle.connection();
        if want(&current) {
            return current;
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                watch
                    .changed()
                    .await
                    .expect("the supervisor is still running");
                let now = watch.borrow().clone();
                if want(&now) {
                    return now;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the handle never reported the state the test waited for"))
    }

    /// The whole point of the supervisor: a player that dies does not take the
    /// client with it. The banner goes up, the client dials again on its own,
    /// and the banner comes down when a player answers.
    #[tokio::test]
    async fn a_dead_player_puts_up_a_banner_and_comes_back() {
        let (_socket, listener) = listener().await;
        let player = fake_player(listener);
        let mut handle = DaemonHandle::connect_or_spawn()
            .await
            .expect("the first connection");

        assert_eq!(handle.connection(), Connection::Live);
        assert!(matches!(event(&mut handle).await, Event::Snapshot(_)));

        player.kill();
        let lost = wait_for(&handle, |c| matches!(c, Connection::Lost { .. })).await;
        let Connection::Lost { reason } = lost else {
            unreachable!("asked for a lost connection")
        };
        assert!(
            !reason.is_empty(),
            "a banner that says only 'lost' is not worth showing"
        );

        // The client is told, and is *not* told to leave. This is the line
        // between a player that died and a player that said goodbye.
        let events = handle.poll();
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::Notice {
                    level: NoticeLevel::Warn,
                    ..
                }
            )),
            "the loss is announced: {events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(e, Event::Bye)),
            "and the client is not sent away: {events:?}"
        );

        wait_for(&handle, |c| matches!(*c, Connection::Live)).await;
        assert!(
            handle
                .poll()
                .iter()
                .any(|e| matches!(e, Event::Snapshot(_))),
            "the new connection is greeted like any other"
        );
        assert_eq!(
            *player.connections.borrow(),
            2,
            "the client dialed again by itself"
        );
    }

    #[tokio::test]
    async fn reconnect_restores_spectrum_subscription_and_settings() {
        let (_socket, listener) = listener().await;
        let mut player = fake_player(listener);
        let mut handle = DaemonHandle::connect_or_spawn().await.unwrap();
        let params = Request::SetFftParams {
            num_bars: 48,
            smoothing: 0.7,
        };
        let subscription = Request::SubscribeVisualizer { on: true };
        handle.dispatch(params.clone());
        handle.dispatch(subscription.clone());
        assert_eq!(player.next_request().await, params);
        assert_eq!(player.next_request().await, subscription);
        player.kill();
        assert_eq!(player.next_request().await, params);
        assert_eq!(player.next_request().await, subscription);
    }

    /// A command given while the player is away is not swallowed: it waits in
    /// the outbox and goes out with the next connection. The user pressed a key
    /// meaning what it meant when they pressed it.
    #[tokio::test]
    async fn a_command_given_during_an_outage_arrives_after_it() {
        let (_socket, listener) = listener().await;
        let mut player = fake_player(listener);
        let mut handle = DaemonHandle::connect_or_spawn()
            .await
            .expect("the first connection");
        assert!(matches!(event(&mut handle).await, Event::Snapshot(_)));

        player.kill();
        wait_for(&handle, |c| matches!(c, Connection::Lost { .. })).await;

        handle.dispatch(Request::SetVolume { volume: 0.25 });

        wait_for(&handle, |c| matches!(*c, Connection::Live)).await;
        assert_eq!(
            player.next_request().await,
            Request::SetVolume { volume: 0.25 }
        );
    }

    /// A player that never comes back leaves the client alive and usable — the
    /// banner is not a dialog, and there is nothing for the user to dismiss.
    ///
    /// This is also the test that drives the supervisor past a *failed* dial,
    /// which is where it would try to start a player: `spawn_detached` refuses
    /// under `cfg(test)`, so the retry loop is what is left, and the client is
    /// still there to offer a `q`.
    #[tokio::test]
    async fn a_client_whose_player_never_returns_stays_up() {
        let (_socket, listener) = listener().await;
        let player = fake_player(listener);
        let handle = DaemonHandle::connect_or_spawn()
            .await
            .expect("the first connection");

        player.vanish();
        wait_for(&handle, |c| matches!(c, Connection::Lost { .. })).await;

        // Several attempts' worth of time, with nothing on the far end.
        tokio::time::sleep(Duration::from_millis(
            runtime::DAEMON_RECONNECT_RETRY_MS * 3,
        ))
        .await;
        assert!(
            matches!(handle.connection(), Connection::Lost { .. }),
            "still lost, and still saying so"
        );
        assert_eq!(
            *player.connections.borrow(),
            1,
            "and it never pretended to reconnect"
        );
    }
}
