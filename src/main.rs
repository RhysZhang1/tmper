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

    let env_filter = std::env::var("RUST_LOG").unwrap_or_default();
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::sync::Mutex::new(log_file))
        .with_target(false);

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    if env_filter.is_empty() {
        tracing_subscriber::registry().with(file_layer).init();
    } else {
        let stderr_layer = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(false);
        tracing_subscriber::registry()
            .with(file_layer)
            .with(stderr_layer)
            .init();
    }

    tracing::info!("tmper starting...");

    let cli = Cli::parse();
    Config::ensure_config_file();
    let config = Config::load_or_default();

    let mut app = app::App::new(&config)?;
    app.run(cli).await
}
