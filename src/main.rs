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
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("termusic starting...");

    let cli = Cli::parse();
    let config = Config::load_or_default();

    let mut app = app::App::new(&config)?;
    app.run(cli).await
}
