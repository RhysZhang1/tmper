mod app;
mod cli;
mod client;
mod config;
mod constants;
mod daemon;
mod error;
mod event;
mod ipc;
mod paths;
mod player;
mod playlist;

mod audio;
mod input;
mod library;
mod lyrics;
mod metadata;
mod ui;
mod visualizer;

use clap::Parser;
use cli::{Cli, Command};
use config::Config;
use paths::state_dir;

#[tokio::main]
async fn main() -> error::AppResult<()> {
    // Parsed before anything else so that `--help` does not create a log file
    // and a config directory on the way to printing itself.
    let cli = Cli::parse();
    init_logging(matches!(cli.command, Some(Command::Daemon)));

    let state = state_dir();
    let _ = std::fs::create_dir_all(&state);
    let _ = std::fs::create_dir_all(paths::data_dir());
    let _ = std::fs::create_dir_all(paths::config_dir());

    tracing::info!("tmper starting...");

    paths::migrate_legacy_layout();
    Config::ensure_config_file();

    // Three ways to be tmper, decided entirely by the verb. The daemon is
    // dispatched first because it is the one that must not touch the config
    // twice or open a second engine; the control verbs next, because they talk
    // to a player that is already running and have no use for a `Config` at
    // all.
    match cli.command {
        Some(Command::Daemon) => return daemon::run().await,
        Some(command) if command.controls_a_running_player() => {
            return client::run_control(command).await;
        }
        _ => {}
    }

    let config = Config::load_or_default();
    let mut app = app::App::connect(&config).await?;
    app.run(cli).await
}

/// Send this process's logs to a file of its own.
///
/// The two halves of tmper are two processes writing two logs, and the reason
/// is `File::create`: the client truncates `tmper.log` at every start, so a
/// daemon that had been logging there for hours would have its history wiped
/// by the next `tmper` — including the lines explaining why it is about to
/// die. `tmper-daemon.log` is the daemon's alone.
fn init_logging(is_daemon: bool) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let name = if is_daemon {
        "tmper-daemon.log"
    } else {
        "tmper.log"
    };
    // Before the `File::create`, not after: on a fresh install nothing has
    // made the state directory yet — `main` does, a few lines further down —
    // and a log that cannot be created falls back to `/dev/null` in silence.
    // The first run of either half is then the one run with nothing to read.
    let _ = std::fs::create_dir_all(state_dir());

    let log_file = std::fs::File::create(state_dir().join(name))
        .unwrap_or_else(|_| std::fs::File::create("/dev/null").expect("/dev/null"));

    // `info` by default, not "everything". A subscriber with no filter logs
    // TRACE, and the dependencies are chatty there: `lofty` narrates every
    // tag block it parses and tokio every poller registration. That was merely
    // untidy while the TUI was the only process — it truncates its log at
    // every start — but the daemon runs for days and gets its log truncated
    // only when it starts, so a library scan would leave megabytes of other
    // people's debug output behind. Everything the README tells a user to grep
    // for (`cell size`, the graphics decision) is logged at `info`, so this
    // keeps that working; `RUST_LOG=debug` brings the noise back on demand.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::sync::Mutex::new(log_file))
        .with_target(false);

    if std::env::var("RUST_LOG").is_ok() {
        let stderr_layer = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(false);
        tracing_subscriber::registry()
            .with(filter)
            .with(file_layer)
            .with(stderr_layer)
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(file_layer)
            .init();
    }
}
