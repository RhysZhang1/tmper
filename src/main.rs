mod app;
mod cli;
mod config;
mod error;
mod event;
mod playlist;

mod audio;
mod input;
mod library;
mod lyrics;
mod metadata;
mod ui;
mod visualizer;

use clap::Parser;
use cli::Cli;
use config::Config;

#[tokio::main]
async fn main() -> error::AppResult<()> {
    // Write logs to a file so they don't interfere with the TUI
    let log_dir = dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("termusic");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_file = std::fs::File::create(log_dir.join("termusic.log"))
        .unwrap_or_else(|_| std::fs::File::create("/dev/null").unwrap());

    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(log_file))
        .with_target(false)
        .init();

    tracing::info!("termusic starting...");

    let cli = Cli::parse();
    let config = Config::load_or_default();

    let mut app = app::App::new(&config)?;
    app.run(cli).await
}
