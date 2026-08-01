//! Theme system — named color palettes loaded from `themes/<name>.toml`.
//!
//! The palette uses semantic slots so views don't hardcode `Color` values.
//! Each slot maps to one of the colors previously hardcoded across the views.
//! A built-in Tokyo Night palette (`Theme::default`) is the fallback whenever
//! the requested theme file is missing or malformed.

use ratatui::style::Color;
use serde::Deserialize;

use crate::paths;

/// Semantic color palette for the whole UI.
///
/// Field names describe where the color is used, not the terminal's naming,
/// so switching themes changes every view consistently.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub name: String,
    /// Was `Color::Cyan` — borders, panel titles, current lyric, selection.
    pub primary: Color,
    /// Was `Color::White` — bold titles.
    pub text: Color,
    /// Was `Color::Gray` — artist, secondary text.
    pub secondary: Color,
    /// Was `Color::DarkGray` — borders, labels, hints.
    pub muted: Color,
    /// Was `Color::Green` — playing indicator, volume, notifications, confirm.
    pub success: Color,
    /// Was `Color::Yellow` — command mode, settings border.
    pub warning: Color,
    /// Was `Color::Magenta` — control bar.
    pub control: Color,
    /// Was `Rgb(255,200,100)` — active playlist name.
    pub accent: Color,
    /// Was `Rgb(180,180,200)` — album.
    pub album: Color,
    /// Was `Rgb(160,200,160)` — genre.
    pub genre: Color,
    /// Was `Rgb(200,180,140)` — year.
    pub year: Color,
    /// Was `Rgb(140,140,180)` — codec.
    pub codec: Color,
    /// Terminal background — used where a themed background is needed.
    pub bg: Color,
}

/// Raw on-disk representation of `themes/<name>.toml` (hex strings).
#[derive(Debug, Deserialize)]
struct ThemeFile {
    name: String,
    primary: String,
    text: String,
    secondary: String,
    muted: String,
    success: String,
    warning: String,
    control: String,
    accent: String,
    album: String,
    genre: String,
    year: String,
    codec: String,
    bg: String,
}

impl Theme {
    /// Built-in Tokyo Night palette — the fallback when a theme can't load.
    pub fn default() -> Self {
        Self {
            name: "tokyo-night".into(),
            primary: Color::Rgb(0x7a, 0xa2, 0xf7),
            text: Color::Rgb(0xc0, 0xca, 0xf5),
            secondary: Color::Rgb(0xa9, 0xb1, 0xd6),
            muted: Color::Rgb(0x56, 0x5f, 0x89),
            success: Color::Rgb(0x9e, 0xce, 0x6a),
            warning: Color::Rgb(0xe0, 0xaf, 0x68),
            control: Color::Rgb(0xbb, 0x9a, 0xf7),
            accent: Color::Rgb(0xff, 0x9e, 0x64),
            album: Color::Rgb(0x7d, 0xcf, 0xff),
            genre: Color::Rgb(0x9e, 0xce, 0x6a),
            year: Color::Rgb(0xe0, 0xaf, 0x68),
            codec: Color::Rgb(0x56, 0x5f, 0x89),
            bg: Color::Rgb(0x1a, 0x1b, 0x26),
        }
    }

    /// Load `themes/<name>.toml` from the project tree. Falls back to the
    /// built-in default on any I/O or parse failure (never panics).
    pub fn load(name: &str) -> Self {
        let path = paths::project_root()
            .join("themes")
            .join(format!("{name}.toml"));
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Theme '{name}' unreadable ({e}); using default");
                return Self::default();
            }
        };
        let file: ThemeFile = match toml::from_str(&content) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("Theme '{name}' malformed ({e}); using default");
                return Self::default();
            }
        };
        Self {
            name: file.name,
            primary: parse_hex(&file.primary),
            text: parse_hex(&file.text),
            secondary: parse_hex(&file.secondary),
            muted: parse_hex(&file.muted),
            success: parse_hex(&file.success),
            warning: parse_hex(&file.warning),
            control: parse_hex(&file.control),
            accent: parse_hex(&file.accent),
            album: parse_hex(&file.album),
            genre: parse_hex(&file.genre),
            year: parse_hex(&file.year),
            codec: parse_hex(&file.codec),
            bg: parse_hex(&file.bg),
        }
    }
}

/// Parse `#rrggbb` into `Color::Rgb`. Invalid input degrades to black.
fn parse_hex(s: &str) -> Color {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        tracing::warn!("Invalid theme color '{s}'");
        return Color::Black;
    }
    let (Ok(r), Ok(g), Ok(b)) = (
        u8::from_str_radix(&s[0..2], 16),
        u8::from_str_radix(&s[2..4], 16),
        u8::from_str_radix(&s[4..6], 16),
    ) else {
        tracing::warn!("Invalid theme color '#{s}'");
        return Color::Black;
    };
    Color::Rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_valid() {
        assert_eq!(parse_hex("#7aa2f7"), Color::Rgb(0x7a, 0xa2, 0xf7));
        assert_eq!(parse_hex("ffffff"), Color::Rgb(255, 255, 255));
    }

    #[test]
    fn parse_hex_invalid_degrades() {
        assert_eq!(parse_hex("#zzzzzz"), Color::Black);
        assert_eq!(parse_hex("#123"), Color::Black);
        assert_eq!(parse_hex(""), Color::Black);
    }

    #[test]
    fn load_missing_theme_falls_back_to_default() {
        assert_eq!(Theme::load("no-such-theme"), Theme::default());
    }

    #[test]
    fn load_real_theme_has_slots() {
        let t = Theme::load("tokyo-night");
        assert_eq!(t.name, "tokyo-night");
        assert_eq!(t.primary, Color::Rgb(0x7a, 0xa2, 0xf7));
    }
}
