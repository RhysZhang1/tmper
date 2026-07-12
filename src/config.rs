use serde::Deserialize;

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct LibraryConfig {
    #[serde(default = "default_music_dirs")]
    pub music_dirs: Vec<String>,

    #[serde(default = "default_extensions")]
    pub extensions: Vec<String>,

    #[serde(default)]
    pub scan_on_startup: bool,

    #[serde(default = "default_true")]
    pub follow_symlinks: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct PlaybackConfig {
    #[serde(default = "default_volume")]
    pub default_volume: f32,

    #[serde(default = "default_true")]
    pub gapless: bool,

    #[serde(default = "default_crossfade")]
    pub crossfade_seconds: u32,

    #[serde(default = "default_true")]
    pub resume_on_startup: bool,

    #[serde(default = "default_seek_step_small")]
    pub seek_step_small_secs: u32,

    #[serde(default = "default_seek_step_large")]
    pub seek_step_large_secs: u32,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct VisualizerConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default = "default_num_bars")]
    pub num_bars: u32,

    #[serde(default = "default_frame_rate")]
    pub frame_rate: u32,

    #[serde(default = "default_smoothing")]
    pub smoothing: f32,

    #[serde(default = "default_char_set")]
    pub char_set: String,

    #[serde(default = "default_color_scheme")]
    pub color_scheme: String,

    #[serde(default = "default_true")]
    pub show_on_idle: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct LyricsConfig {
    #[serde(default = "default_true")]
    pub auto_load: bool,

    #[serde(default = "default_encoding_fallbacks")]
    pub encoding_fallbacks: Vec<String>,

    #[serde(default = "default_display_lines")]
    pub display_lines_before: u32,

    #[serde(default = "default_true")]
    pub lrc_search_embedded: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_theme")]
    pub theme: String,

    #[serde(default = "default_true")]
    pub show_progress_bar: bool,

    #[serde(default = "default_true")]
    pub show_cover_art: bool,

    #[serde(default = "default_cover_art_width")]
    pub cover_art_max_width: u32,

    #[serde(default = "default_view")]
    pub default_view: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub library: LibraryConfig,

    #[serde(default)]
    pub playback: PlaybackConfig,

    #[serde(default)]
    pub visualizer: VisualizerConfig,

    #[serde(default)]
    pub lyrics: LyricsConfig,

    #[serde(default)]
    pub ui: UiConfig,
}

// ── Default value functions ──
#[allow(dead_code)]
fn default_true() -> bool {
    true
}

#[allow(dead_code)]
fn default_volume() -> f32 {
    0.8
}

#[allow(dead_code)]
fn default_seek_step_small() -> u32 {
    5
}

#[allow(dead_code)]
fn default_seek_step_large() -> u32 {
    30
}

#[allow(dead_code)]
fn default_crossfade() -> u32 {
    2
}

#[allow(dead_code)]
fn default_music_dirs() -> Vec<String> {
    vec!["~/Music".to_string()]
}

#[allow(dead_code)]
fn default_extensions() -> Vec<String> {
    vec![
        "mp3".into(),
        "flac".into(),
        "ogg".into(),
        "opus".into(),
        "wav".into(),
        "aac".into(),
        "m4a".into(),
        "ape".into(),
        "wv".into(),
        "aiff".into(),
        "wma".into(),
    ]
}

#[allow(dead_code)]
fn default_num_bars() -> u32 {
    32
}

#[allow(dead_code)]
fn default_frame_rate() -> u32 {
    30
}

#[allow(dead_code)]
fn default_smoothing() -> f32 {
    0.35
}

#[allow(dead_code)]
fn default_char_set() -> String {
    "blocks".into()
}

#[allow(dead_code)]
fn default_color_scheme() -> String {
    "gradient".into()
}

#[allow(dead_code)]
fn default_encoding_fallbacks() -> Vec<String> {
    vec!["utf-8".into(), "gbk".into(), "shift-jis".into()]
}

#[allow(dead_code)]
fn default_display_lines() -> u32 {
    4
}

#[allow(dead_code)]
fn default_theme() -> String {
    "tokyo-night".into()
}

#[allow(dead_code)]
fn default_view() -> String {
    "player".into()
}

#[allow(dead_code)]
fn default_cover_art_width() -> u32 {
    25
}

#[allow(dead_code)]
impl Config {
    pub fn load_or_default() -> Self {
        let config_path = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("termusic")
            .join("config.toml");

        if config_path.exists() {
            match std::fs::read_to_string(&config_path) {
                Ok(content) => match toml::from_str(&content) {
                    Ok(config) => {
                        tracing::info!("Loaded config from {:?}", config_path);
                        return config;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse config, using defaults: {e}");
                    }
                },
                Err(e) => {
                    tracing::warn!("Failed to read config file, using defaults: {e}");
                }
            }
        } else {
            tracing::info!("No config file found, using defaults");
        }

        Self::default()
    }
}

impl Default for LibraryConfig {
    fn default() -> Self {
        Self {
            music_dirs: default_music_dirs(),
            extensions: default_extensions(),
            scan_on_startup: false,
            follow_symlinks: true,
        }
    }
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            default_volume: default_volume(),
            gapless: true,
            crossfade_seconds: 2,
            resume_on_startup: true,
            seek_step_small_secs: 5,
            seek_step_large_secs: 30,
        }
    }
}

impl Default for VisualizerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            num_bars: 32,
            frame_rate: 30,
            smoothing: 0.35,
            char_set: "blocks".into(),
            color_scheme: "gradient".into(),
            show_on_idle: true,
        }
    }
}

impl Default for LyricsConfig {
    fn default() -> Self {
        Self {
            auto_load: true,
            encoding_fallbacks: default_encoding_fallbacks(),
            display_lines_before: 4,
            lrc_search_embedded: true,
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "tokyo-night".into(),
            show_progress_bar: true,
            show_cover_art: true,
            cover_art_max_width: 25,
            default_view: "player".into(),
        }
    }
}
