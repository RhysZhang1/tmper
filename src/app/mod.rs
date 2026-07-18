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
use crate::constants::runtime;
use crate::event::AppEvent;
use crate::input::handler::KeyHandler;
use crate::input::keymap::{self, KeyBindings};
use crate::library::database::LibraryDb;
use crate::ui::cover::{CoverParams, CoverRenderer};
use crate::ui::{self, PlayerCore, UiState};
use serde::{Deserialize, Serialize};

pub(crate) mod handlers;
pub(crate) mod persistence;
pub(crate) mod playback;

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
    cover_renderer: CoverRenderer,
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
                player: PlayerCore {
                    show_cover_art: config.ui.show_cover_art,
                    ..Default::default()
                },
                ..Default::default()
            },
            engine,
            should_quit: false,
            search_mode: false,
            key_handler: KeyHandler::new(runtime::KEY_TIMEOUT_MS, quit_key),
            key_bindings,
            library_db,
            fft_cancel_tx: None,
            fft_data: Arc::new(Mutex::new(Vec::new())),
            cover_renderer: CoverRenderer::new(),
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

            // Clear SIXEL ghost when leaving player view.
            // Uses terminal.clear() so ratatui resets its internal buffer
            // and the next draw() sends ALL cells (not just diffs).
            if self.cover_renderer.needs_clear() {
                let _ = terminal.clear();
                self.cover_renderer.clear_done();
            }

            if let Err(e) = terminal.draw(|f| ui::render(f, &self.ui_state)) {
                tracing::error!("Render error: {e}");
            }

            // Kitty graphics: native pixel rendering (Kitty-compatible terminals)
            let cover_params = CoverParams {
                active_view: self.ui_state.view.active_view,
                show_help: self.ui_state.view.show_help,
                command_mode: self.ui_state.command_mode,
                show_cover_art: self.ui_state.player.show_cover_art,
                cover_gen: self.ui_state.player.cover_gen.get(),
                cover_art: self.ui_state.player.cover_art.clone(),
                cover_rect: self.ui_state.cover_rect.get(),
            };
            self.cover_renderer.render_kitty(&cover_params);
            // SIXEL graphics via chafa subprocess (Konsole, etc.)
            self.cover_renderer.render_chafa(&cover_params);
            // Drain spurious stdin events caused by terminal escape-sequence
            // interference (bytes from Kitty/SIXEL output misinterpreted as
            // key events). Uses a short non-blocking poll — real keypresses
            // from a human won't arrive within this micro-window.
            self.drain_spurious_events();
        }

        // Stop FFT
        self.fft_cancel_tx = None;
        disable_raw_mode().ok();
        execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();

        Ok(())
    }

    /// Drain any stdin events that arrived during cover art rendering.
    /// These are typically spurious — bytes from terminal escape sequences
    /// (Kitty/SIXEL) misinterpreted as keyboard input by the PTY layer.
    /// A very short poll timeout (2ms) ensures real human keypresses
    /// (inter-event gap > 50ms) are never discarded.
    fn drain_spurious_events(&self) {
        use crossterm::event;
        use std::time::Duration;
        // Drain up to 16 events within a 2ms window each
        for _ in 0..16 {
            if !event::poll(Duration::from_millis(2)).unwrap_or(false) {
                break;
            }
            let _ = event::read();
        }
    }
}
