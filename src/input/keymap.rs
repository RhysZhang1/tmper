#![allow(dead_code)]
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;

/// Parse a keybinding string into a crossterm KeyEvent.
/// Supported formats: single char ("j"), "^X" for Ctrl+X, special names.
pub fn parse_key_str(s: &str) -> KeyEvent {
    let s = s.trim();
    if s.is_empty() {
        return KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
    }
    // Ctrl+<char> via caret notation
    if s.len() == 2 && s.starts_with('^') {
        let c = s.chars().nth(1).unwrap();
        return KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    }
    // Single char
    if s.len() == 1 {
        let c = s.chars().next().unwrap();
        return KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    }
    // Special names
    match s.to_lowercase().as_str() {
        "space" => KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
        "enter" => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        "esc" | "escape" => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        "backspace" => KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        "tab" => KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        "up" => KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        "down" => KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        "left" => KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
        "right" => KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        _ => KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
    }
}

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
