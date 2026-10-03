use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;

/// Parse a keybinding string into a crossterm KeyEvent.
///
/// Supported formats: single char ("j", any script), "^X" for Ctrl+X, and the
/// special names below. Counting is done in *characters*: a byte-length check
/// is false for every non-ASCII key, so a multi-byte binding used to skip the
/// single-char branch and land in the fallback — silently becoming Space, and
/// colliding with play/pause.
pub fn parse_key_str(s: &str) -> KeyEvent {
    let s = s.trim();
    if s.is_empty() {
        return KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
    }
    let mut chars = s.chars();
    let first = chars.next().expect("string is non-empty");
    let second = chars.next();

    // Ctrl+<char> via caret notation.
    if first == '^' {
        if let Some(c) = second {
            if chars.next().is_none() {
                return KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
            }
        }
    }
    // A single character, in any script.
    if second.is_none() {
        return KeyEvent::new(KeyCode::Char(first), KeyModifiers::NONE);
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
        other => {
            // Falling back to Space silently is how a typo ends up quietly
            // hijacking the play/pause key. Say so instead.
            tracing::warn!("Unrecognised keybinding '{other}' — using Space");
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyBindings {
    #[serde(default = "default_key")]
    pub play_pause: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn code(s: &str) -> KeyCode {
        parse_key_str(s).code
    }

    #[test]
    fn single_characters_parse() {
        assert_eq!(parse_key_str("j").code, KeyCode::Char('j'));
        assert_eq!(parse_key_str("  n  ").code, KeyCode::Char('n'), "trimmed");
        assert_eq!(parse_key_str("").code, KeyCode::Char(' '), "empty = space");
    }

    #[test]
    fn caret_notation_is_control() {
        assert_eq!(parse_key_str("^R").modifiers, KeyModifiers::CONTROL);
        assert_eq!(parse_key_str("^R").code, KeyCode::Char('R'));
    }

    #[test]
    fn special_names_parse() {
        assert_eq!(code("space"), KeyCode::Char(' '));
        assert_eq!(code("enter"), KeyCode::Enter);
        assert_eq!(code("esc"), KeyCode::Esc);
        assert_eq!(code("escape"), KeyCode::Esc);
        assert_eq!(code("backspace"), KeyCode::Backspace);
        assert_eq!(code("tab"), KeyCode::Tab);
        assert_eq!(code("up"), KeyCode::Up);
        assert_eq!(code("down"), KeyCode::Down);
        assert_eq!(code("left"), KeyCode::Left);
        assert_eq!(code("right"), KeyCode::Right);
        assert_eq!(code("UP"), KeyCode::Up, "names are case-insensitive");
    }

    /// A non-ASCII key is one character but several bytes. The byte-length
    /// check missed it, so it fell through to the fallback and turned into
    /// Space — silently binding the action to play/pause.
    #[test]
    fn multi_byte_characters_are_one_key() {
        assert_eq!(
            parse_key_str("↑").code,
            KeyCode::Char('↑'),
            "must not silently become Space"
        );
        assert_eq!(parse_key_str("é").code, KeyCode::Char('é'));
    }

    /// An unrecognised name is a configuration mistake. It still degrades to
    /// Space (the signature cannot fail), but it is no longer silent.
    #[test]
    fn unknown_names_fall_back_to_space() {
        assert_eq!(code("spacebar"), KeyCode::Char(' '));
        assert_eq!(code("f13"), KeyCode::Char(' '));
    }
}
