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
use base64::Engine;
use image::GenericImageView;
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
    kitty_rendered: bool,
    last_cover_gen: u64,
    chafa_available: bool,
    last_cover_gen_chafa: u64,
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
            kitty_rendered: false,
            last_cover_gen: 0,
            chafa_available: which_chafa(),
            last_cover_gen_chafa: 0,
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

            // Kitty graphics: native pixel rendering (Kitty-compatible terminals)
            self.render_cover_via_kitty();
            // SIXEL graphics via chafa subprocess (Konsole, etc.)
            self.render_cover_via_chafa();
        }

        // Stop FFT
        self.fft_cancel_tx = None;
        disable_raw_mode().ok();
        execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();

        Ok(())
    }

    /// Output cover art via native Kitty graphics protocol.
    /// Uses the image crate to load/resize, then outputs the Kitty escape
    /// sequence with `p=x,y` pixel positioning — no cursor movement,
    /// no crossterm buffer corruption.
    fn render_cover_via_kitty(&mut self) {
        use std::io::Write;

        // Toggle handling: clear Kitty image when cover display is off
        if !self.ui_state.show_cover_art {
            if self.kitty_rendered {
                let _ = write!(std::io::stdout(), "\x1b_Ga=d,d=I\x1b\\");
                let _ = std::io::stdout().flush();
                self.kitty_rendered = false;
            }
            return;
        }

        // Only on terminals that support Kitty graphics protocol
        if !is_kitty_graphics_compatible() {
            self.kitty_rendered = false;
            return;
        }

        let gen = self.ui_state.cover_gen.get();
        if gen == self.last_cover_gen {
            return; // Image unchanged, Kitty image persists on screen
        }
        self.last_cover_gen = gen;

        let cover = match self.ui_state.cover_art {
            Some(ref c) => c.clone(),
            None => return,
        };

        let (x_chars, y_chars, w_chars, h_chars) = self.ui_state.cover_rect.get();
        if w_chars == 0 || h_chars == 0 {
            return;
        }

        // Load, resize, and output via Kitty protocol
        match image::load_from_memory(&cover) {
            Ok(img) => {
                let (img_w, img_h) = img.dimensions();
                // Approximate cell size: 10 px wide × 20 px tall (Kitty default)
                let cell_w = 10u32;
                let cell_h = 20u32;
                let area_w = w_chars as u32 * cell_w;
                let area_h = h_chars as u32 * cell_h;

                // Scale image to fit area while preserving aspect ratio
                let scale = (area_w as f64 / img_w as f64)
                    .min(area_h as f64 / img_h as f64)
                    .min(1.0);
                let out_w = (img_w as f64 * scale).round().max(1.0) as u32;
                let out_h = (img_h as f64 * scale).round().max(1.0) as u32;

                let resized = img.resize_exact(out_w, out_h, image::imageops::FilterType::Lanczos3);
                let rgb = resized.to_rgb8();
                let raw = rgb.into_raw();

                // Base64 encode the pixel data
                let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);

                // Pixel position (p=x,y) from character cell position
                let px = x_chars as u32 * cell_w;
                let py = y_chars as u32 * cell_h;

                // Output Kitty protocol — NO cursor movement, NO crossterm
                // The escape sequence handles both position and sizing.
                // Split into chunks (Kitty recommends ~4K payload per chunk).
                let max_chunk = 4096usize;
                for (i, chunk) in b64.as_bytes().chunks(max_chunk).enumerate() {
                    let chunk_str = std::str::from_utf8(chunk)
                        .expect("base64 output is always valid ASCII");
                    let more = if (i + 1) * max_chunk < b64.len() { 1 } else { 0 };
                    let _ = write!(
                        std::io::stdout(),
                        "\x1b_Ga=T,f=100,s={out_w},v={out_h},c=3,p={px},{py},m={more};{chunk_str}\x1b\\",
                    );
                }
                let _ = std::io::stdout().flush();
                self.kitty_rendered = true;
            }
            Err(e) => {
                tracing::warn!("Failed to decode cover for Kitty protocol: {e}");
            }
        }
    }

    /// Render cover art via chafa subprocess using SIXEL graphics protocol.
    /// Works on Konsole (KDE Plasma) and other terminals with SIXEL support.
    fn render_cover_via_chafa(&mut self) {
        use std::io::Write;

        if !self.chafa_available {
            return;
        }
        if !self.ui_state.show_cover_art {
            if self.last_cover_gen_chafa != 0 {
                // Cover hidden — clear SIXEL by redrawing area with spaces
                let (x, y, w, h) = self.ui_state.cover_rect.get();
                if w > 0 && h > 0 {
                    let clear: String = std::iter::repeat_n(
                        " ".repeat(w as usize),
                        h as usize,
                    )
                    .collect::<Vec<_>>()
                        .join("\r\n");
                    let _ = write!(std::io::stdout(), "\x1b[{};{}H{}", y + 1, x + 1, clear);
                    let _ = std::io::stdout().flush();
                }
                self.last_cover_gen_chafa = 0;
            }
            return;
        }

        let gen = self.ui_state.cover_gen.get();
        if gen == self.last_cover_gen_chafa {
            return; // Already rendered
        }
        self.last_cover_gen_chafa = gen;

        let cover = match self.ui_state.cover_art {
            Some(ref c) => c.clone(), // Arc clone — cheap
            None => return,
        };

        let (x_char, y_char, _w_char, _h_char) = self.ui_state.cover_rect.get();
        if _w_char == 0 || _h_char == 0 {
            return;
        }

        // Spawn chafa with SIXEL output, capture the data
        let output = match std::process::Command::new("chafa")
            .arg("-f")
            .arg("sixels")
            .arg("-c")
            .arg("full")
            .arg("-s")
            .arg(format!("{}x{}", _w_char, _h_char))
            .arg("--no-cache")
            .arg("/dev/stdin")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(&cover);
                }
                drop(child.stdin.take());
                match child.wait_with_output() {
                    Ok(out) => out.stdout,
                    Err(e) => {
                        tracing::warn!("chafa wait failed: {e}");
                        return;
                    }
                }
            }
            Err(e) => {
                tracing::warn!("chafa spawn failed: {e}");
                self.chafa_available = false;
                return;
            }
        };

        if output.is_empty() {
            return;
        }

        // Strip cursor-control sequences that chafa prepends/ appends:
        //   \x1b[?25l (hide cursor), \x1b[?80l (no wrap), \x1b[?8452l (konsole-specific)
        //   \x1b[?25h (show cursor)
        let mut sixel_start = 0;
        while sixel_start < output.len() && output[sixel_start] != 0x1b {
            sixel_start += 1;
        }
        // Find the SIXEL DCS: ESC P
        while sixel_start < output.len() - 1
            && !(output[sixel_start] == 0x1b && output[sixel_start + 1] == b'P')
        {
            sixel_start += 1;
        }
        if sixel_start >= output.len() {
            return;
        }

        // Find the ST (string terminator): ESC \
        let mut sixel_end = output.len();
        while sixel_end > sixel_start + 1
            && !(output[sixel_end - 2] == 0x1b && output[sixel_end - 1] == b'\\')
        {
            sixel_end -= 1;
        }
        if sixel_end <= sixel_start {
            return;
        }

        let sixel_data = &output[sixel_start..sixel_end];

        let _ = write!(
            std::io::stdout(),
            "\x1b[{};{}H",
            y_char + 1,
            x_char + 1
        );
        let _ = std::io::stdout().write_all(sixel_data);
        let _ = std::io::stdout().flush();
    }
}

/// Check if the `chafa` binary is available on PATH.
fn which_chafa() -> bool {
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let candidate = dir.join("chafa");
                // On Unix, check if it's executable (not a directory)
                candidate.is_file()
            }).then_some(())
        })
        .is_some()
}

/// Returns true if the terminal supports the Kitty graphics protocol.
/// Checks known env vars - no stdin query needed.
fn is_kitty_graphics_compatible() -> bool {
    // Native Kitty terminal
    if std::env::var("KITTY_WINDOW_ID").is_ok() {
        return true;
    }
    // WezTerm: sets TERM_PROGRAM=WezTerm, supports full Kitty protocol
    if matches!(std::env::var("TERM_PROGRAM").as_deref(), Ok("WezTerm")) {
        return true;
    }
    // Ghostty: supports Kitty protocol
    if std::env::var("GHOSTTY_RESOURCES_DIR").is_ok() {
        return true;
    }
    false
}
