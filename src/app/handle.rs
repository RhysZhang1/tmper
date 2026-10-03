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

use crate::error::AppResult;
use crate::ipc::proto::{Event, Request};
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
/// Used by tests, and by any future `tmper --standalone`; the shipped TUI uses
/// the socket handle so the music outlives it.
pub struct LocalHandle {
    player: Player,
}

impl LocalHandle {
    pub fn new(player: Player) -> Self {
        Self { player }
    }
}

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

    #[cfg(test)]
    fn local(&self) -> Option<&Player> {
        Some(&self.player)
    }

    #[cfg(test)]
    fn local_mut(&mut self) -> Option<&mut Player> {
        Some(&mut self.player)
    }
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
}
