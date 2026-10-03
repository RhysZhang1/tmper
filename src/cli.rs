//! Command line verbs.
//!
//! Two kinds, and the split matters: a verb either **opens the TUI** (bare
//! `tmper`, or `tmper play <file>`) or **talks to a player that is already
//! running** (`tmper pause`, `tmper status`, …). The second kind never starts
//! a daemon — `tmper pause` on a machine where nothing is playing should say
//! so, not quietly start a player in order to pause it.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "tmper", about = "A terminal-native music player")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Play an audio file, opening the player on it
    ///
    /// Without a file, resumes the player where it left off.
    Play {
        /// Path to the audio file. Omit to resume playback.
        file: Option<PathBuf>,
    },
    /// Pause playback, keeping the track loaded
    Pause,
    /// Skip to the next track
    Next,
    /// Go back to the previous track
    Prev,
    /// Stop playback and stay stopped
    Stop,
    /// Set the volume, as a percentage
    Volume {
        /// 0 to 100
        percent: u8,
    },
    /// Print what the player is doing
    Status,
    /// Stop the player and exit it
    Quit,
    /// Run the player in the foreground
    ///
    /// Normally started on demand by the TUI; this is for watching it directly
    /// when something is wrong.
    Daemon,
}

impl Command {
    /// Whether this verb is a message to a running player, rather than a
    /// request to open the TUI.
    pub fn controls_a_running_player(&self) -> bool {
        // `play` is the one verb that is both: with a file it opens the TUI on
        // that file, without one it means "carry on" — which is what every
        // other player's `play` means, and what a media key sends.
        match self {
            Command::Daemon => false,
            Command::Play { file } => file.is_none(),
            Command::Pause
            | Command::Next
            | Command::Prev
            | Command::Stop
            | Command::Volume { .. }
            | Command::Status
            | Command::Quit => true,
        }
    }
}
