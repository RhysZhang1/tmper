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

    // ── FFT ──

    /// Number of PCM samples fed into the FFT analyzer per frame.
    pub const FFT_SIZE: usize = 2048;

    /// Sleep interval (ms) in the FFT background thread's main loop.
    pub const FFT_LOOP_SLEEP_MS: u64 = 32;

    /// Sleep interval (ms) when the FFT thread has insufficient samples.
    pub const FFT_WAIT_SLEEP_MS: u64 = 16;
}
