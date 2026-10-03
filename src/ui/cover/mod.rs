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

use crate::constants::runtime;
use crate::ui::ViewMode;

/// Read-only parameters for cover art rendering.
/// Extracted from `UiState` so the cover renderer only sees what it needs.
pub struct CoverParams {
    pub active_view: ViewMode,
    pub show_help: bool,
    pub command_mode: bool,
    /// Player queue search is open. The results list replaces the area the
    /// cover occupies, so the persistent image layer has to stand down.
    pub search_active: bool,
    pub show_cover_art: bool,
    pub cover_gen: u64,
    pub cover_art: Option<Arc<Vec<u8>>>,
    pub cover_rect: (u16, u16, u16, u16),
    /// Pixel size of one terminal cell, as measured by the UI for the frame
    /// that was just drawn.
    pub cell_px: (u16, u16),
    /// Whether the half-block layer was suppressed for the frame that was just
    /// drawn. It is what ratatui was told to put in the cover cells, and the
    /// SIXEL payload has to be re-sent whenever it changes: suppressing the
    /// blocks makes ratatui write over the cover rect, and that write lands on
    /// top of the graphics layer underneath.
    pub blocks_suppressed: bool,
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
            // chafa keeps the aspect ratio by default and letterboxes into the
            // size it was given. The box below is already sized to the
            // artwork, so the letterbox was pure slack — and the slack was
            // filled by whatever the half-block layer had drawn there.
            .arg("--stretch")
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
    /// Whether the terminal speaks the Kitty graphics protocol (resolved once
    /// at construction; see `is_kitty_graphics_compatible`).
    kitty_available: bool,
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
    /// Value of `CoverParams::blocks_suppressed` on the frame the current
    /// payload was sent for. A change means the cover cells were rewritten by
    /// the UI, which erases the graphics layer — so the payload is re-sent.
    last_blocks_suppressed: bool,
    /// Set to `true` when leaving the player view — the event loop calls
    /// `terminal.clear()` before the next ratatui draw so the internal diff
    /// buffer covers all cells and overwrites any SIXEL residue.
    clear_pending: bool,
    /// Whether a Kitty image is currently displayed on screen (needed to
    /// send the clear sequence when the cover is hidden or the view changes).
    kitty_active: bool,
    /// Cover identity the last Kitty image was rendered for.
    last_kitty_gen: u64,
    /// Last area the Kitty image was placed at. The SIXEL path already tracked
    /// this; without it a terminal resize left the image at its old position
    /// and size, since the cover itself had not changed.
    last_kitty_rect: Option<(u16, u16, u16, u16)>,
}

impl CoverRenderer {
    /// Create the production renderer writing to stdout.
    pub fn new() -> Self {
        let mut renderer = Self::with_writer(Box::new(std::io::stdout()), Box::new(ChafaEncoder));
        renderer.chafa_available = which_chafa();
        renderer.kitty_available = is_kitty_graphics_compatible();
        renderer
    }

    /// Test constructor — inject a writer and an encoder. The injected encoder
    /// is assumed to work, so `chafa_available` is forced on — the real `chafa`
    /// binary is not installed on CI runners, and the tests stub the encoder
    /// anyway. `kitty_available` starts off and is switched on by the tests
    /// that exercise that path: terminal detection reads the environment, which
    /// cannot be varied per-test without racing the other tests.
    fn with_writer(out: Box<dyn Write + Send>, encoder: Box<dyn SixelEncoder>) -> Self {
        Self {
            out,
            encoder,
            chafa_available: true,
            kitty_available: false,
            chafa_sixel_cache: None,
            last_chafa_gen: 0,
            last_chafa_rect: None,
            chafa_failed: false,
            last_blocks_suppressed: false,
            clear_pending: false,
            kitty_active: false,
            last_kitty_gen: 0,
            last_kitty_rect: None,
        }
    }

    /// Whether a terminal-native image is on screen for the current cover.
    /// The UI reads this (a frame late) to decide whether the half-block
    /// fallback is needed on top of it.
    pub fn native_active(&self) -> bool {
        if self.kitty_available {
            self.kitty_active
        } else {
            self.chafa_sixel_cache.is_some()
        }
    }

    /// Render the cover via whichever protocol this terminal supports:
    /// Kitty if available, otherwise chafa SIXEL. Mutually exclusive — the
    /// two graphics layers would otherwise fight over the same area.
    pub fn render(&mut self, params: &CoverParams) {
        if self.kitty_available {
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
        // Protocol selection already happened in `render`; re-checking the
        // environment here was redundant and put this whole path out of reach
        // of the tests.
        // Not on the player view, or an overlay is on top → clear the image.
        // Search counts: the results panel replaces the area the cover sits in,
        // and a persisted image would cover the text.
        if params.active_view != ViewMode::Player
            || params.show_help
            || params.command_mode
            || params.search_active
        {
            self.clear_kitty();
            return;
        }
        // Cover hidden → clear the image.
        if !params.show_cover_art {
            self.clear_kitty();
            return;
        }

        let gen = params.cover_gen;
        // Re-place the image when the cover changes *or* the area moves: a
        // resize keeps the same cover but needs the image re-sent at the new
        // geometry, which is what the SIXEL path already did. The suppression
        // flip counts as a move: ratatui rewrote the cover cells that frame.
        if self.kitty_active
            && gen == self.last_kitty_gen
            && self.last_kitty_rect == Some(params.cover_rect)
            && self.last_blocks_suppressed == params.blocks_suppressed
        {
            return; // image unchanged — Kitty image persists on screen
        }
        self.last_kitty_gen = gen;
        self.last_kitty_rect = Some(params.cover_rect);
        self.last_blocks_suppressed = params.blocks_suppressed;

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

                // Scale image to fill the cover rect, preserving its aspect
                // (which is the rect's aspect too — see `fit_cover_rect`).
                // Deliberately not capped at 1.0: a cap left artwork smaller
                // than the rect rendered at its own size, uncovered strip and
                // all, which is one of the ways the panel showed through.
                let scale = (area_w as f64 / img_w as f64).min(area_h as f64 / img_h as f64);
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
                    // `c`/`r` tell the terminal how many cells the image
                    // occupies. This used to say `c=3`, which asked every
                    // Kitty terminal for a three-column cover; naming both
                    // dimensions ties the image to the cover rect exactly.
                    let _ = write!(
                        self.out,
                        "\x1b_Ga=T,f=100,s={out_w},v={out_h},c={w_chars},r={h_chars},p={px},{py},m={more};{chunk_str}\x1b\\",
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
        // Forget the placement too: coming back must re-send, even if the
        // cover is the same one that was cleared.
        self.last_kitty_rect = None;
        if self.kitty_active {
            let _ = write!(self.out, "\x1b_Ga=d,d=I\x1b\\");
            let _ = self.out.flush();
            self.kitty_active = false;
        }
    }

    // ── chafa SIXEL (Konsole Plasma 6+) ──

    fn render_chafa(&mut self, params: &CoverParams) {
        // Only render on the player view; hide under overlays.
        if params.active_view != ViewMode::Player
            || params.show_help
            || params.command_mode
            || params.search_active
        {
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
            || self.last_chafa_rect != Some(rect)
            // The UI rewrote the cover cells (the block layer was suppressed),
            // which paints over the SIXEL underneath.
            || self.last_blocks_suppressed != params.blocks_suppressed;
        if !dirty {
            return;
        }
        self.chafa_failed = false;
        self.last_chafa_gen = gen;
        self.last_chafa_rect = Some(rect);
        self.last_blocks_suppressed = params.blocks_suppressed;

        let (box_w, box_h) = chafa_box(rect, params.cell_px);
        match self.encoder.encode(&cover, box_w, box_h) {
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

/// Translate the cover rect from terminal cells into the cell counts chafa has
/// to be asked for, so that the SIXEL it emits covers the rect's *pixels*.
///
/// chafa sizes its output at [`runtime::CHAFA_SIXEL_CELL_PX`] per requested
/// cell and cannot see this terminal, so passing the rect's own cell counts —
/// what the code used to do — was only correct on a terminal whose cells
/// happen to be that size. Everywhere else the payload came out the wrong
/// size, and the half-block art drawn underneath showed around it.
///
/// Rounded down on purpose: a payload a few pixels short leaves a clean margin,
/// while one that is too big would spill over the panel border.
fn chafa_box(rect: (u16, u16, u16, u16), cell_px: (u16, u16)) -> (u16, u16) {
    let cell = runtime::CHAFA_SIXEL_CELL_PX;
    let px_w = rect.2 as u32 * cell_px.0.max(1) as u32;
    let px_h = rect.3 as u32 * cell_px.1.max(1) as u32;
    ((px_w / cell).max(1) as u16, (px_h / cell).max(1) as u16)
}

/// Pixel size of one terminal cell, or [`runtime::FALLBACK_CELL_PX`] when the
/// terminal does not report one.
///
/// `window_size()` is a plain ioctl on the tty. Unlike an escape-sequence
/// query it writes nothing to stdin, so it cannot produce the phantom key
/// events that `chafa --probe off` exists to avoid.
pub fn terminal_cell_px() -> (u16, u16) {
    match crossterm::terminal::window_size() {
        Ok(ws) => cell_px_from_window((ws.columns, ws.rows, ws.width, ws.height))
            .unwrap_or(runtime::FALLBACK_CELL_PX),
        Err(_) => runtime::FALLBACK_CELL_PX,
    }
}

/// Sanity-check a reported window size and reduce it to a cell size.
///
/// Some terminals report pixel dimensions of zero, and a few report nonsense;
/// a bad cell size skews every cover box, so anything outside a plausible
/// range falls back to the default rather than being trusted.
///
/// Takes `(columns, rows, width_px, height_px)` — split out from
/// `terminal_cell_px` because the rules are worth testing without a terminal.
fn cell_px_from_window(ws: (u16, u16, u16, u16)) -> Option<(u16, u16)> {
    let (cols, rows, px_w, px_h) = ws;
    if cols == 0 || rows == 0 || px_w == 0 || px_h == 0 {
        return None;
    }
    let w = px_w / cols;
    let h = px_h / rows;
    // A cell is taller than it is wide, but not arbitrarily: outside this
    // range the terminal's numbers are not describing a text grid.
    let ratio = h as f32 / w.max(1) as f32;
    if w == 0 || h == 0 || !(1.2..=4.0).contains(&ratio) {
        return None;
    }
    Some((w, h))
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
/// Checks known env vars — no stdin query needed.
fn is_kitty_graphics_compatible() -> bool {
    // Inside a multiplexer the graphics sequences are swallowed unless
    // passthrough is explicitly configured, and they fail silently. That was
    // survivable while the block art was always drawn underneath; now that the
    // blocks stand aside for a native image, guessing wrong would leave an
    // empty panel. Fall through to chafa / blocks instead.
    if std::env::var_os("TMUX").is_some() || std::env::var_os("STY").is_some() {
        return false;
    }
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
            search_active: false,
            show_cover_art: true,
            cover_gen: gen,
            cover_art: cover.map(|b| Arc::new(b.to_vec())),
            cover_rect: rect,
            cell_px: (10, 20),
            blocks_suppressed: false,
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

    // ── chafa geometry ──

    /// chafa emits a fixed 20 px per requested cell and cannot see this
    /// terminal, so the rect has to be converted into chafa's own units. On a
    /// 10×20 cell, ten columns of terminal are five cells of chafa.
    #[test]
    fn chafa_box_converts_cells_to_chafa_pixels() {
        assert_eq!(chafa_box((0, 0, 10, 10), (10, 20)), (5, 10));
        // A terminal with 20×20 cells maps one-to-one — the size chafa assumes.
        assert_eq!(chafa_box((0, 0, 10, 10), (20, 20)), (10, 10));
    }

    /// Rounded down, never up: a payload a few pixels short leaves a clean
    /// margin, one that is too big spills over the panel border.
    #[test]
    fn chafa_box_never_overflows_the_rect() {
        for cell in [(10u16, 20u16), (8, 17), (12, 24), (20, 20)] {
            for (w, h) in [(13u16, 7u16), (40, 20), (3, 30), (4, 4)] {
                let (bw, bh) = chafa_box((0, 0, w, h), cell);
                let px_w = bw as u32 * runtime::CHAFA_SIXEL_CELL_PX;
                let px_h = bh as u32 * runtime::CHAFA_SIXEL_CELL_PX;
                assert!(
                    px_w <= w as u32 * cell.0.max(1) as u32,
                    "box {bw} cells wide overflows {w} columns at cell {cell:?}"
                );
                assert!(px_h <= h as u32 * cell.1.max(1) as u32);
                assert!(bw >= 1 && bh >= 1, "chafa needs a non-zero box");
            }
        }
    }

    /// chafa cannot draw less than one of its cells, so a cover rect narrower
    /// than that is the one case the box overshoots. Degenerate terminals are
    /// refused by `render` long before a cover panel gets that small; what
    /// matters here is that it stays usable rather than collapsing to zero.
    #[test]
    fn chafa_box_stays_usable_for_a_sub_cell_rect() {
        assert_eq!(chafa_box((0, 0, 1, 1), (10, 20)), (1, 1));

        // An unknown cell size is clamped instead of dividing by zero.
        assert_eq!(chafa_box((0, 0, 4, 4), (0, 0)), (1, 1));
    }

    // ── Cell size detection ──

    #[test]
    fn cell_size_is_derived_from_the_window_pixels() {
        // 1000×800 px over 100×40 cells = 10×20 px per cell.
        assert_eq!(cell_px_from_window((100, 40, 1000, 800)), Some((10, 20)));
        assert_eq!(cell_px_from_window((80, 24, 800, 480)), Some((10, 20)));
    }

    /// This runs on every frame, including when there is no terminal at all
    /// (a test process, a redirected run). It must come back with something
    /// usable rather than panicking or returning a zero cell.
    #[test]
    fn terminal_cell_px_always_yields_a_usable_cell() {
        let (w, h) = terminal_cell_px();
        assert!(w >= 1 && h >= 1, "got a {w}x{h} cell");
    }

    /// Terminals that don't report pixels (tmux, some emulators) must fall
    /// back rather than divide by zero.
    #[test]
    fn cell_size_rejects_unusable_window_reports() {
        assert_eq!(cell_px_from_window((0, 0, 0, 0)), None);
        assert_eq!(cell_px_from_window((100, 40, 0, 0)), None, "no pixel size");
        assert_eq!(cell_px_from_window((0, 40, 1000, 800)), None, "no columns");
        // 1000 px over 100 columns is 10 px wide but 1 px tall — not a text
        // grid, and trusting it would make every cover box absurd.
        assert_eq!(cell_px_from_window((100, 800, 1000, 800)), None);
    }

    // ── Block-layer suppression ──

    /// The UI stops drawing blocks once a native image is up. That rewrite
    /// lands on top of the graphics layer, so the payload has to go out again
    /// on the same frame or the cover disappears.
    #[test]
    fn suppressing_the_block_layer_resends_the_sixel() {
        let buf = SharedBuf::default();
        let mut r = renderer(&buf);
        r.render_chafa(&params(1, (0, 0, 10, 10), Some(&[1])));
        let placed = written(&buf);
        assert!(r.native_active(), "a payload was sent");

        // Blocks still drawn, nothing changed → the image persists.
        r.render_chafa(&params(1, (0, 0, 10, 10), Some(&[1])));
        assert_eq!(written(&buf), placed, "no change, no re-send");

        // The UI suppressed the blocks this frame: ratatui wrote over the
        // cover rect, so the payload is sent again.
        let mut suppressed = params(1, (0, 0, 10, 10), Some(&[1]));
        suppressed.blocks_suppressed = true;
        r.render_chafa(&suppressed);
        assert!(
            written(&buf) > placed,
            "the rewritten cells erase the SIXEL"
        );

        // ...and then it settles: no further re-sends while nothing changes.
        let settled = written(&buf);
        r.render_chafa(&suppressed);
        assert_eq!(written(&buf), settled, "stable once the blocks are gone");
    }

    /// `native_active` is what the UI consults, so it must be false whenever
    /// the encoder produced nothing — otherwise the fallback art is dropped
    /// for an image that never arrives.
    #[test]
    fn native_active_is_false_when_the_encode_produced_nothing() {
        let buf = SharedBuf::default();
        let mut r = CoverRenderer::with_writer(Box::new(buf.clone()), Box::new(EmptyEncoder));
        r.render_chafa(&params(1, (0, 0, 10, 10), Some(&[1])));
        assert!(!r.native_active());
    }

    /// Leaving the player view clears the image, and `native_active` has to
    /// follow so the blocks come back on the next frame that shows a cover.
    #[test]
    fn native_active_tracks_the_clear_on_view_change() {
        let buf = SharedBuf::default();
        let mut r = renderer(&buf);
        r.render_chafa(&params(1, (0, 0, 10, 10), Some(&[1])));
        assert!(r.native_active());

        let mut away = params(1, (0, 0, 10, 10), Some(&[1]));
        away.active_view = ViewMode::Library;
        r.render_chafa(&away);
        assert!(!r.native_active());
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

    // ── Kitty placement and overlays ──

    /// A tiny in-memory 8×8 PNG — the Kitty path decodes real image bytes.
    fn make_png() -> Vec<u8> {
        let img = image::RgbaImage::from_fn(8, 8, |x, y| {
            let v = ((x + y) * 32).min(255) as u8;
            image::Rgba([v, v, v, 255])
        });
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    /// The terminal-detection path reads the environment, which a test cannot
    /// vary without racing every other test in the process, so the resolved
    /// flag is switched on directly — the same way `chafa_available` is.
    fn kitty_renderer(buf: &SharedBuf) -> CoverRenderer {
        let mut r = renderer(buf);
        r.kitty_available = true;
        r
    }

    fn kitty_clear_sent(buf: &SharedBuf) -> bool {
        String::from_utf8_lossy(&buf.0.lock().unwrap()).contains("\x1b_Ga=d,d=I")
    }

    /// Resizing keeps the same cover but moves the area it is drawn into. The
    /// Kitty path compared only the cover identity, so after a resize the image
    /// stayed at its old position and size.
    #[test]
    fn kitty_image_is_replaced_when_the_area_changes() {
        let buf = SharedBuf::default();
        let mut r = kitty_renderer(&buf);
        let png = make_png();

        r.render(&params(1, (0, 0, 10, 10), Some(&png)));
        let placed = written(&buf);
        assert!(placed > 0, "first render places the image");

        // Same cover, same area → the image persists, nothing to send.
        r.render(&params(1, (0, 0, 10, 10), Some(&png)));
        assert_eq!(
            written(&buf),
            placed,
            "unchanged cover and area: no re-send"
        );

        // Same cover, new area (terminal resize) → re-place.
        r.render(&params(1, (0, 0, 20, 12), Some(&png)));
        assert!(
            written(&buf) > placed,
            "a resize must re-place the image at the new geometry"
        );
    }

    /// Player search replaces the area the cover occupies. The image is a
    /// persistent layer, so it has to be removed or it paints over the results.
    #[test]
    fn search_overlay_clears_the_kitty_image() {
        let buf = SharedBuf::default();
        let mut r = kitty_renderer(&buf);
        let png = make_png();

        r.render(&params(1, (0, 0, 10, 10), Some(&png)));
        assert!(r.kitty_active, "image placed to begin with");

        let mut search = params(1, (0, 0, 10, 10), Some(&png));
        search.search_active = true;
        r.render(&search);

        assert!(!r.kitty_active, "image must be removed while searching");
        assert!(kitty_clear_sent(&buf), "clear sequence must be sent");
    }

    #[test]
    fn search_overlay_clears_the_sixel_layer() {
        let buf = SharedBuf::default();
        let mut r = renderer(&buf); // chafa path

        r.render(&params(1, (0, 0, 10, 10), Some(&[1, 2, 3])));
        assert!(!r.needs_clear(), "nothing to clear after a fresh send");

        let mut search = params(1, (0, 0, 10, 10), Some(&[1, 2, 3]));
        search.search_active = true;
        r.render(&search);

        assert!(
            r.needs_clear(),
            "SIXEL residue must be cleared for the search results"
        );
    }
}
