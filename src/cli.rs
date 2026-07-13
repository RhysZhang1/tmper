use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "tmper", about = "A terminal-native music player")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Play an audio file
    Play {
        /// Path to the audio file
        file: PathBuf,
    },
}
