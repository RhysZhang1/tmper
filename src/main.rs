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
/// The two halves of tmper are two processes writing two logs: a daemon that
/// has been logging for hours must not have its history — including the lines
/// explaining why it is about to die — wiped by an unrelated `tmper` starting
/// up. `tmper-daemon.log` is the daemon's alone, and [`open_log`] makes even
/// that boundary non-destructive.
fn init_logging(is_daemon: bool) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let name = if is_daemon {
        "tmper-daemon.log"
    } else {
        "tmper.log"
    };
    // Before the log is opened, not after: on a fresh install nothing has made
    // the state directory yet — `main` does, a few lines further down — and a
    // log that cannot be created falls back to `/dev/null` in silence. The
    // first run of either half is then the one run with nothing to read.
    let _ = std::fs::create_dir_all(state_dir());

    let log_file = open_log(&state_dir().join(name))
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

/// Open a log for appending, rotating it aside first if it has grown too large.
///
/// **Append, never truncate.** Several clients are alive at once — a TUI, any
/// number of one-shot verbs, and possibly a second TUI — and the `File::create`
/// that used to be here meant whichever started last erased the history of the
/// ones already running. The README tells users to grep this file for the
/// graphics verdict, and a `tmper status` in between wiped exactly that line.
/// It is the same bug the client/daemon log split fixed, one level down: a
/// short-lived process destroying a long-lived one's record.
///
/// Appending removes the only thing that bounded the file, so
/// [`LOG_MAX_BYTES`](constants::runtime::LOG_MAX_BYTES) puts the bound back —
/// by *renaming*, not truncating. A client that is still running holds the old
/// inode open and keeps writing into `tmper.log.1`, so rotation cannot destroy
/// a live log either; truncating here would resurrect the bug being fixed.
fn open_log(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let over_cap = std::fs::metadata(path)
        .map(|meta| meta.len() > constants::runtime::LOG_MAX_BYTES)
        .unwrap_or(false);

    if over_cap {
        let mut rotated = path.as_os_str().to_os_string();
        rotated.push(".1");
        // Best effort. A failed rename is not worth refusing to log over: the
        // file is simply bigger than the cap until the next process starts.
        let _ = std::fs::rename(path, rotated);
    }

    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A path of this test process's own, so the three cases below can run in
    /// parallel without rotating each other's files.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tmper-logging-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn opening_the_log_keeps_what_was_already_in_it() {
        // The bug this guards: `tmper status` starts with File::create and
        // erased the graphics verdict a running TUI had already written.
        let path = scratch("append.log");
        std::fs::write(&path, "the TUI was here\n").unwrap();

        let mut file = open_log(&path).unwrap();
        writeln!(file, "tmper starting...").unwrap();
        drop(file);

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(
            contents.contains("the TUI was here"),
            "the earlier client's history was erased: {contents:?}"
        );
        assert!(
            contents.contains("tmper starting..."),
            "nothing was appended: {contents:?}"
        );
    }

    #[test]
    fn a_log_past_the_cap_is_rotated_rather_than_truncated() {
        let path = scratch("rotate.log");
        let rotated = scratch("rotate.log.1");
        let _ = std::fs::remove_file(&rotated);
        let oversized = "x".repeat(constants::runtime::LOG_MAX_BYTES as usize + 1);
        std::fs::write(&path, &oversized).unwrap();

        let _ = open_log(&path).unwrap();

        assert!(rotated.exists(), "an oversized log was not rotated aside");
        // Renamed, not truncated: every byte is still there to be read.
        assert_eq!(
            std::fs::read_to_string(&rotated).unwrap().len(),
            oversized.len()
        );
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    }

    #[test]
    fn a_log_within_the_cap_is_left_alone() {
        let path = scratch("small.log");
        let rotated = scratch("small.log.1");
        let _ = std::fs::remove_file(&rotated);
        std::fs::write(&path, "small\n").unwrap();

        let _ = open_log(&path).unwrap();

        assert!(
            !rotated.exists(),
            "rotation ran on a file that was within the cap"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "small\n");
    }
}
