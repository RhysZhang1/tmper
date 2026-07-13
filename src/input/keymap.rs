#![allow(dead_code)] // WIP: public API not yet wired to UI
#[allow(dead_code)]
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct KeyBindings {
    #[serde(default = "default_key")]
    pub play_pause: String,
    #[serde(default = "default_key")]
    pub stop: String,
    #[serde(default = "default_next")]
    pub next_track: String,
    #[serde(default = "default_prev")]
    pub prev_track: String,
    #[serde(default = "default_vol_down")]
    pub vol_down: String,
    #[serde(default = "default_vol_up")]
    pub vol_up: String,
    #[serde(default = "default_quit")]
    pub quit: String,
    #[serde(default = "default_up")]
    pub up: String,
    #[serde(default = "default_down")]
    pub down: String,
}

fn default_key() -> String {
    " ".into()
}
fn default_next() -> String {
    "n".into()
}
fn default_prev() -> String {
    "p".into()
}
fn default_vol_down() -> String {
    "-".into()
}
fn default_vol_up() -> String {
    "=".into()
}
fn default_quit() -> String {
    "q".into()
}
fn default_up() -> String {
    "k".into()
}
fn default_down() -> String {
    "j".into()
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            play_pause: " ".into(),
            stop: "s".into(),
            next_track: "n".into(),
            prev_track: "p".into(),
            vol_down: "-".into(),
            vol_up: "=".into(),
            quit: "q".into(),
            up: "k".into(),
            down: "j".into(),
        }
    }
}

impl KeyBindings {
    pub fn load() -> Self {
        let path = crate::paths::config_dir().join("keybindings.toml");
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(bindings) = toml::from_str(&content) {
                    tracing::info!("Loaded keybindings from {:?}", path);
                    return bindings;
                }
            }
        }
        Self::default()
    }
}
