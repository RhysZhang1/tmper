/// Centralized constants for runtime tuning parameters.
///
/// Values that appear in multiple call sites or represent a configuration
/// knob that doesn't belong in a user-facing config file live here.
pub mod runtime {
    // ── Audio ──

    /// Capacity of the PCM ring buffer shared between the rodio source and
    /// the FFT thread (in `f32` samples).
    pub const PCM_BUFFER_CAPACITY: usize = 32768;

    /// Default sample rate used when Symphonia can't determine the rate from
    /// the container / codec params.
    pub const DEFAULT_SAMPLE_RATE: u32 = 44100;

    /// Maximum decoded audio queued ahead of the device.
    pub const AUDIO_PREBUFFER_SECS: u32 = 2;

    pub const AUDIO_BACKPRESSURE_SLEEP_MS: u64 = 10;
    pub const AUDIO_COMPLETION_POLL_MS: u64 = 20;

    // ── Keyboard ──

    /// Timeout window (ms) for detecting double-key sequences like `gg` / `dd`.
    pub const KEY_TIMEOUT_MS: u64 = 200;

    /// Single-step volume delta used by the global up/down keybindings.
    pub const VOLUME_STEP: f32 = 0.05;

    /// Rows treated as the visible page height for scroll/navigation
    /// (Ctrl+d / Ctrl+u move by half of this).
    pub const VISIBLE_ROWS: u16 = 10;

    // ── Notifications ──

    /// How long a mode-change / action notification stays visible (seconds).
    pub const NOTIFICATION_DURATION_SECS: f64 = 0.5;

    /// How long a playlist-operation notification stays visible (seconds).
    pub const PLAYLIST_NOTIFICATION_SECS: f64 = 1.0;

    // ── Cover art ──

    /// Maximum chunk size (bytes) for Kitty graphics protocol payloads.
    /// Kitty terminals recommend keeping payloads ≤ 4 KiB per escape sequence.
    pub const KITTY_CHUNK_SIZE: usize = 4096;

    /// Color registers the in-process SIXEL encoder may use.
    ///
    /// SIXEL addresses colors with 8 bits, so 256 is the format's own ceiling —
    /// which is why replacing chafa changed no color budget: chafa's `-c full`
    /// emits at most 255 registers on this path regardless (measured; see
    /// `progress/2026-10-03-encoder-and-terminal-compat.md`).
    pub const SIXEL_MAX_COLORS: u16 = 256;

    /// Floyd–Steinberg error-diffusion strength for the SIXEL encoder, 0.0–1.0.
    ///
    /// 0.875 (7/8) is the encoder's own default and its recommendation for
    /// photographs with smooth gradients, which is what cover art is. Lower
    /// values trade banding for sharper edges on flat graphics.
    pub const SIXEL_DIFFUSION: f32 = 0.875;

    /// Cell pixel size assumed when the terminal reports one through neither
    /// `TIOCGWINSZ` nor `CSI 16 t` (tmux, some multiplexers).
    ///
    /// The cover box needs the cell's *shape* to keep the artwork's aspect, and
    /// the SIXEL payload is sized in these same pixels — so this number is now
    /// used on one side only, by the one process that draws the cover.
    pub const FALLBACK_CELL_PX: (u16, u16) = (10, 20);

    // ── FFT ──

    /// Number of PCM samples fed into the FFT analyzer per frame.
    pub const FFT_SIZE: usize = 2048;

    /// Sleep interval (ms) in the FFT background thread's main loop.
    pub const FFT_LOOP_SLEEP_MS: u64 = 32;

    /// Sleep interval (ms) when the FFT thread has insufficient samples.
    pub const FFT_WAIT_SLEEP_MS: u64 = 16;
}
