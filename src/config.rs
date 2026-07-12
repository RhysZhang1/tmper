use serde::Deserialize;

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_volume")]
    pub default_volume: f32,

    #[serde(default = "default_seek_step")]
    pub seek_step_small_secs: u32,
}

fn default_volume() -> f32 {
    0.8
}

fn default_seek_step() -> u32 {
    5
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

impl Default for Config {
    fn default() -> Self {
        Self {
            default_volume: default_volume(),
            seek_step_small_secs: default_seek_step(),
        }
    }
}
