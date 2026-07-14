use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{Event as CrosstermEvent, EventStream};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures_util::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::audio::engine::AudioEngine;
use crate::cli::{Cli, Command};
use crate::config::Config;
use crate::event::AppEvent;
use crate::input::handler::KeyHandler;
use crate::input::keymap::{self, KeyBindings};
use crate::library::database::LibraryDb;
use crate::ui::{self, UiState};
use serde::{Deserialize, Serialize};

pub(crate) mod handlers;
pub(crate) mod persistence;

pub struct App {
    config: Config,
    ui_state: UiState,
    engine: AudioEngine,
    should_quit: bool,
    search_mode: bool,
    key_handler: KeyHandler,
    key_bindings: KeyBindings,
    fft_cancel_tx: Option<tokio::sync::watch::Sender<()>>,
    library_db: LibraryDb,
    fft_data: Arc<Mutex<Vec<f32>>>,
}

#[derive(Serialize, Deserialize)]
struct SavedState {
    volume: f32,
    repeat_mode: crate::ui::RepeatMode,
    lyrics_offset_ms: i64,
    last_track_path: Option<String>,
}

impl App {
    pub fn new(config: &Config) -> crate::error::AppResult<Self> {
        let engine = AudioEngine::new()?;
        let library_db = LibraryDb::open(&crate::paths::data_dir().join("library.db"))
            .unwrap_or_else(|_| LibraryDb::open_memory().expect("in-memory db"));
        let key_bindings = KeyBindings::load();
        let quit_key = keymap::parse_key_str(&key_bindings.quit);
        Ok(Self {
            config: config.clone(),
            ui_state: UiState {
                volume: config.playback.default_volume,
                show_cover_art: config.ui.show_cover_art,
                ..Default::default()
            },
            engine,
            should_quit: false,
            search_mode: false,
            key_handler: KeyHandler::new(200, quit_key),
            key_bindings,
            library_db,
            fft_cancel_tx: None,
            fft_data: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub async fn run(&mut self, cli: Cli) -> crate::error::AppResult<()> {
        enable_raw_mode().map_err(|e| {
            crate::error::AppError::Config(format!("Failed to enable raw mode: {e}"))
        })?;

        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen).map_err(|e| {
            crate::error::AppError::Config(format!("Failed to enter alternate screen: {e}"))
        })?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).map_err(|e| {
            crate::error::AppError::Config(format!("Failed to create terminal: {e}"))
        })?;

        self.load_library_paths();
        self.load_playlists();
        if let Some(Command::Play { file }) = cli.command {
            self.load_and_play(&file);
            self.start_fft();
        }

        let mut reader = EventStream::new();
        let mut tick_interval = tokio::time::interval(Duration::from_millis(
            (1000 / self.config.visualizer.frame_rate.max(1)) as u64,
        ));

        loop {
            tokio::select! {
                crossterm_event = reader.next() => {
                    match crossterm_event {
                        Some(Ok(event)) => {
                            match event {
                                CrosstermEvent::Key(key) => {
                                    // Bypass KeyHandler delay in insert/typing modes
                                    let needs_bypass = matches!(self.ui_state.playlist_state.insert_mode, crate::ui::views::playlist_view::InsertMode::Typing(_));
                                    if needs_bypass {
                                        self.handle_event(crate::event::AppEvent::Key(key));
                                    } else if let Some(app_event) = self.key_handler.process(key) {
                                        self.handle_event(app_event);
                                    }
                                }
                                CrosstermEvent::Resize(_, _) => {}
                                _ => {}
                            }
                        }
                        Some(Err(e)) => {
                            tracing::error!("Crossterm event error: {e}");
                        }
                        None => break,
                    }
                }
                _ = tick_interval.tick() => {
                    self.handle_event(AppEvent::Tick);
                }
            }

            if self.should_quit {
                break;
            }

            if let Err(e) = terminal.draw(|f| ui::render(f, &self.ui_state)) {
                tracing::error!("Render error: {e}");
            }

        }

        // Stop FFT
        self.fft_cancel_tx = None;
        disable_raw_mode().ok();
        execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();

        Ok(())
    }

}
