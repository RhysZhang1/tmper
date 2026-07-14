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
use image::GenericImageView;
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
    last_cover_art_version: u64,
    viuer_rendered: bool,
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
            last_cover_art_version: 0,
            viuer_rendered: false,
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

            // viuer: render cover art via Kitty terminal graphics protocol
            self.render_cover_via_viuer();
        }

        // Stop FFT
        self.fft_cancel_tx = None;
        disable_raw_mode().ok();
        execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();

        Ok(())
    }

    /// Render cover art via viuer (Kitty/iTerm2 graphics protocol).
    /// Only renders when cover art changes; Kitty images persist across frames.
    fn render_cover_via_viuer(&mut self) {
        if !self.ui_state.show_cover_art {
            // Clear any previously rendered Kitty image
            if self.viuer_rendered {
                use std::io::Write;
                // Kitty graphics protocol: delete all placed images
                let _ = write!(std::io::stdout(), "\x1b_Ga=d,d=I\x1b\\");
                let _ = std::io::stdout().flush();
                self.viuer_rendered = false;
            }
            self.last_cover_art_version = self.ui_state.cover_art_version.get();
            return;
        }

        let version = self.ui_state.cover_art_version.get();
        if version == self.last_cover_art_version {
            return;
        }
        self.last_cover_art_version = version;

        let cover = match self.ui_state.cover_art {
            Some(ref c) => c.clone(),
            None => return,
        };

        let (x, y, w, h) = self.ui_state.cover_art_area.get();
        if w == 0 || h == 0 {
            return;
        }

        // Detect terminal image protocol support (env vars only, no terminal query)
        // Protocol detection — env vars only, no stdin query (avoids conflict
        // with crossterm's event reader in raw mode).
        let supports_kitty = viuer::get_kitty_support() != viuer::KittySupport::None;
        let supports_iterm = viuer::is_iterm_supported();
        // Sixel is supported by foot, WezTerm, Konsole, etc.
        // Detection is TERM-based (no reliable env var, but viuer's sixel
        // support is always-on when compiled with the feature).
        let supports_sixel = std::env::var("TERM")
            .map(|v| {
                v.contains("foot")
                    || v.contains("wezterm")
                    || v.contains("contour")
                    || v.contains("yaft")
                    || v.contains("mlterm")
            })
            .unwrap_or(false);
        if !supports_kitty && !supports_iterm && !supports_sixel {
            return; // No protocol support → rely on ratatui block chars
        }

        // Load image and calculate dimensions that preserve aspect ratio
        match image::load_from_memory(&cover) {
            Ok(img) => {
                let (img_w, img_h) = img.dimensions();
                // Approximate pixel dimensions of the cell area
                let cell_px_w = w as u32 * 10;
                let cell_px_h = h as u32 * 20;

                // Scale to fit within cell area while maintaining aspect ratio
                let scale = (cell_px_w as f64 / img_w as f64)
                    .min(cell_px_h as f64 / img_h as f64)
                    .min(1.0);
                let out_w = (img_w as f64 * scale).round() as u32;
                let out_h = (img_h as f64 * scale).round() as u32;

                let mut config = viuer::Config {
                    x,
                    y: y as i16,
                    width: Some(out_w),
                    height: Some(out_h),
                    absolute_offset: true,
                    restore_cursor: false,
                    use_kitty: supports_kitty,
                    use_iterm: supports_iterm,
                    use_sixel: supports_sixel,
                    transparent: false,
                    ..Default::default()
                };
                // On non-Kitty/non-iTerm terminals, Sixel is our only hope
                if !supports_kitty && !supports_iterm {
                    config.use_kitty = false;
                    config.use_iterm = false;
                }

                if viuer::print(&img, &config).is_ok() {
                    self.viuer_rendered = true;
                }
            }
            Err(e) => {
                tracing::warn!("Failed to decode cover art for viuer: {e}");
            }
        }
    }
}
