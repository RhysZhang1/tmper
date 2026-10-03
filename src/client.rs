//! The one-shot side of being a client.
//!
//! `tmper status`, `tmper next`, `tmper quit` and the rest are clients in
//! exactly the way the TUI is — they speak the same protocol to the same
//! socket, and go through the same [`DaemonHandle`]. They just have no
//! terminal to draw on and nothing to do afterwards, so they send one message,
//! wait for the answer, print it, and leave.
//!
//! They deliberately do **not** start a daemon. `tmper pause` on a machine
//! where nothing is playing is a mistake worth reporting, not a reason to
//! launch a player so that it can be paused.

use std::fmt::Write as _;

use crate::app::handle::{DaemonHandle, PlayerHandle};
use crate::cli::Command;
use crate::error::{AppError, AppResult};
use crate::ipc::proto::{Event, QueueTrack, Request, StateSnapshot};
use crate::ui::format_duration;

/// Run a control verb against a running player.
pub async fn run_control(command: Command) -> AppResult<()> {
    // Unwrapped, deliberately: the error already names the socket it tried and
    // the reason it was refused, which is the whole of what a user needs to
    // know here.
    let mut handle = DaemonHandle::connect().await?;

    if matches!(command, Command::Status) {
        let (state, queue) = greeting(&mut handle).await?;
        print!("{}", status_report(&state, &queue));
        return Ok(());
    }

    let request = request_for(&command).expect("only control verbs get this far");

    // The greeting snapshot describes the state *before* this command. Reading
    // it first means the snapshot that follows is unambiguously the answer,
    // with no sleeping and no guessing — the daemon handles the join before
    // any request, and one socket preserves that order.
    let _greeting = next_snapshot(&mut handle).await?;
    handle.dispatch(request);
    next_snapshot(&mut handle).await?;
    Ok(())
}

/// The message a verb sends, or `None` for the one that asks instead of tells.
fn request_for(command: &Command) -> Option<Request> {
    match command {
        Command::Play { file: None } => Some(Request::Resume),
        Command::Pause => Some(Request::Pause),
        Command::Next => Some(Request::Next),
        Command::Prev => Some(Request::Prev),
        Command::Stop => Some(Request::Stop),
        Command::Volume { percent } => Some(Request::SetVolume {
            volume: f32::from((*percent).min(100)) / 100.0,
        }),
        Command::Quit => Some(Request::Shutdown),
        Command::Status | Command::Play { file: Some(_) } | Command::Daemon => None,
    }
}

/// The state and the queue a fresh connection is greeted with.
async fn greeting(handle: &mut DaemonHandle) -> AppResult<(StateSnapshot, Vec<QueueTrack>)> {
    let mut state = None;
    let mut queue = None;
    while state.is_none() || queue.is_none() {
        match handle.next_event().await {
            Some(Event::Snapshot(snapshot)) => state = Some(*snapshot),
            Some(Event::Queue { tracks, .. }) => queue = Some(tracks),
            // A spectrum frame can only arrive here if something else is
            // subscribed to the same daemon; it is not this client's business.
            Some(_) => continue,
            None => return Err(AppError::Ipc("the player closed the connection".into()).into()),
        }
    }
    Ok((state.expect("set above"), queue.expect("set above")))
}

/// The next full state, skipping anything else on the wire.
async fn next_snapshot(handle: &mut DaemonHandle) -> AppResult<StateSnapshot> {
    loop {
        match handle.next_event().await {
            Some(Event::Snapshot(snapshot)) => return Ok(*snapshot),
            Some(Event::Bye) | None => {
                return Err(AppError::Ipc("the player closed the connection".into()).into());
            }
            Some(_) => continue,
        }
    }
}

/// A status report meant to be read in a terminal and grepped in a script.
///
/// One `key   value` pair per line, no colour, no box: `tmper status` is the
/// thing you reach for when the TUI will not tell you what is wrong, so it
/// should work when the terminal is a pipe.
fn status_report(state: &StateSnapshot, queue: &[QueueTrack]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "status    {}", status_word(state));
    if let Some(path) = &state.path {
        let _ = writeln!(out, "track     {} — {}", state.artist, state.title);
        let _ = writeln!(out, "file      {}", path.display());
        if !state.album.is_empty() {
            let _ = writeln!(out, "album     {}", describe_album(state));
        }
        let _ = writeln!(
            out,
            "position  {} / {}",
            format_duration(state.position_secs),
            format_duration(state.duration_secs)
        );
    }
    let _ = writeln!(out, "volume    {}%", (state.volume * 100.0).round());
    let _ = writeln!(out, "repeat    {}", repeat_word(state));
    match state.playing_index {
        Some(index) => {
            let _ = writeln!(
                out,
                "queue     {} tracks, playing #{}",
                queue.len(),
                index + 1
            );
        }
        None => {
            let _ = writeln!(out, "queue     {} tracks", queue.len());
        }
    }
    out
}

fn status_word(state: &StateSnapshot) -> String {
    use crate::audio::engine::PlaybackState;
    match &state.status {
        PlaybackState::Stopped => "stopped".into(),
        PlaybackState::Loading => "loading".into(),
        PlaybackState::Playing => "playing".into(),
        PlaybackState::Paused => "paused".into(),
        PlaybackState::Seeking => "seeking".into(),
        PlaybackState::Finished => "finished".into(),
        PlaybackState::Failed(why) => format!("failed: {why}"),
    }
}

fn repeat_word(state: &StateSnapshot) -> &'static str {
    use crate::ipc::proto::RepeatMode;
    match state.repeat {
        RepeatMode::Sequential => "sequential",
        RepeatMode::Shuffle => "shuffle",
        RepeatMode::SingleTrack => "single track",
    }
}

fn describe_album(state: &StateSnapshot) -> String {
    let mut album = state.album.clone();
    if !state.year.is_empty() {
        let _ = write!(album, " ({})", state.year);
    }
    if !state.codec.is_empty() {
        let _ = write!(album, " [{}]", state.codec);
    }
    album
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::engine::PlaybackState;
    use crate::ipc::proto::RepeatMode;
    use std::path::PathBuf;

    fn playing() -> StateSnapshot {
        StateSnapshot {
            status: PlaybackState::Playing,
            path: Some(PathBuf::from("/music/一首歌.flac")),
            title: "A Song".into(),
            artist: "An Artist".into(),
            album: "An Album".into(),
            genre: "Rock".into(),
            year: "2026".into(),
            codec: "flac".into(),
            duration_secs: 245.0,
            position_secs: 65.0,
            volume: 0.8,
            repeat: RepeatMode::Shuffle,
            playing_index: Some(2),
            queue_rev: 4,
            lyrics_offset_ms: 0,
        }
    }

    #[test]
    fn a_playing_track_is_reported_in_full() {
        let report = status_report(
            &playing(),
            &[QueueTrack {
                path: PathBuf::from("/music/a.flac"),
                title: "A".into(),
                artist: "An Artist".into(),
                duration_secs: 1.0,
            }],
        );

        assert!(report.contains("status    playing"), "{report}");
        assert!(report.contains("track     An Artist — A Song"), "{report}");
        assert!(report.contains("file      /music/一首歌.flac"), "{report}");
        assert!(
            report.contains("album     An Album (2026) [flac]"),
            "{report}"
        );
        assert!(report.contains("position  01:05 / 04:05"), "{report}");
        assert!(report.contains("volume    80%"), "{report}");
        assert!(report.contains("repeat    shuffle"), "{report}");
        // One-based for the human: the index is the daemon's, the number is
        // the reader's.
        assert!(
            report.contains("queue     1 tracks, playing #3"),
            "{report}"
        );
    }

    /// With nothing loaded there is no track to describe, and inventing empty
    /// fields for one would be worse than saying nothing.
    #[test]
    fn an_empty_player_reports_only_what_it_knows() {
        let state = StateSnapshot {
            status: PlaybackState::Stopped,
            path: None,
            volume: 0.5,
            repeat: RepeatMode::Sequential,
            ..Default::default()
        };
        let report = status_report(&state, &[]);

        assert!(report.contains("status    stopped"), "{report}");
        // The `track` *line* is what must be absent; the word appears in
        // "0 tracks" either way.
        assert!(!report.contains("track  "), "{report}");
        assert!(!report.contains("position"), "{report}");
        assert!(report.contains("queue     0 tracks"), "{report}");
        assert!(!report.contains("playing #"), "{report}");
    }

    /// A failure is the reason someone runs `tmper status` at all, so it is
    /// reported verbatim rather than collapsed to "stopped".
    #[test]
    fn a_failure_says_what_failed() {
        let state = StateSnapshot {
            status: PlaybackState::Failed("no such file".into()),
            ..Default::default()
        };
        assert!(
            status_report(&state, &[]).contains("status    failed: no such file"),
            "the decoder's own words are the useful part"
        );
    }

    /// Every control verb maps to exactly one message, and the verbs that open
    /// the TUI map to none.
    #[test]
    fn every_control_verb_has_a_request() {
        assert_eq!(
            request_for(&Command::Play { file: None }),
            Some(Request::Resume)
        );
        assert_eq!(request_for(&Command::Pause), Some(Request::Pause));
        assert_eq!(request_for(&Command::Next), Some(Request::Next));
        assert_eq!(request_for(&Command::Prev), Some(Request::Prev));
        assert_eq!(request_for(&Command::Stop), Some(Request::Stop));
        assert_eq!(request_for(&Command::Quit), Some(Request::Shutdown));
        assert_eq!(
            request_for(&Command::Volume { percent: 40 }),
            Some(Request::SetVolume { volume: 0.4 })
        );
        assert_eq!(request_for(&Command::Status), None);
        assert_eq!(
            request_for(&Command::Play {
                file: Some(PathBuf::from("/music/a.flac"))
            }),
            None
        );
        assert_eq!(request_for(&Command::Daemon), None);
    }

    /// `tmper volume 250` is a typo, not a request for 2.5x.
    #[test]
    fn an_out_of_range_volume_is_clamped_rather_than_obeyed() {
        assert_eq!(
            request_for(&Command::Volume { percent: 250 }),
            Some(Request::SetVolume { volume: 1.0 })
        );
        assert_eq!(
            request_for(&Command::Volume { percent: 0 }),
            Some(Request::SetVolume { volume: 0.0 })
        );
    }

    /// The verb classification and the request mapping have to agree. A verb
    /// that claims to control a player but carries no message would connect
    /// and then do nothing — except `status`, which asks instead of telling
    /// and is answered before this point.
    #[test]
    fn a_control_verb_always_has_something_to_say() {
        let controls = [
            Command::Play { file: None },
            Command::Pause,
            Command::Next,
            Command::Prev,
            Command::Stop,
            Command::Volume { percent: 50 },
            Command::Status,
            Command::Quit,
        ];
        for command in &controls {
            assert!(
                command.controls_a_running_player(),
                "{command:?} was classified as opening the TUI"
            );
            if !matches!(command, Command::Status) {
                assert!(
                    request_for(command).is_some(),
                    "{command:?} has no message to send"
                );
            }
        }

        // And the other direction: the verbs that open the TUI must not be
        // mistaken for messages, or `tmper play song.flac` would silently
        // become `tmper play`.
        for command in [
            Command::Play {
                file: Some(PathBuf::from("/music/a.flac")),
            },
            Command::Daemon,
        ] {
            assert!(!command.controls_a_running_player());
        }
    }
}
