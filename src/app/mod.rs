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

            // Always render half-blocks as fallback underneath any native
            // graphics overlay (Kitty/SIXEL). If the terminal supports the
            // protocol the native image covers the blocks; if not the user
            // still sees the half-block fallback.
            self.ui_state.native_cover_active = false;

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

    /// Render cover art via chafa subprocess using symbol (half-block) output.
    ///
    /// Runs chafa once when the cover changes, parses the ANSI-colored output
    /// into ratatui Lines, and stores them in UiState for the normal render
    /// pipeline — no terminal graphics protocol needed.
    fn render_cover_via_chafa(&mut self) {
        use std::io::Write;

        if !self.chafa_available {
            return;
        }

        // Cover hidden — clear cache
        if !self.ui_state.show_cover_art {
            if self.ui_state.cover_chafa_lines.is_some() {
                self.ui_state.cover_chafa_lines = None;
                self.last_cover_gen_chafa = 0;
            }
            return;
        }

        // Regenerate parsed lines when cover art changes
        let gen = self.ui_state.cover_gen.get();
        if self.ui_state.cover_chafa_lines.is_some() && gen == self.last_cover_gen_chafa {
            return; // Cache is up to date
        }
        self.last_cover_gen_chafa = gen;

        let (w_char, h_char) = {
            let r = self.ui_state.cover_rect.get();
            (r.2, r.3)
        };
        if w_char == 0 || h_char == 0 {
            return;
        }

        let cover = match self.ui_state.cover_art {
            Some(ref c) => c.clone(),
            None => {
                self.ui_state.cover_chafa_lines = None;
                return;
            }
        };

        // Run chafa in symbols mode — outputs ANSI-colored half-block text
        let output = match std::process::Command::new("chafa")
            .arg("-f")
            .arg("symbols")
            .arg("--symbols")
            .arg("half")
            .arg("-c")
            .arg("full")
            .arg("-s")
            .arg(format!("{}x{}", w_char, h_char))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(mut child) => {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(&cover);
                }
                drop(child.stdin.take());
                match child.wait_with_output() {
                    Ok(out) => {
                        if !out.stderr.is_empty() {
                            let msg = String::from_utf8_lossy(&out.stderr);
                            tracing::warn!("chafa stderr: {msg}");
                        }
                        out.stdout
                    }
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
            tracing::warn!("chafa produced empty output");
            self.ui_state.cover_chafa_lines = None;
            return;
        }

        // Parse ANSI-colored output into Vec<Line<'static>>
        let lines = parse_chafa_symbols_output(&output, w_char);
        let count = lines.len();
        self.ui_state.cover_chafa_lines = Some(lines);
        tracing::info!("chafa rendered: {count} lines (area {}x{})", w_char, h_char);
    }
}

/// Check if the `chafa` binary is available and working.
fn which_chafa() -> bool {
    let ok = std::process::Command::new("chafa")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    tracing::info!("chafa detected: {ok}");
    ok
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

/// Parse chafa `-f symbols` ANSI output into ratatui text Lines.
///
/// chafa outputs lines like:
///   \x1b[0m\x1b[38;2;R;G;B;48;2;R;G;Bm▄\x1b[0m...
/// with cursor-control wrappers and newlines between rows.
fn parse_chafa_symbols_output(data: &[u8], cols: u16) -> Vec<ratatui::text::Line<'static>> {
    use ratatui::style::Color;
    use ratatui::text::{Line, Span};

    let text = match std::str::from_utf8(data) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    // Split into lines, find the content between cursor sequences
    let mut raw_lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "\x1b[?25l" || trimmed == "\x1b[?25h" {
            continue;
        }
        raw_lines.push(trimmed);
    }

    let mut result: Vec<Line<'static>> = Vec::with_capacity(raw_lines.len());

    for raw in &raw_lines {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let bytes = raw.as_bytes();
        let mut i = 0;

        // Current colors
        let mut fg: Option<(u8, u8, u8)> = None;
        let mut bg: Option<(u8, u8, u8)> = None;

        while i < bytes.len() {
            if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                // Parse ANSI escape sequence: ESC [ params m
                i += 2; // skip ESC[
                let start = i;
                while i < bytes.len() && bytes[i] != b'm' {
                    i += 1;
                }
                if i < bytes.len() {
                    let params = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
                    i += 1; // skip 'm'

                    // Reset
                    if params == "0" || params.is_empty() {
                        fg = None;
                        bg = None;
                        continue;
                    }

                    // Parse SGR parameters
                    let parts: Vec<&str> = params.split(';').collect();
                    let mut pi = 0;
                    while pi < parts.len() {
                        match parts[pi] {
                            "0" | "" => {
                                fg = None;
                                bg = None;
                            }
                            "38" if pi + 4 < parts.len() && parts[pi + 1] == "2" => {
                                let r = parts[pi + 2].parse().unwrap_or(0);
                                let g = parts[pi + 3].parse().unwrap_or(0);
                                let b = parts[pi + 4].parse().unwrap_or(0);
                                fg = Some((r, g, b));
                                pi += 4;
                            }
                            "48" if pi + 4 < parts.len() && parts[pi + 1] == "2" => {
                                let r = parts[pi + 2].parse().unwrap_or(0);
                                let g = parts[pi + 3].parse().unwrap_or(0);
                                let b = parts[pi + 4].parse().unwrap_or(0);
                                bg = Some((r, g, b));
                                pi += 4;
                            }
                            _ => {}
                        }
                        pi += 1;
                    }
                }
            } else {
                // Regular character — collect all contiguous non-ESC chars
                let start = i;
                while i < bytes.len() && !(bytes[i] == 0x1b) {
                    i += 1;
                }
                let chunk = &bytes[start..i];
                if let Ok(chunk_str) = std::str::from_utf8(chunk) {
                    let mut style = ratatui::style::Style::default();
                    if let Some((r, g, b)) = fg {
                        style = style.fg(Color::Rgb(r, g, b));
                    }
                    if let Some((r, g, b)) = bg {
                        style = style.bg(Color::Rgb(r, g, b));
                    }
                    spans.push(Span::styled(chunk_str.to_string(), style));
                }
            }
        }

        result.push(Line::from(spans));
    }

    // Pad short rows to maintain alignment
    let target_len = cols as usize;
    for line in &mut result {
        let current_len = line.width();
        if current_len < target_len {
            line.push_span(Span::raw(" ".repeat(target_len - current_len)));
        }
    }

    result
}
