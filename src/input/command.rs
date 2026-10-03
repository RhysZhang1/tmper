use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    /// Leave the TUI. The player is a separate process and carries on.
    Quit,
    /// Leave the TUI *and* stop the player: `:quit!`, or `tmper quit`.
    QuitAll,
    Help,
    Version,
    Theme(String),
    Seek(String),
    Volume(u32),
    Repeat(String),
    Shuffle(String),
    View(String),
    Import(String),
    Export(String),
    Unknown(String),
}

pub fn parse_command(input: &str) -> Command {
    let input = input.trim().trim_start_matches(':');
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    let cmd = parts[0].to_lowercase();
    let arg = parts.get(1).map(|s| s.trim());

    match cmd.as_str() {
        "q" | "quit" | "wq" => Command::Quit,
        // The bang is the whole difference, and it is worth a whole variant:
        // one of these leaves the music playing and the other stops it.
        "q!" | "quit!" | "wq!" => Command::QuitAll,
        "help" => Command::Help,
        "version" => Command::Version,
        "theme" => Command::Theme(arg.unwrap_or("").to_string()),
        "seek" => Command::Seek(arg.unwrap_or("").to_string()),
        "volume" | "vol" => {
            if let Some(a) = arg {
                if let Ok(v) = a.parse::<u32>() {
                    return Command::Volume(v.min(100));
                }
            }
            Command::Unknown(input.to_string())
        }
        "repeat" => Command::Repeat(arg.unwrap_or("").to_string()),
        "shuffle" => Command::Shuffle(arg.unwrap_or("").to_string()),
        "view" => Command::View(arg.unwrap_or("").to_string()),
        "import" => Command::Import(arg.unwrap_or("").to_string()),
        "export" => Command::Export(arg.unwrap_or("").to_string()),
        _ => Command::Unknown(input.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_quit() {
        assert!(matches!(parse_command(":q"), Command::Quit));
        assert!(matches!(parse_command(":quit"), Command::Quit));
        assert!(matches!(parse_command(":wq"), Command::Quit));
    }

    /// The bang is not decoration: `:quit` leaves the music playing and
    /// `:quit!` stops it, so a parser that folded the two together would
    /// silently cut the sound on the wrong keypress.
    #[test]
    fn test_parse_quit_all() {
        for input in [":q!", ":quit!", ":wq!", ":QUIT!"] {
            assert!(
                matches!(parse_command(input), Command::QuitAll),
                "{input} should be the one that stops the player"
            );
        }
    }

    #[test]
    fn test_parse_theme() {
        match parse_command(":theme dracula") {
            Command::Theme(name) => assert_eq!(name, "dracula"),
            _ => panic!("Expected Theme"),
        }
    }

    #[test]
    fn test_parse_volume() {
        match parse_command(":volume 80") {
            Command::Volume(v) => assert_eq!(v, 80),
            _ => panic!("Expected Volume"),
        }
    }

    #[test]
    fn test_parse_unknown() {
        match parse_command(":nonexistent") {
            Command::Unknown(_) => {}
            _ => panic!("Expected Unknown"),
        }
    }
}
