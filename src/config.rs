use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlaybackConfig {
    #[serde(default = "default_volume", serialize_with = "serialize_f32_rounded")]
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
    #[serde(
        default = "default_smoothing",
        serialize_with = "serialize_f32_rounded"
    )]
    pub smoothing: f32,
}

/// Write an `f32` knob at two decimals.
///
/// `serde_json`/`toml` widen an `f32` to `f64` before printing, which exposes
/// the exact binary value: saving the settings view produced
/// `default_volume = 0.10000000149011612`. The value is identical either way,
/// but the file is meant to be hand-edited, so it should read like one.
/// Rounding in `f64` space is what makes `0.1` print as `0.1`; rounding the
/// `f32` first would just reproduce the artifact.
fn serialize_f32_rounded<S: serde::Serializer>(
    value: &f32,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_f64((f64::from(*value) * 100.0).round() / 100.0)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The template is what a first run writes to `config.toml`, so it has to
    /// agree with the compiled-in defaults — otherwise a fresh install and a
    /// reset one would behave differently. Nothing enforced that before.
    #[test]
    fn shipped_template_matches_the_code_defaults() {
        let template: Config =
            toml::from_str(include_str!("../config/default.toml")).expect("template parses");
        let defaults = Config::default();

        assert_eq!(
            template.playback.default_volume,
            defaults.playback.default_volume
        );
        assert_eq!(
            template.playback.seek_step_small_secs,
            defaults.playback.seek_step_small_secs
        );
        assert_eq!(template.visualizer.num_bars, defaults.visualizer.num_bars);
        assert_eq!(
            template.visualizer.frame_rate,
            defaults.visualizer.frame_rate
        );
        assert_eq!(template.visualizer.smoothing, defaults.visualizer.smoothing);
        assert_eq!(template.ui.theme, defaults.ui.theme);
        assert_eq!(template.ui.show_cover_art, defaults.ui.show_cover_art);
    }

    // ── clamp ──

    /// A zero here divides at runtime, so the floor is the load-bearing part.
    #[test]
    fn clamp_raises_hostile_values_to_the_floor() {
        let mut config = Config {
            playback: PlaybackConfig {
                default_volume: -1.0,
                ..Default::default()
            },
            visualizer: VisualizerConfig {
                num_bars: 0,
                frame_rate: 0,
                smoothing: -2.0,
            },
            ui: UiConfig::default(),
        };

        config.clamp();

        assert_eq!(config.playback.default_volume, 0.0);
        assert_eq!(config.visualizer.num_bars, 1, "0 bars would divide by zero");
        assert_eq!(
            config.visualizer.frame_rate, 1,
            "0 frame rate would divide by zero"
        );
        assert_eq!(config.visualizer.smoothing, 0.0);
    }

    #[test]
    fn clamp_lowers_values_above_the_ceiling() {
        let mut config = Config {
            playback: PlaybackConfig {
                default_volume: 3.0,
                ..Default::default()
            },
            visualizer: VisualizerConfig {
                num_bars: 10_000,
                frame_rate: 10_000,
                smoothing: 2.0,
            },
            ui: UiConfig::default(),
        };

        config.clamp();

        assert_eq!(config.playback.default_volume, 1.0);
        assert_eq!(config.visualizer.num_bars, 256);
        assert_eq!(config.visualizer.frame_rate, 120);
        assert_eq!(config.visualizer.smoothing, 1.0);
    }

    #[test]
    fn clamp_leaves_usable_values_alone() {
        let mut config = Config::default();
        let before = config.clone();
        config.clamp();

        assert_eq!(
            config.playback.default_volume,
            before.playback.default_volume
        );
        assert_eq!(config.visualizer.num_bars, before.visualizer.num_bars);
        assert_eq!(config.visualizer.frame_rate, before.visualizer.frame_rate);
        assert_eq!(config.visualizer.smoothing, before.visualizer.smoothing);
    }

    // ── Serialization ──

    /// `f32` widens to `f64` when serialized, which exposes its exact binary
    /// value: this used to write `0.10000000149011612` for a volume of 0.1.
    /// The file is meant to be hand-edited, so it has to read like one.
    #[test]
    fn floats_are_written_at_two_decimals() {
        let mut config = Config::default();
        config.playback.default_volume = 0.1;
        config.visualizer.smoothing = 0.55;

        let text = toml::to_string_pretty(&config).expect("serializes");

        assert!(text.contains("default_volume = 0.1\n"), "got:\n{text}");
        assert!(text.contains("smoothing = 0.55\n"), "got:\n{text}");
        assert!(
            !text.contains("0.10000000149011612"),
            "the f32 artifact leaked into the file:\n{text}"
        );
    }

    /// Rounding must not change the value: the file is read back into `f32`.
    #[test]
    fn rounded_floats_round_trip_to_the_same_f32() {
        for value in [0.0f32, 0.1, 0.35, 0.55, 0.8, 1.0] {
            let mut config = Config::default();
            config.playback.default_volume = value;
            let text = toml::to_string_pretty(&config).expect("serializes");
            let parsed: Config = toml::from_str(&text).expect("parses");
            assert_eq!(
                parsed.playback.default_volume, value,
                "volume {value} did not survive the round trip"
            );
        }
    }

    #[test]
    fn every_field_survives_a_round_trip() {
        let mut config = Config::default();
        config.playback.default_volume = 0.37;
        config.playback.seek_step_small_secs = 12;
        config.visualizer.num_bars = 48;
        config.visualizer.frame_rate = 60;
        config.visualizer.smoothing = 0.42;
        config.ui.theme = "dracula".into();
        config.ui.show_cover_art = false;

        let text = toml::to_string_pretty(&config).expect("serializes");
        let parsed: Config = toml::from_str(&text).expect("parses");

        assert_eq!(parsed.playback.default_volume, 0.37);
        assert_eq!(parsed.playback.seek_step_small_secs, 12);
        assert_eq!(parsed.visualizer.num_bars, 48);
        assert_eq!(parsed.visualizer.frame_rate, 60);
        assert_eq!(parsed.visualizer.smoothing, 0.42);
        assert_eq!(parsed.ui.theme, "dracula");
        assert!(!parsed.ui.show_cover_art);
    }

    // ── Deserialization ──

    /// Every field carries `#[serde(default)]`, so a document that mentions
    /// only one knob keeps the compiled-in value for the rest. This is the
    /// same parsing `load_or_default` performs, exercised without the file.
    #[test]
    fn a_partial_document_fills_in_the_defaults() {
        let parsed: Config = toml::from_str("[visualizer]\nnum_bars = 64\n").expect("parses");

        assert_eq!(parsed.visualizer.num_bars, 64);
        assert_eq!(
            parsed.visualizer.frame_rate,
            Config::default().visualizer.frame_rate
        );
        assert_eq!(
            parsed.playback.default_volume,
            Config::default().playback.default_volume
        );
        assert_eq!(parsed.ui.theme, Config::default().ui.theme);
    }

    #[test]
    fn an_empty_document_is_all_defaults() {
        let parsed: Config = toml::from_str("").expect("parses");
        assert_eq!(parsed.ui.theme, Config::default().ui.theme);
    }

    /// Unknown keys are ignored rather than rejected, so a config written by
    /// a newer build still loads.
    #[test]
    fn unknown_keys_are_ignored() {
        let parsed: Config =
            toml::from_str("[ui]\ntheme = \"nord\"\nsomething_new = 1\n").expect("parses");
        assert_eq!(parsed.ui.theme, "nord");
    }
}
