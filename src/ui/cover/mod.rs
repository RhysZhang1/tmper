//! Cover art rendering — outputs pixel data directly to the terminal
//! outside of ratatui's buffer, using terminal-native graphics protocols.
//!
//! Rendering priority chain:
//!   Kitty protocol (Kitty / WezTerm / Ghostty)
//!   → SIXEL encoded in-process (Konsole Plasma 6+)
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

use std::io::{IsTerminal, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

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
/// Abstracted so tests can stub the encoder.
///
/// `Send` keeps `CoverRenderer` (and thus `App`) `Send` for the multi-threaded
/// tokio runtime.
trait SixelEncoder: Send {
    /// Encode `cover` (the raw embedded image, any size) into a payload that
    /// covers exactly `px` pixels. Returns the payload to write to the
    /// terminal, or a human-readable error.
    fn encode(&self, cover: &[u8], px: (u16, u16)) -> Result<Vec<u8>, String>;
}

/// Default encoder: SIXEL, encoded in-process by `icy_sixel`.
///
/// This used to shell out to `chafa`. Two things changed with the swap: the
/// raster is now the pixel box tmper asks for (rather than a cell count
/// multiplied back up by a *second* program's idea of a cell — the mismatch
/// behind `progress/2026-10-03-chafa-cell-units.md`), and the color ceiling is
/// unchanged, because SIXEL addresses colors with 8 bits and chafa capped its
/// own output at 255 registers anyway. Measured against chafa at the same
/// raster, this encoder is at or ahead on MAE/PSNR and ~7× faster; the
/// numbers are in `progress/2026-10-03-encoder-and-terminal-compat.md`.
struct IcySixelEncoder;

impl SixelEncoder for IcySixelEncoder {
    fn encode(&self, cover: &[u8], px: (u16, u16)) -> Result<Vec<u8>, String> {
        let (w, h) = (px.0.max(1) as u32, px.1.max(1) as u32);
        let img =
            image::load_from_memory(cover).map_err(|e| format!("cover decode failed: {e}"))?;
        // Stretched to the box rather than fitted inside it: the box is
        // already aspect-fitted to the artwork (`player_view::fit_cover_rect`),
        // so filling it is exact, and letterboxing here would only re-open the
        // slack the cover panel used to show.
        let rgba = img
            .resize_exact(w, h, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        let opts = icy_sixel::EncodeOptions {
            max_colors: runtime::SIXEL_MAX_COLORS,
            diffusion: runtime::SIXEL_DIFFUSION,
            quantize_method: icy_sixel::QuantizeMethod::Wu,
        };
        icy_sixel::sixel_encode(rgba.as_raw(), w as usize, h as usize, &opts)
            .map(String::into_bytes)
            .map_err(|e| format!("sixel encode failed: {e}"))
    }
}

/// Manages cover-art rendering state and terminal protocol output.
///
/// All fields are private — interaction goes through the public methods.
/// Protocol output goes through an injectable writer so tests can capture it.
pub struct CoverRenderer {
    out: Box<dyn Write + Send>,
    encoder: Box<dyn SixelEncoder>,
    /// Whether the terminal can display SIXEL. `None` (production) defers to
    /// the startup probe, which cannot run yet: `CoverRenderer::new` is called
    /// from `App::new`, and the probe needs the tty in raw mode, which
    /// `App::run` sets up afterwards. Tests pin the value instead.
    sixel_available: Option<bool>,
    /// Whether the terminal speaks the Kitty graphics protocol (resolved once
    /// at construction; see `is_kitty_graphics_compatible`).
    kitty_available: bool,
    /// Cached SIXEL payload — re-sent only when the cover or area changes.
    sixel_cache: Option<Vec<u8>>,
    /// Cover identity the cached SIXEL payload was rendered for.
    last_sixel_gen: u64,
    /// Last area (x,y,w,h) the SIXEL payload was rendered for.
    last_sixel_rect: Option<(u16, u16, u16, u16)>,
    /// Sticky: encoding produced no usable payload. Stops per-frame encodes
    /// until the cover or area changes (a terminal that rejects SIXEL won't
    /// start accepting it mid-session).
    sixel_failed: bool,
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
        let mut renderer =
            Self::with_writer(Box::new(std::io::stdout()), Box::new(IcySixelEncoder));
        renderer.sixel_available = None; // ask the startup probe
        renderer.kitty_available = is_kitty_graphics_compatible();
        renderer
    }

    /// Test constructor — inject a writer and an encoder. The injected encoder
    /// is assumed to work, so SIXEL is forced on: a test process has no
    /// terminal to probe and the tests stub the encoder anyway.
    /// `kitty_available` starts off and is switched on by the tests that
    /// exercise that path: terminal detection reads the environment, which
    /// cannot be varied per-test without racing the other tests.
    fn with_writer(out: Box<dyn Write + Send>, encoder: Box<dyn SixelEncoder>) -> Self {
        Self {
            out,
            encoder,
            sixel_available: Some(true),
            kitty_available: false,
            sixel_cache: None,
            last_sixel_gen: 0,
            last_sixel_rect: None,
            sixel_failed: false,
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
            self.sixel_cache.is_some()
        }
    }

    /// Whether this terminal can display SIXEL — the probe's answer when the
    /// renderer is the production one, the pinned value in tests.
    fn sixel_supported(&self) -> bool {
        self.sixel_available
            .unwrap_or_else(|| terminal_caps().sixel)
    }

    /// Render the cover via whichever protocol this terminal supports:
    /// Kitty if available, otherwise SIXEL. Mutually exclusive — the two
    /// graphics layers would otherwise fight over the same area.
    pub fn render(&mut self, params: &CoverParams) {
        if self.kitty_available {
            self.render_kitty(params);
        } else {
            self.render_sixel(params);
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

    // ── SIXEL (Konsole Plasma 6+) ──

    fn render_sixel(&mut self, params: &CoverParams) {
        // Only render on the player view; hide under overlays.
        if params.active_view != ViewMode::Player
            || params.show_help
            || params.command_mode
            || params.search_active
        {
            self.reset_sixel();
            return;
        }
        // Cover hidden → clear any previously displayed SIXEL.
        if !params.show_cover_art {
            self.reset_sixel();
            return;
        }
        if !self.sixel_supported() {
            return;
        }

        // No cover on this track → a previously shown SIXEL must be cleared,
        // not left over the fallback song-info text.
        let cover = match &params.cover_art {
            Some(c) => c.clone(),
            None => {
                self.reset_sixel();
                return;
            }
        };

        let gen = params.cover_gen;
        let rect = params.cover_rect;

        // A previously-failed encode only retries when the cover or area
        // actually changed — no per-frame encodes on terminals that reject
        // SIXEL.
        if self.sixel_failed && self.last_sixel_gen == gen && self.last_sixel_rect == Some(rect) {
            return;
        }

        // SIXEL is a persistent graphics layer on Konsole: one send per
        // change is enough. Re-sending every frame was the root cause of
        // phantom key events and UI lag (see progress/2026-08-01).
        let dirty = self.sixel_cache.is_none()
            || gen != self.last_sixel_gen
            || self.last_sixel_rect != Some(rect)
            // The UI rewrote the cover cells (the block layer was suppressed),
            // which paints over the SIXEL underneath.
            || self.last_blocks_suppressed != params.blocks_suppressed;
        if !dirty {
            return;
        }
        self.sixel_failed = false;
        self.last_sixel_gen = gen;
        self.last_sixel_rect = Some(rect);
        self.last_blocks_suppressed = params.blocks_suppressed;

        // The payload covers the rect's pixels exactly. There is no cell
        // count to convert any more: the encoder is asked for the same box
        // the half-block layer draws into, which is the whole point of
        // encoding it here rather than in a second program.
        let px = (
            rect.2.saturating_mul(params.cell_px.0).max(1),
            rect.3.saturating_mul(params.cell_px.1).max(1),
        );
        match self.encoder.encode(&cover, px) {
            Ok(payload) if !payload.is_empty() => {
                self.sixel_cache = Some(payload.clone());
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
                tracing::warn!("SIXEL output is empty — terminal may not support it");
                self.sixel_cache = None;
                self.sixel_failed = true;
            }
            Err(e) => {
                tracing::warn!("sixel encode failed: {e}");
                self.sixel_cache = None;
                self.sixel_failed = true;
            }
        }
    }

    /// Reset SIXEL state when leaving the player view or hiding the cover.
    /// Requests a full `terminal.clear()` so ratatui's diff buffer overwrites
    /// any SIXEL residue.
    fn reset_sixel(&mut self) {
        if self.sixel_cache.is_some() {
            self.clear_pending = true;
        }
        self.sixel_cache = None;
        self.last_sixel_gen = 0;
        self.last_sixel_rect = None;
        self.sixel_failed = false;
    }
}

// ── Helpers ──

/// Pixel size of one terminal cell.
///
/// Three sources, best first:
///
/// 1. [`probe_terminal_once`]'s answer — the terminal's own font metrics,
///    which is the only source that is right on Konsole;
/// 2. `TIOCGWINSZ`'s `ws_xpixel`/`ws_ypixel` fields, when the terminal fills
///    them in (they are plain integers in the struct — no escape sequences,
///    no stdin involvement);
/// 3. [`runtime::FALLBACK_CELL_PX`].
pub fn terminal_cell_px() -> (u16, u16) {
    if let Some(cell) = terminal_caps().cell_px {
        return cell;
    }
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
    // range the terminal's numbers are not describing a text grid. The lower
    // bound is 1.0 rather than something rounder because square cells are
    // real — a wide font at a large size gives e.g. 20×20.
    let ratio = h as f32 / w.max(1) as f32;
    if w == 0 || h == 0 || !(1.0..=6.0).contains(&ratio) {
        return None;
    }
    Some((w, h))
}

// ── Startup terminal probe ──

/// What the terminal told us about itself at startup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalCaps {
    /// Cell size in pixels, when the terminal reported one.
    pub cell_px: Option<(u16, u16)>,
    /// The terminal advertised SIXEL graphics in its primary device
    /// attributes reply (DA1, parameter 4).
    pub sixel: bool,
}

/// xterm's "report character cell size in pixels" (`CSI 16 t`), plus the pair
/// of text-area reports (`CSI 14 t` / `CSI 18 t`) for terminals that implement
/// only those — their quotient gives the same answer — and finally the primary
/// device attributes request (`CSI c`).
///
/// DA1 goes **last** on purpose. Replies come back in the order the queries
/// were sent and every terminal answers DA1, so a complete DA1 reply is a
/// barrier: everything asked before it has already arrived.
const TERMINAL_QUERY: &[u8] = b"\x1b[16t\x1b[14t\x1b[18t\x1b[c";

/// How long the terminal gets to answer before we settle for `TIOCGWINSZ`.
///
/// A local tty answers in well under a millisecond; this budget only ever runs
/// out on terminals that ignore the query, where it costs a few frames of
/// startup and nothing else.
const TERMINAL_QUERY_BUDGET: Duration = Duration::from_millis(80);

/// What the probe learned, once it has run.
static PROBED_CAPS: OnceLock<TerminalCaps> = OnceLock::new();

/// The capabilities probed at startup. Empty (no cell size, no SIXEL) until
/// [`probe_terminal_once`] has run, and in any process without a tty.
pub fn terminal_caps() -> TerminalCaps {
    PROBED_CAPS.get().copied().unwrap_or_default()
}

/// Ask the terminal what it is — cell size and graphics support — once.
///
/// The cell size is the only source that works on Konsole: it leaves the
/// `ws_xpixel`/`ws_ypixel` fields of `TIOCGWINSZ` at zero, so without the probe
/// the cover is sized from a 10×20 guess — the wrong *scale*, and (because
/// `fit_cover_rect` divides the panel up using the same numbers) the wrong
/// *shape*. The SIXEL answer decides whether a graphics payload is written at
/// all; a terminal that cannot display one must keep the half-block art.
///
/// Must be called with the tty already in raw mode and before anything else
/// reads stdin. In canonical mode the reply never becomes readable at all —
/// the line discipline holds it back until a newline that never comes — and a
/// concurrent reader could swallow it. `App::run` satisfies both: it calls this
/// right after `TerminalGuard::enter`, which is where raw mode is set.
pub fn probe_terminal_once() -> TerminalCaps {
    *PROBED_CAPS.get_or_init(|| {
        // Every exit from here logs, including the ones that detect nothing:
        // this is the single place that knows why the cover came out the size
        // it did, and `grep "cell size" ~/.local/state/tmper/tmper.log` is what
        // `config.toml`'s `cell_px` comment tells the user to run.
        if !std::io::stdin().is_terminal() {
            tracing::info!("stdin is not a tty: no terminal probe, no cover sizing");
            return TerminalCaps::default();
        }
        let caps = query_terminal(TERMINAL_QUERY_BUDGET);
        match caps.cell_px {
            Some((w, h)) => tracing::info!("terminal cell size: {w}x{h} px (CSI 16t)"),
            None => tracing::info!(
                "terminal answered no cell-size query; TIOCGWINSZ reports {:?}",
                crossterm::terminal::window_size()
                    .map(|ws| (ws.columns, ws.rows, ws.width, ws.height))
            ),
        }
        // Konsole answers `CSI ? 62 ; 1 ; 4 c` — "a VT2xx with 132 columns and
        // Sixel". Terminals that omit the 4 cannot display a sixel and say so
        // here rather than by silently swallowing the payload.
        tracing::info!(
            "terminal graphics: sixel {} (DA1)",
            if caps.sixel { "yes" } else { "no" }
        );
        caps
    })
}

/// Send the probe's queries and collect the replies, giving up after `budget`.
///
/// `poll(2)` is what makes this safe to do at all: it waits *without
/// consuming*, so a terminal that never answers leaves the input queue exactly
/// as it found it and costs nothing but the timeout. A blocking read — even on
/// a helper thread — would sit on the tty and could steal a later keystroke.
///
/// Should a reply arrive too late for this window it lands on the input thread
/// instead, where it is harmless. crossterm's parser ends `CSI <n> ; … t` in
/// `parse_csi_modifier_key_code`, which only knows `A B C D F H P Q R S` and
/// errors on anything else — and `read()` is called with `if let Ok(..)` — so a
/// stray report cannot become a key event. A DA1 reply is even quieter: it maps
/// to `InternalEvent::PrimaryDeviceAttributes`, which has no public `Event`
/// counterpart and is dropped by the reader.
fn query_terminal(budget: Duration) -> TerminalCaps {
    let mut out = std::io::stdout();
    query_terminal_on(&mut out, std::io::stdin().as_raw_fd(), budget)
}

/// [`query_terminal`] with the terminal passed in.
///
/// The wait and the parse are the whole of the probe's logic, and neither cares
/// that the far end is a tty — only that bytes go out and the reply comes back.
/// Taking the descriptor as a parameter is what lets the tests drive that loop
/// over a pipe, where "data is waiting" and "nothing ever arrives" are both
/// reproducible without a terminal.
fn query_terminal_on(out: &mut dyn Write, fd: RawFd, budget: Duration) -> TerminalCaps {
    let mut caps = TerminalCaps::default();
    if out
        .write_all(TERMINAL_QUERY)
        .and_then(|_| out.flush())
        .is_err()
    {
        return caps;
    }

    let deadline = Instant::now() + budget;
    let mut seen = Vec::with_capacity(32);
    let mut chunk = [0u8; 64];
    loop {
        // The DA1 reply is the barrier: it was asked last, so once it is
        // complete every earlier answer is already in `seen`. Waiting longer
        // would only slow startup down.
        if let Some(sixel) = parse_da1_sixel(&seen) {
            caps.sixel = sixel;
            break;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pfd` is an initialised pollfd and the count is its length.
        let ready =
            unsafe { libc::poll(&mut pfd, 1, left.as_millis().min(i32::MAX as u128) as i32) };
        if ready < 0 {
            // A signal (SIGWINCH, SIGCHLD) interrupts the wait; the budget,
            // not the signal, is what should end it.
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if ready == 0 {
            break;
        }
        // SAFETY: `chunk` is a valid writable buffer of `chunk.len()` bytes.
        let n = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if n <= 0 {
            break; // EOF: nothing more is coming
        }
        seen.extend_from_slice(&chunk[..n as usize]);
    }
    // Whatever arrived, parse it — a terminal may answer one query and not the
    // other, and half an answer is still worth having.
    caps.cell_px = parse_cell_size_reply(&seen);
    if let Some(sixel) = parse_da1_sixel(&seen) {
        caps.sixel = sixel;
    }
    caps
}

/// The SIXEL bit of a primary device attributes reply, if the reply is here.
///
/// `None` means "no complete DA1 reply in the buffer" — deliberately distinct
/// from `Some(false)`, which means the terminal answered and did not claim
/// SIXEL. The caller uses the difference to stop reading early.
///
/// DA1 parameter 4 is xterm's "Sixel graphics" attribute, and it is what
/// Konsole (`CSI ? 62 ; 1 ; 4 c`), foot (`CSI ? 62 ; 4 ; 22 ; 28 ; 52 c`) and
/// xterm itself report when sixel is compiled in and enabled.
fn parse_da1_sixel(bytes: &[u8]) -> Option<bool> {
    let mut i = 0;
    while i + 3 < bytes.len() {
        // `CSI ?` …
        if bytes[i] != 0x1b || bytes[i + 1] != b'[' || bytes[i + 2] != b'?' {
            i += 1;
            continue;
        }
        let mut end = i + 3;
        let mut params: Vec<u16> = Vec::new();
        let mut current: Option<u16> = None;
        while end < bytes.len() {
            match bytes[end] {
                b'0'..=b'9' => {
                    let digit = u16::from(bytes[end] - b'0');
                    current = Some(
                        current
                            .unwrap_or(0)
                            .saturating_mul(10)
                            .saturating_add(digit),
                    );
                }
                b';' => {
                    params.push(current.unwrap_or(0));
                    current = None;
                }
                _ => break,
            }
            end += 1;
        }
        if bytes.get(end) == Some(&b'c') && end > i + 3 {
            if let Some(last) = current {
                params.push(last);
            }
            return Some(params.contains(&4));
        }
        i += 1;
    }
    None
}

/// Pull the cell size out of whatever the terminal sent back.
///
/// Accepts the direct answer (`CSI 6 ; <height> ; <width> t`) and, failing
/// that, the `CSI 4 ; <height> ; <width> t` / `CSI 8 ; <rows> ; <cols> t` pair
/// whose quotient is the same thing.
fn parse_cell_size_reply(bytes: &[u8]) -> Option<(u16, u16)> {
    let mut cell = None;
    // The text-area reports are in pixels and cells respectively, so they are
    // magnitudes apart from a cell size and get their own plausibility test
    // (non-zero) before being divided.
    let mut area_px: Option<(u32, u32)> = None;
    let mut area_cells: Option<(u32, u32)> = None;
    for params in csi_reports(bytes) {
        match params.as_slice() {
            [6, h, w] => cell = cell.or_else(|| plausible_cell(*w, *h)),
            [4, h, w] if *w > 0 && *h > 0 => area_px = Some((u32::from(*w), u32::from(*h))),
            [8, rows, cols] if *cols > 0 && *rows > 0 => {
                area_cells = Some((u32::from(*cols), u32::from(*rows)));
            }
            _ => {}
        }
    }
    cell.or_else(|| {
        let (px_w, px_h) = area_px?;
        let (cols, rows) = area_cells?;
        plausible_cell(
            u16::try_from(px_w / cols).ok()?,
            u16::try_from(px_h / rows).ok()?,
        )
    })
}

/// Every complete `CSI <params> t` report in `bytes`, as parameter lists.
///
/// The queue can legitimately hold other bytes — the user typing during
/// startup, a terminal answering some other query — so anything that is not
/// one of these is stepped over rather than treated as an error.
fn csi_reports(bytes: &[u8]) -> Vec<Vec<u16>> {
    let mut reports = Vec::new();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] != 0x1b || bytes[i + 1] != b'[' {
            i += 1;
            continue;
        }
        let mut end = i + 2;
        while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b';') {
            end += 1;
        }
        if end == i + 2 || bytes.get(end) != Some(&b't') {
            i += 1;
            continue;
        }
        let params: Option<Vec<u16>> =
            std::str::from_utf8(&bytes[i + 2..end])
                .ok()
                .and_then(|text| {
                    text.split(';')
                        .map(str::parse::<u16>)
                        .collect::<Result<Vec<u16>, _>>()
                        .ok()
                });
        if let Some(params) = params {
            reports.push(params);
        }
        i = end + 1;
    }
    reports
}

/// Reject values that cannot describe a text cell rather than sizing the cover
/// from them.
fn plausible_cell(w: u16, h: u16) -> Option<(u16, u16)> {
    ((1..=64).contains(&w) && (1..=128).contains(&h)).then_some((w, h))
}

/// Returns true if the terminal supports the Kitty graphics protocol.
/// Checks known env vars — no stdin query needed.
fn is_kitty_graphics_compatible() -> bool {
    // Inside a multiplexer the graphics sequences are swallowed unless
    // passthrough is explicitly configured, and they fail silently. That was
    // survivable while the block art was always drawn underneath; now that the
    // blocks stand aside for a native image, guessing wrong would leave an
    // empty panel. Fall through to SIXEL / blocks instead.
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
        /// Echoes the box it was asked for, so tests can assert on the
        /// geometry without a real encode.
        fn encode(&self, cover: &[u8], px: (u16, u16)) -> Result<Vec<u8>, String> {
            Ok(format!("SIXEL:{}x{}:{}B", px.0, px.1, cover.len()).into_bytes())
        }
    }

    struct EmptyEncoder;
    impl SixelEncoder for EmptyEncoder {
        fn encode(&self, _cover: &[u8], _px: (u16, u16)) -> Result<Vec<u8>, String> {
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

    /// Everything written so far, as text (the stub's payloads are ASCII).
    fn sent(buf: &SharedBuf) -> String {
        String::from_utf8_lossy(&buf.0.lock().unwrap()).into_owned()
    }

    // ── Core invariant: one SIXEL send per change ──

    #[test]
    fn sixel_sends_once_until_cover_changes() {
        let buf = SharedBuf::default();
        let mut r = renderer(&buf);
        let p = params(1, (0, 0, 10, 10), Some(&[1, 2, 3]));
        r.render_sixel(&p);
        let first = written(&buf);
        assert!(first > 0, "first render must send the SIXEL payload");

        // Same cover + same area → nothing written again.
        r.render_sixel(&p);
        assert_eq!(written(&buf), first, "unchanged cover must not re-send");

        // A new cover (gen bump) re-sends.
        r.render_sixel(&params(2, (0, 0, 10, 10), Some(&[4, 5])));
        assert!(written(&buf) > first, "cover change must re-send");
    }

    #[test]
    fn sixel_resent_on_area_change() {
        let buf = SharedBuf::default();
        let mut r = renderer(&buf);
        r.render_sixel(&params(1, (0, 0, 10, 10), Some(&[1])));
        let first = written(&buf);

        // Terminal resize changes the render area → re-send.
        r.render_sixel(&params(1, (0, 0, 12, 10), Some(&[1])));
        assert!(written(&buf) > first, "resize must re-send");
    }

    // ── Cleanup on view switch / cover hide / coverless track ──

    #[test]
    fn leaving_player_view_requests_clear() {
        let buf = SharedBuf::default();
        let mut r = renderer(&buf);
        r.render_sixel(&params(1, (0, 0, 10, 10), Some(&[1])));
        assert!(!r.needs_clear());

        let mut p = params(1, (0, 0, 10, 10), Some(&[1]));
        p.active_view = ViewMode::Library;
        r.render_sixel(&p);
        assert!(r.needs_clear(), "leaving player view must schedule a clear");
    }

    #[test]
    fn hiding_cover_schedules_clear() {
        let buf = SharedBuf::default();
        let mut r = renderer(&buf);
        r.render_sixel(&params(1, (0, 0, 10, 10), Some(&[1])));

        let mut p = params(1, (0, 0, 10, 10), Some(&[1]));
        p.show_cover_art = false;
        r.render_sixel(&p);
        assert!(r.needs_clear(), "hiding the cover must schedule a clear");
    }

    #[test]
    fn coverless_track_clears_stale_sixel() {
        let buf = SharedBuf::default();
        let mut r = renderer(&buf);
        r.render_sixel(&params(1, (0, 0, 10, 10), Some(&[1])));
        assert!(!r.needs_clear());

        // Next track has no embedded cover — the stale SIXEL must not linger.
        r.render_sixel(&params(2, (0, 0, 10, 10), None));
        assert!(
            r.needs_clear(),
            "stale SIXEL must be cleared on a coverless track"
        );
    }

    // ── Geometry: the payload covers the rect's pixels ──

    /// The encoder is asked for the rect's own pixels — cells × cell size —
    /// and nothing else. Under chafa this was a *cell count*, converted
    /// through a second program's idea of a cell, and that conversion is what
    /// used to shrink the cover to 0.8 × 0.75 of its box.
    #[test]
    fn the_encoder_is_asked_for_the_rects_pixels() {
        // `params` carries a 10×20 cell.
        let buf = SharedBuf::default();
        renderer(&buf).render_sixel(&params(1, (0, 0, 33, 18), Some(&[1])));
        assert!(sent(&buf).contains("SIXEL:330x360:"), "{}", sent(&buf));

        // The same box on Konsole's 8×15 cell is a smaller raster — the cell
        // size is the only thing that varies between terminals.
        let konsole = SharedBuf::default();
        let mut p = params(1, (0, 0, 33, 18), Some(&[1]));
        p.cell_px = (8, 15);
        renderer(&konsole).render_sixel(&p);
        assert!(
            sent(&konsole).contains("SIXEL:264x270:"),
            "{}",
            sent(&konsole)
        );
    }

    /// Degenerate geometry must stay encodable: a zero-pixel request asks the
    /// encoder for nothing at all, and a cover panel can be one cell wide
    /// before the layout refuses to draw it.
    #[test]
    fn a_degenerate_rect_never_asks_for_zero_pixels() {
        let buf = SharedBuf::default();
        let mut p = params(1, (0, 0, 0, 0), Some(&[1]));
        p.cell_px = (0, 0);
        renderer(&buf).render_sixel(&p);
        assert!(sent(&buf).contains("SIXEL:1x1:"), "{}", sent(&buf));
    }

    // ── The real encoder ──

    /// PNG bytes in, a SIXEL payload whose raster header names the exact box
    /// asked for. That property is what chafa could not be held to — it sized
    /// the raster from its own reading of the terminal.
    #[test]
    fn the_encoder_emits_the_requested_raster() {
        let out = IcySixelEncoder
            .encode(&png_cover(40, 30), (24, 40))
            .expect("encode a real PNG");
        let text = String::from_utf8_lossy(&out);
        assert!(text.starts_with("\x1bP"), "a DCS introducer opens it");
        assert!(
            text.contains("\"1;1;24;40"),
            "raster attributes must name the box: {:?}",
            &text[..40.min(text.len())]
        );
    }

    /// Cover bytes come out of other people's tags; a file that is not an
    /// image at all is an error to report, not a panic.
    #[test]
    fn the_encoder_rejects_bytes_that_are_not_an_image() {
        assert!(IcySixelEncoder.encode(b"not an image", (8, 8)).is_err());
    }

    /// A captioned PNG of the given size, built in memory.
    fn png_cover(w: u32, h: u32) -> Vec<u8> {
        let img =
            image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 5) as u8, (y * 7) as u8, 160]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("encode the test PNG");
        out.into_inner()
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

    /// Square cells are unusual but real (a wide font at a large size), and
    /// rejecting them would silently mis-size every cover on such a terminal.
    #[test]
    fn cell_size_accepts_square_cells() {
        assert_eq!(cell_px_from_window((80, 40, 1600, 800)), Some((20, 20)));
    }

    // ── Cell size probe (`CSI 16 t`) ──

    /// The direct answer: `CSI 6 ; <height> ; <width> t`, width and height in
    /// the order the cover code wants them.
    #[test]
    fn probe_reads_the_cell_size_report() {
        assert_eq!(parse_cell_size_reply(b"\x1b[6;26;12t"), Some((12, 26)));
    }

    /// The reply shares the queue with whatever else the terminal sent — a
    /// device-attributes answer, another report, the user typing during
    /// startup — and arrives in as many pieces as the tty feels like.
    #[test]
    fn probe_ignores_what_is_not_its_reply() {
        assert_eq!(parse_cell_size_reply(b"abc\x1b[6;17;8txyz"), Some((8, 17)));
        assert_eq!(
            parse_cell_size_reply(b"\x1b[?62;c\x1b[6;17;8t"),
            Some((8, 17)),
            "a device-attributes reply must not confuse it"
        );
        assert_eq!(parse_cell_size_reply(b"\x1b[6;2"), None, "half a reply");
        assert_eq!(parse_cell_size_reply(b"\x1b[6;26;"), None, "half a reply");
        assert_eq!(parse_cell_size_reply(b"\x1b[6;17;8"), None, "no final byte");
        assert_eq!(
            parse_cell_size_reply(b"\x1b[6;17;8u"),
            None,
            "not our report"
        );
        assert_eq!(
            parse_cell_size_reply(b"\x1b[6;0;0t"),
            None,
            "zero-sized cell"
        );
        assert_eq!(parse_cell_size_reply(b"\x1b[6;99999;8t"), None, "not a u16");
        assert_eq!(parse_cell_size_reply(b""), None);
    }

    /// Terminals that implement only the text-area reports still answer: the
    /// quotient of `CSI 4 t` and `CSI 8 t` is the same cell size.
    #[test]
    fn probe_derives_the_cell_from_the_text_area_reports() {
        // 800x480 px of text area over 80x24 cells is a 10x20 cell.
        assert_eq!(
            parse_cell_size_reply(b"\x1b[4;480;800t\x1b[8;24;80t"),
            Some((10, 20))
        );
        // Either half on its own says nothing.
        assert_eq!(parse_cell_size_reply(b"\x1b[4;480;800t"), None);
        assert_eq!(parse_cell_size_reply(b"\x1b[8;24;80t"), None);
        // The direct answer wins when both arrive.
        assert_eq!(
            parse_cell_size_reply(b"\x1b[4;480;800t\x1b[8;24;80t\x1b[6;26;12t"),
            Some((12, 26))
        );
    }

    // ── The probe's read loop, on a pipe ──
    //
    // `query_cell_px_on` is the only part of the probe that touches a live
    // terminal, and it does not care that the far end is one: all it needs is
    // bytes going out and a reply coming back. A pipe provides both — plus a
    // deterministic "nothing ever arrives" and a deterministic EOF, which no
    // terminal emulator will give you on demand.

    /// Owns a raw descriptor so a test closes it on every path out.
    struct Fd(RawFd);

    impl Fd {
        /// A connected pair: writing to `.1` makes `.0` readable.
        fn pipe() -> (Fd, Fd) {
            let mut fds = [0 as RawFd; 2];
            // SAFETY: `fds` has room for the two descriptors `pipe` writes.
            assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe failed");
            (Fd(fds[0]), Fd(fds[1]))
        }
    }

    impl Drop for Fd {
        fn drop(&mut self) {
            // SAFETY: the descriptor came from `pipe` and is closed once.
            unsafe { libc::close(self.0) };
        }
    }

    /// Write the whole buffer to a raw descriptor.
    fn write_fd(fd: RawFd, bytes: &[u8]) {
        // SAFETY: `bytes` is a valid readable buffer of `bytes.len()` bytes.
        let n = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        assert_eq!(n, bytes.len() as isize, "short write to the test pipe");
    }

    /// The happy path: the queries go out, the replies come back through
    /// `poll(2)` + `read(2)`, and both answers fall out of them. Konsole's real
    /// DA1 reply is the sixel line here.
    #[test]
    fn probe_asks_and_reads_the_answer() {
        let (read_end, write_end) = Fd::pipe();
        write_fd(write_end.0, b"\x1b[6;15;8t\x1b[?62;1;4c");

        let mut sent = Vec::new();
        let caps = query_terminal_on(&mut sent, read_end.0, Duration::from_secs(5));

        assert_eq!(caps.cell_px, Some((8, 15)));
        assert!(caps.sixel, "DA1 parameter 4 advertises sixel");
        assert_eq!(sent, TERMINAL_QUERY, "the query is what opens the exchange");
    }

    /// A tty is free to hand the reply over in pieces, and the pieces need not
    /// be split on report boundaries. Reading until the parse succeeds is what
    /// makes that a non-event.
    #[test]
    fn probe_joins_a_reply_split_across_reads() {
        let (read_end, write_end) = Fd::pipe();
        // The first half is in the pipe before the call, so the first read is
        // guaranteed to return only that much.
        write_fd(write_end.0, b"\x1b[6;1");
        let tail_fd = write_end.0;
        let tail = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            write_fd(tail_fd, b"5;8t\x1b[?62;1;4c");
        });

        let mut sent = Vec::new();
        let caps = query_terminal_on(&mut sent, read_end.0, Duration::from_secs(5));
        tail.join().expect("writer thread");

        assert_eq!(caps.cell_px, Some((8, 15)));
        assert!(caps.sixel);
    }

    /// A terminal that ignores the query costs the budget and nothing else —
    /// this is the case that has to stay bounded, since it runs at startup.
    #[test]
    fn probe_gives_up_when_nothing_answers() {
        let (read_end, _write_end) = Fd::pipe();

        let started = Instant::now();
        let mut sent = Vec::new();
        let caps = query_terminal_on(&mut sent, read_end.0, Duration::from_millis(40));
        let waited = started.elapsed();

        assert_eq!(caps, TerminalCaps::default());
        assert_eq!(sent, TERMINAL_QUERY, "it still asks");
        assert!(
            waited >= Duration::from_millis(20),
            "it waited for the answer instead of giving up on arrival: {waited:?}"
        );
        assert!(
            waited < Duration::from_secs(3),
            "the wait is bounded by the budget, not by the terminal: {waited:?}"
        );
    }

    /// DA1 is asked last and answered by every terminal, so a complete reply to
    /// it means the answers before it are already in hand — no reason to sit
    /// out the rest of the budget. This is what keeps the probe from adding
    /// startup latency on a terminal that answers only some of the queries.
    #[test]
    fn probe_stops_at_the_da1_barrier() {
        let (read_end, write_end) = Fd::pipe();
        write_fd(write_end.0, b"\x1b[?62;1;4c"); // no cell-size answer at all

        let started = Instant::now();
        let mut sent = Vec::new();
        let caps = query_terminal_on(&mut sent, read_end.0, Duration::from_secs(30));
        let waited = started.elapsed();

        assert_eq!(caps.cell_px, None, "it did not answer the cell query");
        assert!(caps.sixel);
        assert!(
            waited < Duration::from_secs(3),
            "the DA1 reply should end the wait: {waited:?}"
        );
    }

    /// The negative answer is an answer: a terminal that lists its attributes
    /// without the sixel one must not be sent a graphics payload.
    #[test]
    fn probe_reports_no_sixel_when_da1_omits_it() {
        let (read_end, write_end) = Fd::pipe();
        write_fd(write_end.0, b"\x1b[6;15;8t\x1b[?62;1;2;6;9;15;18;21;22;29c");

        let mut sent = Vec::new();
        let caps = query_terminal_on(&mut sent, read_end.0, Duration::from_secs(5));

        assert_eq!(caps.cell_px, Some((8, 15)));
        assert!(!caps.sixel);
    }

    // ── DA1 parsing ──

    /// Attribute 4 is xterm's "Sixel graphics". Konsole answers exactly this;
    /// without it tmper would be writing DCS payloads at a terminal that has no
    /// idea what to do with them.
    #[test]
    fn da1_sixel_attribute_is_read() {
        assert_eq!(parse_da1_sixel(b"\x1b[?62;1;4c"), Some(true));
        assert_eq!(parse_da1_sixel(b"\x1b[?62;4c"), Some(true));
        // foot 1.27's reply, real and complete.
        assert_eq!(parse_da1_sixel(b"\x1b[?62;4;22;28;52c"), Some(true));
    }

    /// A terminal that answers without the attribute is not a sixel terminal —
    /// and "24" is not "4": the parameters are matched as numbers, not as
    /// substrings.
    #[test]
    fn da1_without_the_attribute_is_a_no() {
        assert_eq!(parse_da1_sixel(b"\x1b[?1;2c"), Some(false));
        assert_eq!(
            parse_da1_sixel(b"\x1b[?62;1;2;6;9;15;18;21;22;29c"),
            Some(false)
        );
        assert_eq!(parse_da1_sixel(b"\x1b[?62;24;1c"), Some(false));
        assert_eq!(parse_da1_sixel(b"\x1b[?62;c"), Some(false));
    }

    /// `None` — no reply yet — is deliberately not `Some(false)`: the read
    /// loop uses the difference to decide whether it may stop waiting.
    #[test]
    fn an_incomplete_da1_reply_is_not_an_answer() {
        assert_eq!(parse_da1_sixel(b""), None);
        assert_eq!(parse_da1_sixel(b"\x1b[?62;1;4"), None, "no final byte");
        assert_eq!(parse_da1_sixel(b"\x1b[6;15;8t"), None, "another report");
        assert_eq!(parse_da1_sixel(b"abc"), None);
    }

    /// The reply shares the queue with the cell-size reports and whatever else
    /// the terminal sent, in any order.
    #[test]
    fn da1_is_found_among_other_traffic() {
        assert_eq!(
            parse_da1_sixel(b"\x1b[6;15;8t\x1b[?62;1;4c\x1b[4;480;800t"),
            Some(true)
        );
        assert_eq!(
            parse_da1_sixel(b"\x1b[?62;1;4c\x1b[6;15;8t"),
            Some(true),
            "DA1 first is not how a terminal answers, but it parses"
        );
    }

    /// stdin at EOF (a closed pty, a multiplexer that went away) reports
    /// readable forever. Waiting out the full budget there would be pure
    /// startup latency.
    #[test]
    fn probe_returns_promptly_at_eof() {
        let (read_end, write_end) = Fd::pipe();
        drop(write_end);

        let started = Instant::now();
        let mut sent = Vec::new();
        let caps = query_terminal_on(&mut sent, read_end.0, Duration::from_secs(30));
        let waited = started.elapsed();

        assert_eq!(caps, TerminalCaps::default());
        assert!(
            waited < Duration::from_secs(5),
            "EOF ended the wait instead of the budget: {waited:?}"
        );
    }

    // ── Block-layer suppression ──

    /// The UI stops drawing blocks once a native image is up. That rewrite
    /// lands on top of the graphics layer, so the payload has to go out again
    /// on the same frame or the cover disappears.
    #[test]
    fn suppressing_the_block_layer_resends_the_sixel() {
        let buf = SharedBuf::default();
        let mut r = renderer(&buf);
        r.render_sixel(&params(1, (0, 0, 10, 10), Some(&[1])));
        let placed = written(&buf);
        assert!(r.native_active(), "a payload was sent");

        // Blocks still drawn, nothing changed → the image persists.
        r.render_sixel(&params(1, (0, 0, 10, 10), Some(&[1])));
        assert_eq!(written(&buf), placed, "no change, no re-send");

        // The UI suppressed the blocks this frame: ratatui wrote over the
        // cover rect, so the payload is sent again.
        let mut suppressed = params(1, (0, 0, 10, 10), Some(&[1]));
        suppressed.blocks_suppressed = true;
        r.render_sixel(&suppressed);
        assert!(
            written(&buf) > placed,
            "the rewritten cells erase the SIXEL"
        );

        // ...and then it settles: no further re-sends while nothing changes.
        let settled = written(&buf);
        r.render_sixel(&suppressed);
        assert_eq!(written(&buf), settled, "stable once the blocks are gone");
    }

    /// `native_active` is what the UI consults, so it must be false whenever
    /// the encoder produced nothing — otherwise the fallback art is dropped
    /// for an image that never arrives.
    #[test]
    fn native_active_is_false_when_the_encode_produced_nothing() {
        let buf = SharedBuf::default();
        let mut r = CoverRenderer::with_writer(Box::new(buf.clone()), Box::new(EmptyEncoder));
        r.render_sixel(&params(1, (0, 0, 10, 10), Some(&[1])));
        assert!(!r.native_active());
    }

    /// Leaving the player view clears the image, and `native_active` has to
    /// follow so the blocks come back on the next frame that shows a cover.
    #[test]
    fn native_active_tracks_the_clear_on_view_change() {
        let buf = SharedBuf::default();
        let mut r = renderer(&buf);
        r.render_sixel(&params(1, (0, 0, 10, 10), Some(&[1])));
        assert!(r.native_active());

        let mut away = params(1, (0, 0, 10, 10), Some(&[1]));
        away.active_view = ViewMode::Library;
        r.render_sixel(&away);
        assert!(!r.native_active());
    }

    // ── Failure handling ──

    #[test]
    fn empty_sixel_output_is_not_retried_every_frame() {
        let buf = SharedBuf::default();
        let mut r = CoverRenderer::with_writer(Box::new(buf.clone()), Box::new(EmptyEncoder));
        r.render_sixel(&params(1, (0, 0, 10, 10), Some(&[1])));
        let first = written(&buf);
        assert_eq!(first, 0, "empty payload writes nothing");

        // Terminal rejects SIXEL → must not re-encode on every frame.
        r.render_sixel(&params(1, (0, 0, 10, 10), Some(&[1])));
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
    /// flag is switched on directly — the same way `sixel_available` is.
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
        let mut r = renderer(&buf); // the SIXEL path (kitty off)

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
