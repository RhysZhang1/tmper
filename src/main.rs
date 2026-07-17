mod app;
mod cli;
mod config;
mod constants;
mod error;
mod event;
mod paths;
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
use paths::data_dir;

#[tokio::main]
async fn main() -> error::AppResult<()> {
    let data = data_dir();
    let _ = std::fs::create_dir_all(&data);
    let log_file = std::fs::File::create(data.join("tmper.log"))
        .unwrap_or_else(|_| std::fs::File::create("/dev/null").unwrap());

    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(log_file))
        .with_target(false)
        .init();

    tracing::info!("tmper starting...");

    let cli = Cli::parse();
    let config = Config::load_or_default();

    let mut app = app::App::new(&config)?;
    app.run(cli).await
}
