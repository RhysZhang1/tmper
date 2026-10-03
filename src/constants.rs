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

    /// Pixel size chafa uses for one cell of a SIXEL payload.
    ///
    /// chafa cannot see this terminal (all three stdio streams are pipes, and
    /// `--probe off` disables the escape-sequence query), so its SIXEL geometry
    /// is a pure function of `--size`: 20 px per requested column, 20 px per
    /// requested row rounded down to a multiple of 6. Measured against chafa
    /// 1.18 — `-s 20x10` yields 400×198 px whatever the real terminal reports,
    /// and `--font-ratio` does not change it. We therefore ask for the number
    /// of chafa-cells that spans the cover rect's *pixels*, not its cells.
    pub const CHAFA_SIXEL_CELL_PX: u32 = 20;

    /// Cell pixel size assumed when the terminal does not report one
    /// (`window_size()` returns 0 — tmux, some multiplexers). 10×20 is the
    /// common xterm/Konsole cell and the model the Kitty path already used.
    pub const FALLBACK_CELL_PX: (u16, u16) = (10, 20);

    // ── FFT ──

    /// Number of PCM samples fed into the FFT analyzer per frame.
    pub const FFT_SIZE: usize = 2048;

    /// Sleep interval (ms) in the FFT background thread's main loop.
    pub const FFT_LOOP_SLEEP_MS: u64 = 32;

    /// Sleep interval (ms) when the FFT thread has insufficient samples.
    pub const FFT_WAIT_SLEEP_MS: u64 = 16;
}
