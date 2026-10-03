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
use tokio::sync::mpsc;

use crate::constants::runtime;
use crate::error::{AppError, AppResult};
use crate::ipc::proto::{Event, NoticeLevel, Request, PROTOCOL_VERSION};
#[cfg(test)]
use crate::player::Player;

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
}

impl DaemonHandle {
    /// Connect to a running daemon. Fails if there is none — use
    /// [`DaemonHandle::connect_or_spawn`] to start one.
    pub async fn connect() -> AppResult<Self> {
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

        let (outbox, outbox_rx) = mpsc::channel::<Request>(runtime::DAEMON_CLIENT_QUEUE);
        tokio::spawn(write_requests(write_half, outbox_rx));

        // Bounded, and deliberately the same bound the daemon applies on its
        // side: a client that stops draining its inbox applies backpressure
        // all the way to the daemon's mailbox, which is what makes "this
        // client is too far behind" a judgement both ends agree on.
        let (inbox_tx, inbox) = mpsc::channel::<Event>(runtime::DAEMON_CLIENT_QUEUE);
        tokio::spawn(read_events(reader, inbox_tx));

        Ok(Self { outbox, inbox })
    }

    /// Connect to a running daemon, starting one if there is none.
    ///
    /// Bounded on purpose: a player that cannot come up — no sound card, a
    /// socket it cannot bind — has to surface as an error the user can read,
    /// not as a TUI that hangs on a blank screen.
    pub async fn connect_or_spawn() -> AppResult<Self> {
        if let Ok(handle) = Self::connect().await {
            return Ok(handle);
        }
        crate::daemon::spawn_detached()?;

        let deadline =
            std::time::Instant::now() + Duration::from_millis(runtime::DAEMON_START_TIMEOUT_MS);
        let mut last = None;
        while std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(runtime::DAEMON_CONNECT_RETRY_MS)).await;
            match Self::connect().await {
                Ok(handle) => return Ok(handle),
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

/// Read events until the daemon is gone, and report that as an event.
///
/// A client whose player has died has nothing left to control: the `Bye` tells
/// [`crate::app::App`] to leave, and the notice in front of it says why, which
/// is the difference between a TUI that vanished and one that explained
/// itself.
///
/// The reader is the handshake's, passed on rather than rebuilt — see
/// [`DaemonHandle::connect`] for what a second `BufReader` over one socket
/// costs.
async fn read_events(
    mut reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    inbox: mpsc::Sender<Event>,
) {
    loop {
        match crate::ipc::read_message_async::<_, Event>(&mut reader).await {
            Ok(Some(event)) => {
                if inbox.send(event).await.is_err() {
                    break;
                }
            }
            Ok(None) => {
                let _ = inbox
                    .send(Event::Notice {
                        level: NoticeLevel::Error,
                        message: "The player has gone; restart tmper to reconnect".into(),
                    })
                    .await;
                let _ = inbox.send(Event::Bye).await;
                break;
            }
            Err(error) => {
                tracing::warn!("Lost the player connection: {error}");
                let _ = inbox
                    .send(Event::Notice {
                        level: NoticeLevel::Error,
                        message: format!("Lost the player connection: {error}"),
                    })
                    .await;
                let _ = inbox.send(Event::Bye).await;
                break;
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

    /// A listener where [`DaemonHandle::connect`] will look for one.
    fn listener() -> tokio::net::UnixListener {
        let path = crate::paths::socket_path();
        crate::paths::ensure_runtime_dir().expect("runtime dir");
        let _ = std::fs::remove_file(&path);
        tokio::net::UnixListener::bind(&path).expect("bind")
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
        let listener = listener();
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
        let listener = listener();
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
}
