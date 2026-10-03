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

    /// Maximum base64 payload per Kitty graphics escape sequence.
    ///
    /// The protocol's limit is 4096 bytes for the *whole* sequence, header
    /// included, so the payload is held a little under it. Konsole accepts
    /// exactly 4096 and kitty is lenient, but the spec's number is the one
    /// that stays valid everywhere.
    pub const KITTY_CHUNK_SIZE: usize = 4000;

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

    // ── Daemon ──

    /// How long the daemon stays alive with no client attached and nothing
    /// playing before it exits.
    ///
    /// Idle is exactly that: nobody attached, and no sound being made. A pause
    /// counts — it holds a place rather than doing anything, and a daemon that
    /// counted it as work would never exit at all, since a pause can be left
    /// standing for days. The place is not lost with it: `state.json` carries
    /// the queue and the position, and the next run comes back parked on the
    /// same track at the same second.
    pub const DAEMON_IDLE_EXIT_SECS: u64 = 300;

    /// Messages one client may be behind by before the daemon gives up on it.
    ///
    /// The steady state is a handful: snapshots and spectrum frames arrive
    /// together at the frame rate, and the client drains both every tick. This
    /// is the backstop for a client that has stopped reading altogether, and
    /// it is sized so that reaching it means seconds of total silence, not a
    /// slow frame.
    pub const DAEMON_CLIENT_QUEUE: usize = 256;

    /// How long a freshly spawned daemon is given to answer on its socket
    /// before the client gives up and reports the failure.
    pub const DAEMON_START_TIMEOUT_MS: u64 = 3000;

    /// Gap between connection attempts while waiting for a daemon to come up.
    pub const DAEMON_CONNECT_RETRY_MS: u64 = 25;

    /// How long the client waits for `Event::Welcome` after sending `Hello`.
    ///
    /// A daemon that accepts a connection and then says nothing is
    /// indistinguishable from a hung one, and the TUI must not start into a
    /// frozen state because of it.
    pub const DAEMON_HELLO_TIMEOUT_MS: u64 = 2000;

    // ── FFT ──

    /// Number of PCM samples fed into the FFT analyzer per frame.
    pub const FFT_SIZE: usize = 2048;

    /// Sleep interval (ms) in the FFT background thread's main loop.
    pub const FFT_LOOP_SLEEP_MS: u64 = 32;

    /// Sleep interval (ms) when the FFT thread has insufficient samples.
    pub const FFT_WAIT_SLEEP_MS: u64 = 16;
}
