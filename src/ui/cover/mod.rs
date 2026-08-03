//! Cover art rendering — outputs pixel data directly to the terminal
//! outside of ratatui's buffer, using terminal-native graphics protocols.
//!
//! Rendering priority chain:
//!   Kitty protocol (Kitty / WezTerm / Ghostty)
//!   → SIXEL via chafa subprocess (Konsole Plasma 6+)
//!   → half-block characters (fallback in player_view.rs)
//!
//! Design: SIXEL/Kitty images are a *persistent graphics layer* on modern
//! terminals — once sent they survive subsequent ratatui redraws. Each
//! protocol payload is therefore sent ONCE per change (new cover, new area,
//! terminal resize, returning to the player view) and left on screen.
//! Re-sending every frame was the historical root cause of spurious stdin
//! events ("phantom keys") and UI lag on Konsole — see
//! `progress/2026-08-01-cover-rollback.md`. Sending once also makes the old
//! defense stack (frame suppression, blanket input guard) unnecessary.

use std::io::Write;
use std::sync::Arc;

use base64::Engine;
use image::GenericImageView;

use crate::ui::ViewMode;

/// Read-only parameters for cover art rendering.
/// Extracted from `UiState` so the cover renderer only sees what it needs.
pub struct CoverParams {
    pub active_view: ViewMode,
    pub show_help: bool,
    pub command_mode: bool,
    pub show_cover_art: bool,
    pub cover_gen: u64,
    pub cover_art: Option<Arc<Vec<u8>>>,
    pub cover_rect: (u16, u16, u16, u16),
}

/// Converts cover image bytes into a terminal graphics payload (e.g. SIXEL).
/// Abstracted so tests can stub the chafa subprocess.
///
/// `Send` keeps `CoverRenderer` (and thus `App`) `Send` for the multi-threaded
/// tokio runtime.
trait SixelEncoder: Send {
    /// Returns the payload to write to the terminal, or a human-readable error.
    fn encode(&self, cover: &[u8], cols: u16, rows: u16) -> Result<Vec<u8>, String>;
}

/// Default encoder: runs `chafa` as a subprocess (Konsole SIXEL).
struct ChafaEncoder;

impl SixelEncoder for ChafaEncoder {
    fn encode(&self, cover: &[u8], cols: u16, rows: u16) -> Result<Vec<u8>, String> {
        // `--probe off` is critical: by default chafa probes the controlling
        // terminal for capabilities (incl. background color via OSC 10/11
        // queries through `/dev/tty`) and waits up to 5s for the response.
        // The response lands on the same PTY as tmper's stdin, so it is read
        // as phantom key events, and the 5s wait blocks the main thread.
        // We already know the terminal supports SIXEL — no probing needed.
        let mut child = std::process::Command::new("chafa")
            .arg("--probe")
            .arg("off")
            .arg("-f")
            .arg("sixels")
            .arg("-c")
            .arg("full")
            .arg("-s")
            .arg(format!("{cols}x{rows}"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("chafa spawn failed: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(cover);
        }
        let out = child
            .wait_with_output()
            .map_err(|e| format!("chafa wait failed: {e}"))?;
        if !out.stderr.is_empty() {
            tracing::warn!("chafa stderr: {}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(out.stdout)
    }
}

/// Manages cover-art rendering state and terminal protocol output.
///
/// All fields are private — interaction goes through the public methods.
/// Protocol output goes through an injectable writer so tests can capture it.
pub struct CoverRenderer {
    out: Box<dyn Write + Send>,
    encoder: Box<dyn SixelEncoder>,
    /// Whether the `chafa` binary is usable (checked once at construction).
    chafa_available: bool,
    /// Cached SIXEL payload — re-sent only when the cover or area changes.
    chafa_sixel_cache: Option<Vec<u8>>,
    /// Cover identity the cached SIXEL payload was rendered for.
    last_chafa_gen: u64,
    /// Last area (x,y,w,h) the SIXEL payload was rendered for.
    last_chafa_rect: Option<(u16, u16, u16, u16)>,
    /// Sticky: encoding produced no usable payload. Stops per-frame chafa
    /// spawns until the cover or area changes (a terminal that doesn't
    /// support SIXEL won't start supporting it mid-session).
    chafa_failed: bool,
    /// Set to `true` when leaving the player view — the event loop calls
    /// `terminal.clear()` before the next ratatui draw so the internal diff
    /// buffer covers all cells and overwrites any SIXEL residue.
    clear_pending: bool,
    /// Whether a Kitty image is currently displayed on screen (needed to
    /// send the clear sequence when the cover is hidden or the view changes).
    kitty_active: bool,
    /// Cover identity the last Kitty image was rendered for.
    last_kitty_gen: u64,
}

impl CoverRenderer {
    /// Create the production renderer writing to stdout.
    pub fn new() -> Self {
        Self::with_writer(Box::new(std::io::stdout()), Box::new(ChafaEncoder))
    }

    /// Test constructor — inject a writer and an encoder.
    fn with_writer(out: Box<dyn Write + Send>, encoder: Box<dyn SixelEncoder>) -> Self {
        let chafa_available = which_chafa();
        Self {
            out,
            encoder,
            chafa_available,
            chafa_sixel_cache: None,
            last_chafa_gen: 0,
            last_chafa_rect: None,
            chafa_failed: false,
            clear_pending: false,
            kitty_active: false,
            last_kitty_gen: 0,
        }
    }

    /// Render the cover via whichever protocol this terminal supports:
    /// Kitty if available, otherwise chafa SIXEL. Mutually exclusive — the
    /// two graphics layers would otherwise fight over the same area.
    pub fn render(&mut self, params: &CoverParams) {
        if is_kitty_graphics_compatible() {
            self.render_kitty(params);
        } else {
            self.render_chafa(params);
        }
    }

    /// Returns `true` when a `terminal.clear()` is needed before the next
    /// ratatui draw (SIXEL cleanup on view switch / cover change).
    pub fn needs_clear(&self) -> bool {
        self.clear_pending
    }

    /// Mark the clear as handled so it only fires once.
    pub fn clear_done(&mut self) {
        self.clear_pending = false;
    }

    // ── Kitty protocol (Kitty / WezTerm / Ghostty) ──

    fn render_kitty(&mut self, params: &CoverParams) {
        if !is_kitty_graphics_compatible() {
            self.kitty_active = false;
            return;
        }
        // Not on the player view, or an overlay is on top → clear the image.
        if params.active_view != ViewMode::Player || params.show_help || params.command_mode {
            self.clear_kitty();
            return;
        }
        // Cover hidden → clear the image.
        if !params.show_cover_art {
            self.clear_kitty();
            return;
        }

        let gen = params.cover_gen;
        if self.kitty_active && gen == self.last_kitty_gen {
            return; // image unchanged — Kitty image persists on screen
        }
        self.last_kitty_gen = gen;

        let cover = match &params.cover_art {
            Some(c) => c.clone(),
            None => {
                self.clear_kitty();
                return;
            }
        };

        let (x_chars, y_chars, w_chars, h_chars) = params.cover_rect;
        if w_chars == 0 || h_chars == 0 {
            return;
        }

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
                        self.out,
                        "\x1b_Ga=T,f=100,s={out_w},v={out_h},c=3,p={px},{py},m={more};{chunk_str}\x1b\\",
                    );
                }
                let _ = self.out.flush();
                self.kitty_active = true;
            }
            Err(e) => {
                tracing::warn!("Failed to decode cover for Kitty protocol: {e}");
            }
        }
    }

    /// Send the Kitty clear sequence if an image is currently displayed.
    fn clear_kitty(&mut self) {
        if self.kitty_active {
            let _ = write!(self.out, "\x1b_Ga=d,d=I\x1b\\");
            let _ = self.out.flush();
            self.kitty_active = false;
        }
    }

    // ── chafa SIXEL (Konsole Plasma 6+) ──

    fn render_chafa(&mut self, params: &CoverParams) {
        // Only render on the player view; hide under overlays.
        if params.active_view != ViewMode::Player || params.show_help || params.command_mode {
            self.reset_chafa();
            return;
        }
        // Cover hidden → clear any previously displayed SIXEL.
        if !params.show_cover_art {
            self.reset_chafa();
            return;
        }
        if !self.chafa_available {
            return;
        }

        // No cover on this track → a previously shown SIXEL must be cleared,
        // not left over the fallback song-info text.
        let cover = match &params.cover_art {
            Some(c) => c.clone(),
            None => {
                self.reset_chafa();
                return;
            }
        };

        let gen = params.cover_gen;
        let rect = params.cover_rect;

        // A previously-failed encode only retries when the cover or area
        // actually changed — no per-frame chafa spawns on terminals that
        // don't support SIXEL.
        if self.chafa_failed && self.last_chafa_gen == gen && self.last_chafa_rect == Some(rect) {
            return;
        }

        // SIXEL is a persistent graphics layer on Konsole: one send per
        // change is enough. Re-sending every frame was the root cause of
        // phantom key events and UI lag (see progress/2026-08-01).
        let dirty = self.chafa_sixel_cache.is_none()
            || gen != self.last_chafa_gen
            || self.last_chafa_rect != Some(rect);
        if !dirty {
            return;
        }
        self.chafa_failed = false;
        self.last_chafa_gen = gen;
        self.last_chafa_rect = Some(rect);

        match self.encoder.encode(&cover, rect.2, rect.3) {
            Ok(payload) if !payload.is_empty() => {
                self.chafa_sixel_cache = Some(payload.clone());
                // Position the cursor at the cover area, then send the payload
                // once. No cursor-hide sequence: hiding the cursor is cosmetic,
                // and a raw `\x1b[?25l` bypassing ratatui's cursor management
                // left the cursor hidden after exit.
                let _ = write!(self.out, "\x1b[{};{}H", rect.1 + 1, rect.0 + 1);
                let _ = self.out.write_all(&payload);
                let _ = self.out.flush();
            }
            Ok(_) => {
                // Empty output — terminal doesn't support SIXEL.
                tracing::warn!("chafa SIXEL output is empty — terminal may not support it");
                self.chafa_sixel_cache = None;
                self.chafa_failed = true;
            }
            Err(e) => {
                tracing::warn!("chafa failed: {e}");
                self.chafa_sixel_cache = None;
                self.chafa_failed = true;
            }
        }
    }

    /// Reset SIXEL state when leaving the player view or hiding the cover.
    /// Requests a full `terminal.clear()` so ratatui's diff buffer overwrites
    /// any SIXEL residue.
    fn reset_chafa(&mut self) {
        if self.chafa_sixel_cache.is_some() {
            self.clear_pending = true;
        }
        self.chafa_sixel_cache = None;
        self.last_chafa_gen = 0;
        self.last_chafa_rect = None;
        self.chafa_failed = false;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Write sink that records bytes so tests can assert on output volume.
    #[derive(Clone, Default)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct StubEncoder;
    impl SixelEncoder for StubEncoder {
        fn encode(&self, cover: &[u8], cols: u16, rows: u16) -> Result<Vec<u8>, String> {
            Ok(format!("SIXEL:{cols}x{rows}:{}B", cover.len()).into_bytes())
        }
    }

    struct EmptyEncoder;
    impl SixelEncoder for EmptyEncoder {
        fn encode(&self, _cover: &[u8], _cols: u16, _rows: u16) -> Result<Vec<u8>, String> {
            Ok(Vec::new()) // simulates a terminal that rejects SIXEL
        }
    }

    fn renderer(buf: &SharedBuf) -> CoverRenderer {
        CoverRenderer::with_writer(Box::new(buf.clone()), Box::new(StubEncoder))
    }

    fn params(gen: u64, rect: (u16, u16, u16, u16), cover: Option<&[u8]>) -> CoverParams {
        CoverParams {
            active_view: ViewMode::Player,
            show_help: false,
            command_mode: false,
            show_cover_art: true,
            cover_gen: gen,
            cover_art: cover.map(|b| Arc::new(b.to_vec())),
            cover_rect: rect,
        }
    }

    fn written(buf: &SharedBuf) -> usize {
        buf.0.lock().unwrap().len()
    }

    // ── Core invariant: one SIXEL send per change ──

    #[test]
    fn chafa_sends_once_until_cover_changes() {
        let buf = SharedBuf::default();
        let mut r = renderer(&buf);
        let p = params(1, (0, 0, 10, 10), Some(&[1, 2, 3]));
        r.render_chafa(&p);
        let first = written(&buf);
        assert!(first > 0, "first render must send the SIXEL payload");

        // Same cover + same area → nothing written again.
        r.render_chafa(&p);
        assert_eq!(written(&buf), first, "unchanged cover must not re-send");

        // A new cover (gen bump) re-sends.
        r.render_chafa(&params(2, (0, 0, 10, 10), Some(&[4, 5])));
        assert!(written(&buf) > first, "cover change must re-send");
    }

    #[test]
    fn chafa_resent_on_area_change() {
        let buf = SharedBuf::default();
        let mut r = renderer(&buf);
        r.render_chafa(&params(1, (0, 0, 10, 10), Some(&[1])));
        let first = written(&buf);

        // Terminal resize changes the render area → re-send.
        r.render_chafa(&params(1, (0, 0, 12, 10), Some(&[1])));
        assert!(written(&buf) > first, "resize must re-send");
    }

    // ── Cleanup on view switch / cover hide / coverless track ──

    #[test]
    fn leaving_player_view_requests_clear() {
        let buf = SharedBuf::default();
        let mut r = renderer(&buf);
        r.render_chafa(&params(1, (0, 0, 10, 10), Some(&[1])));
        assert!(!r.needs_clear());

        let mut p = params(1, (0, 0, 10, 10), Some(&[1]));
        p.active_view = ViewMode::Library;
        r.render_chafa(&p);
        assert!(r.needs_clear(), "leaving player view must schedule a clear");
    }

    #[test]
    fn hiding_cover_schedules_clear() {
        let buf = SharedBuf::default();
        let mut r = renderer(&buf);
        r.render_chafa(&params(1, (0, 0, 10, 10), Some(&[1])));

        let mut p = params(1, (0, 0, 10, 10), Some(&[1]));
        p.show_cover_art = false;
        r.render_chafa(&p);
        assert!(r.needs_clear(), "hiding the cover must schedule a clear");
    }

    #[test]
    fn coverless_track_clears_stale_sixel() {
        let buf = SharedBuf::default();
        let mut r = renderer(&buf);
        r.render_chafa(&params(1, (0, 0, 10, 10), Some(&[1])));
        assert!(!r.needs_clear());

        // Next track has no embedded cover — the stale SIXEL must not linger.
        r.render_chafa(&params(2, (0, 0, 10, 10), None));
        assert!(
            r.needs_clear(),
            "stale SIXEL must be cleared on a coverless track"
        );
    }

    // ── Failure handling ──

    #[test]
    fn empty_sixel_output_is_not_retried_every_frame() {
        let buf = SharedBuf::default();
        let mut r = CoverRenderer::with_writer(Box::new(buf.clone()), Box::new(EmptyEncoder));
        r.render_chafa(&params(1, (0, 0, 10, 10), Some(&[1])));
        let first = written(&buf);
        assert_eq!(first, 0, "empty payload writes nothing");

        // Terminal rejects SIXEL → must not spawn chafa on every frame.
        r.render_chafa(&params(1, (0, 0, 10, 10), Some(&[1])));
        assert_eq!(
            written(&buf),
            0,
            "failed encode must not be retried per frame"
        );
    }
}
