use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaybackConfig {
    #[serde(default = "default_volume")]
    pub default_volume: f32,
    #[serde(default = "default_seek_step_small")]
    pub seek_step_small_secs: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VisualizerConfig {
    #[serde(default = "default_num_bars")]
    pub num_bars: u32,
    #[serde(default = "default_frame_rate")]
    pub frame_rate: u32,
    #[serde(default = "default_smoothing")]
    pub smoothing: f32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UiConfig {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_true")]
    pub show_cover_art: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Config {
    #[serde(default)]
    pub playback: PlaybackConfig,
    #[serde(default)]
    pub visualizer: VisualizerConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

fn default_true() -> bool {
    true
}
fn default_volume() -> f32 {
    0.8
}
fn default_seek_step_small() -> u32 {
    5
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
fn default_theme() -> String {
    "tokyo-night".into()
}

impl Config {
    /// Ensure an editable XDG config exists using the built-in template.
    pub fn ensure_config_file() {
        let dir = crate::paths::config_dir();
        std::fs::create_dir_all(&dir).ok();
        let cfg = dir.join("config.toml");
        if !cfg.exists() {
            match std::fs::write(&cfg, include_str!("../config/default.toml")) {
                Ok(_) => tracing::info!("Generated config.toml from default.toml"),
                Err(e) => tracing::warn!("Failed to generate config.toml: {e}"),
            }
        }
    }

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

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            default_volume: default_volume(),
            seek_step_small_secs: default_seek_step_small(),
        }
    }
}
impl Default for VisualizerConfig {
    fn default() -> Self {
        Self {
            num_bars: default_num_bars(),
            frame_rate: default_frame_rate(),
            smoothing: default_smoothing(),
        }
    }
}
impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            show_cover_art: default_true(),
        }
    }
}
