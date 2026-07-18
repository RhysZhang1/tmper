use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
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

fn default_true() -> bool {
    true
}
fn default_volume() -> f32 {
    0.8
}
fn default_crossfade() -> u32 {
    2
}
fn default_seek_step_small() -> u32 {
    5
}
fn default_seek_step_large() -> u32 {
    30
}
fn default_music_dirs() -> Vec<String> {
    vec!["~/Music".to_string()]
}
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
fn default_num_bars() -> u32 {
    32
}
fn default_frame_rate() -> u32 {
    30
}
fn default_smoothing() -> f32 {
    0.35
}
fn default_char_set() -> String {
    "blocks".into()
}
fn default_color_scheme() -> String {
    "gradient".into()
}
fn default_encoding_fallbacks() -> Vec<String> {
    vec!["utf-8".into(), "gbk".into(), "shift-jis".into()]
}
fn default_display_lines() -> u32 {
    4
}
fn default_theme() -> String {
    "tokyo-night".into()
}
fn default_view() -> String {
    "player".into()
}
fn default_cover_art_width() -> u32 {
    25
}

impl Config {
    pub fn load_or_default() -> Self {
        let config_path = crate::paths::config_dir().join("config.toml");
        let mut config = if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(cfg) = toml::from_str(&content) {
                    tracing::info!("Loaded config from {:?}", config_path);
                    cfg
                } else {
                    tracing::warn!("Invalid config file at {:?}, using defaults", config_path);
                    Self::default()
                }
            } else {
                tracing::info!("No config file at {:?}, using defaults", config_path);
                Self::default()
            }
        } else {
            tracing::info!("No config file at {:?}, using defaults", config_path);
            Self::default()
        };
        config.clamp();
        config
    }

    /// Clamp critical fields to safe ranges to prevent runtime panics
    /// (e.g. divide-by-zero on frame_rate=0).
    fn clamp(&mut self) {
        self.playback.default_volume = self.playback.default_volume.clamp(0.0, 1.0);
        self.visualizer.frame_rate = self.visualizer.frame_rate.clamp(1, 120);
        self.visualizer.num_bars = self.visualizer.num_bars.clamp(1, 256);
        self.visualizer.smoothing = self.visualizer.smoothing.clamp(0.0, 1.0);
    }
}

impl Default for LibraryConfig {
    fn default() -> Self {
        Self {
            music_dirs: default_music_dirs(),
            extensions: default_extensions(),
            scan_on_startup: false,
            follow_symlinks: default_true(),
        }
    }
}
impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            default_volume: default_volume(),
            gapless: default_true(),
            crossfade_seconds: default_crossfade(),
            resume_on_startup: default_true(),
            seek_step_small_secs: default_seek_step_small(),
            seek_step_large_secs: default_seek_step_large(),
        }
    }
}
impl Default for VisualizerConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            num_bars: default_num_bars(),
            frame_rate: default_frame_rate(),
            smoothing: default_smoothing(),
            char_set: default_char_set(),
            color_scheme: default_color_scheme(),
            show_on_idle: default_true(),
        }
    }
}
impl Default for LyricsConfig {
    fn default() -> Self {
        Self {
            auto_load: default_true(),
            encoding_fallbacks: default_encoding_fallbacks(),
            display_lines_before: default_display_lines(),
            lrc_search_embedded: default_true(),
        }
    }
}
impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            show_progress_bar: default_true(),
            show_cover_art: default_true(),
            cover_art_max_width: default_cover_art_width(),
            default_view: default_view(),
        }
    }
}
