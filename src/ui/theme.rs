use ratatui::style::Color;

#[derive(Debug, Clone)]
pub struct ThemeColors {
    pub bg: Color,
    pub fg: Color,
    pub accent: Color,
    pub highlight: Color,
    pub dimmed: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub bar_low: Color,
    pub bar_mid: Color,
    pub bar_high: Color,
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub colors: ThemeColors,
}

impl Theme {
    pub fn preset(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "tokyo-night" => Some(Self {
                name: "tokyo-night".into(),
                colors: ThemeColors {
                    bg: Color::from_u32(0x001a1b26),
                    fg: Color::from_u32(0x00c0caf5),
                    accent: Color::from_u32(0x007aa2f7),
                    highlight: Color::from_u32(0x00bb9af7),
                    dimmed: Color::from_u32(0x00565f89),
                    success: Color::from_u32(0x009ece6a),
                    warning: Color::from_u32(0x00e0af68),
                    error: Color::from_u32(0x00f7768e),
                    bar_low: Color::from_u32(0x009ece6a),
                    bar_mid: Color::from_u32(0x00e0af68),
                    bar_high: Color::from_u32(0x00f7768e),
                },
            }),
            "dracula" => Some(Self {
                name: "dracula".into(),
                colors: ThemeColors {
                    bg: Color::from_u32(0x00282a36),
                    fg: Color::from_u32(0x00f8f8f2),
                    accent: Color::from_u32(0x00bd93f9),
                    highlight: Color::from_u32(0x00ff79c6),
                    dimmed: Color::from_u32(0x006272a4),
                    success: Color::from_u32(0x0050fa7b),
                    warning: Color::from_u32(0x00f1fa8c),
                    error: Color::from_u32(0x00ff5555),
                    bar_low: Color::from_u32(0x0050fa7b),
                    bar_mid: Color::from_u32(0x00f1fa8c),
                    bar_high: Color::from_u32(0x00ff5555),
                },
            }),
            "nord" => Some(Self {
                name: "nord".into(),
                colors: ThemeColors {
                    bg: Color::from_u32(0x002e3440),
                    fg: Color::from_u32(0x00d8dee9),
                    accent: Color::from_u32(0x0081a1c1),
                    highlight: Color::from_u32(0x0088c0d0),
                    dimmed: Color::from_u32(0x004c566a),
                    success: Color::from_u32(0x00a3be8c),
                    warning: Color::from_u32(0x00ebcb8b),
                    error: Color::from_u32(0x00bf616a),
                    bar_low: Color::from_u32(0x00a3be8c),
                    bar_mid: Color::from_u32(0x00ebcb8b),
                    bar_high: Color::from_u32(0x00bf616a),
                },
            }),
            "solarized-dark" => Some(Self {
                name: "solarized-dark".into(),
                colors: ThemeColors {
                    bg: Color::from_u32(0x00002b36),
                    fg: Color::from_u32(0x00839a96),
                    accent: Color::from_u32(0x00268bd2),
                    highlight: Color::from_u32(0x006c71c4),
                    dimmed: Color::from_u32(0x00586e75),
                    success: Color::from_u32(0x00859900),
                    warning: Color::from_u32(0x00b58900),
                    error: Color::from_u32(0x00dc322f),
                    bar_low: Color::from_u32(0x00859900),
                    bar_mid: Color::from_u32(0x00b58900),
                    bar_high: Color::from_u32(0x00dc322f),
                },
            }),
            "catppuccin-mocha" => Some(Self {
                name: "catppuccin-mocha".into(),
                colors: ThemeColors {
                    bg: Color::from_u32(0x001e1e2e),
                    fg: Color::from_u32(0x00cdd6f4),
                    accent: Color::from_u32(0x0089b4fa),
                    highlight: Color::from_u32(0x00cba6f7),
                    dimmed: Color::from_u32(0x005850b0),
                    success: Color::from_u32(0x00a6e3a1),
                    warning: Color::from_u32(0x00f9e2af),
                    error: Color::from_u32(0x00f38ba8),
                    bar_low: Color::from_u32(0x00a6e3a1),
                    bar_mid: Color::from_u32(0x00f9e2af),
                    bar_high: Color::from_u32(0x00f38ba8),
                },
            }),
            _ => None,
        }
    }
}
