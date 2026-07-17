//! Cover art rendering — outputs pixel data directly to the terminal
//! outside of ratatui's buffer, using terminal-native graphics protocols.
//!
//! Rendering priority chain:
//!   Kitty protocol (Kitty / WezTerm / Ghostty)
//!   → SIXEL via chafa subprocess (Konsole Plasma 6+)
//!   → half-block characters (fallback in player_view.rs)

use std::io::Write;

use base64::Engine;
use image::GenericImageView;

use crate::ui::UiState;

/// Manages cover-art rendering state and terminal protocol output.
///
/// All fields are private — interaction goes through the public methods.
pub struct CoverRenderer {
    kitty_rendered: bool,
    last_cover_gen: u64,
    chafa_available: bool,
    last_cover_gen_chafa: u64,
    chafa_sixel_cache: Option<Vec<u8>>,
    /// Set to `true` when leaving the player view — the event loop calls
    /// `terminal.clear()` before the next ratatui draw so the internal diff
    /// buffer covers all cells and overwrites any SIXEL residue.
    clear_pending: bool,
}

impl CoverRenderer {
    pub fn new() -> Self {
        let chafa_available = which_chafa();
        Self {
            kitty_rendered: false,
            last_cover_gen: 0,
            chafa_available,
            last_cover_gen_chafa: 0,
            chafa_sixel_cache: None,
            clear_pending: false,
        }
    }

    // ── Public interface called from App ──

    /// Returns `true` when a `terminal.clear()` is needed before the next
    /// ratatui draw (SIXEL cleanup on view switch).
    pub fn needs_clear(&self) -> bool {
        self.clear_pending
    }

    /// Mark the clear as handled so it only fires once.
    pub fn clear_done(&mut self) {
        self.clear_pending = false;
    }

    /// Call after every ratatui draw when on the player view (key 1).
    /// Attempts Kitty protocol first; falls back to chafa SIXEL.
    pub fn render_kitty(&mut self, state: &UiState) {
        // Only render cover on the player view
        if state.active_view != crate::ui::ViewMode::Player {
            self.kitty_rendered = false;
            return;
        }

        // Toggle handling: clear Kitty image when cover display is off
        if !state.show_cover_art {
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

        let gen = state.cover_gen.get();
        if gen == self.last_cover_gen {
            return; // Image unchanged, Kitty image persists on screen
        }
        self.last_cover_gen = gen;

        let cover = match state.cover_art {
            Some(ref c) => c.clone(),
            None => return,
        };

        let (x_chars, y_chars, w_chars, h_chars) = state.cover_rect.get();
        if w_chars == 0 || h_chars == 0 {
            return;
        }

        // Load, resize, and output via Kitty protocol
        match image::load_from_memory(&cover[..]) {
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
                let max_chunk = crate::constants::runtime::KITTY_CHUNK_SIZE;
                for (i, chunk) in b64.as_bytes().chunks(max_chunk).enumerate() {
                    let chunk_str =
                        std::str::from_utf8(chunk).expect("base64 output is always valid ASCII");
                    let more = if (i + 1) * max_chunk < b64.len() {
                        1
                    } else {
                        0
                    };
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

    /// Render cover art via chafa subprocess using SIXEL protocol.
    /// Only shows on the player view (key 1).
    ///
    /// Caches the FULL chafa output (including Konsole-specific setup
    /// sequences) and re-sends every frame so the image survives redraws.
    pub fn render_chafa(&mut self, state: &UiState) {
        // Only render cover on the player view
        if state.active_view != crate::ui::ViewMode::Player {
            if self.chafa_sixel_cache.is_some() {
                self.clear_pending = true;
            }
            self.chafa_sixel_cache = None;
            self.last_cover_gen_chafa = 0;
            return;
        }
        // Hide cover when overlays (help, command input) are on top
        if state.show_help || state.command_mode {
            if self.chafa_sixel_cache.is_some() {
                self.clear_pending = true;
            }
            self.chafa_sixel_cache = None;
            self.last_cover_gen_chafa = 0;
            return;
        }

        if !self.chafa_available {
            return;
        }

        let (x_char, y_char, w_char, h_char) = state.cover_rect.get();
        if w_char == 0 || h_char == 0 {
            return;
        }

        // Cover hidden — clear cache
        if !state.show_cover_art {
            if self.chafa_sixel_cache.is_some() {
                self.clear_pending = true;
            }
            self.chafa_sixel_cache = None;
            self.last_cover_gen_chafa = 0;
            return;
        }

        // Regenerate cache when cover art changes
        let gen = state.cover_gen.get();
        if self.chafa_sixel_cache.is_none() || gen != self.last_cover_gen_chafa {
            self.last_cover_gen_chafa = gen;

            let cover = match state.cover_art {
                Some(ref c) => c.clone(),
                None => {
                    self.chafa_sixel_cache = None;
                    return;
                }
            };

            let output = match std::process::Command::new("chafa")
                .arg("-f")
                .arg("sixels")
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
                tracing::warn!("chafa SIXEL output is empty — terminal may not support it");
                self.chafa_sixel_cache = None;
                return;
            }

            // Cache the FULL output including any terminal setup sequences
            self.chafa_sixel_cache = Some(output.clone());
            tracing::info!(
                "chafa SIXEL: {} bytes cached (area {}x{})",
                output.len(),
                w_char,
                h_char
            );
        }

        // Re-send FULL cached data every frame
        if let Some(ref data) = self.chafa_sixel_cache {
            let _ = write!(std::io::stdout(), "\x1b[{};{}H", y_char + 1, x_char + 1);
            let _ = std::io::stdout().write_all(data);
            let _ = write!(std::io::stdout(), "\x1b[?25l");
            let _ = std::io::stdout().flush();
        }
    }
}

// ── Helpers ──

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
/// Checks known env vars — no stdin query needed.
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
