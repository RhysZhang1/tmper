use std::path::PathBuf;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

#[derive(Debug, Clone)]
pub struct PlaylistData {
    pub name: String,
    pub songs: Vec<PathBuf>,
    #[allow(dead_code)]
    pub expanded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaylistPanel {
    Library,
    Playlists,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertMode {
    Off,
    Typing(String),
}

pub struct PlaylistManagerState {
    pub playlists: Vec<PlaylistData>,
    pub library_paths: Vec<PathBuf>,
    pub selected_playlist: usize,
    pub selected_song_in_playlist: usize,
    pub selected_library_song: usize,
    pub focused: PlaylistPanel,
    pub insert_mode: InsertMode,
    pub notification: Option<(String, std::time::Instant)>,
    pub expanded_playlist: Option<usize>,
}

impl Default for PlaylistManagerState {
    fn default() -> Self {
        Self {
            playlists: Vec::new(),
            library_paths: Vec::new(),
            selected_playlist: 0,
            selected_song_in_playlist: 0,
            selected_library_song: 0,
            focused: PlaylistPanel::Library,
            insert_mode: InsertMode::Off,
            notification: None,
            expanded_playlist: None,
        }
    }
}

pub fn render_playlist_view(f: &mut Frame, area: Rect, state: &PlaylistManagerState) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    render_library_panel(f, cols[0], state);
    render_playlists_panel(f, cols[1], state);

    // Notification overlay
    if let Some((ref msg, _)) = state.notification {
        let popup_w = msg.len() as u16 + 4;
        let popup_h = 3u16;
        let x = (area.width.saturating_sub(popup_w)) / 2;
        let y = (area.height.saturating_sub(popup_h)) / 2;
        let popup = Rect::new(x, y, popup_w, popup_h);
        f.render_widget(Clear, popup);
        let para = Paragraph::new(msg.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::Yellow)),
        );
        f.render_widget(para, popup);
    }
}

fn render_library_panel(f: &mut Frame, area: Rect, state: &PlaylistManagerState) {
    let is_focused = state.focused == PlaylistPanel::Library;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let items: Vec<ListItem> = state
        .library_paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
            let style = if i == state.selected_library_song && is_focused {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            ListItem::new(Line::from(Span::styled(name, style)))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Library ")
            .border_style(border_style),
    );
    f.render_widget(list, area);
}

fn render_playlists_panel(f: &mut Frame, area: Rect, state: &PlaylistManagerState) {
    let is_focused = state.focused == PlaylistPanel::Playlists;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let mut lines: Vec<Line> = Vec::new();

    // First row: "..." for creating new playlist
    let create_style = if is_focused && state.selected_playlist == 0 {
        Style::default()
            .fg(Color::White)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Yellow)
    };

    if matches!(state.insert_mode, InsertMode::Typing(_)) {
        let name = match &state.insert_mode {
            InsertMode::Typing(s) => format!("... {}", s),
            _ => String::new(),
        };
        lines.push(Line::from(Span::styled(
            name,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
    } else {
        lines.push(Line::from(Span::styled("...", create_style)));
    }

    // Rest: playlists
    for (i, pl) in state.playlists.iter().enumerate() {
        let idx = i + 1; // offset for "..." row
        let is_selected = is_focused && state.selected_playlist == idx;
        let is_expanded = state.expanded_playlist == Some(i);

        let icon = if is_expanded { "▼" } else { "▶" };
        let line_style = if is_selected {
            Style::default().fg(Color::White).bg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Gray)
        };
        lines.push(Line::from(Span::styled(
            format!("{} {}", icon, pl.name),
            line_style,
        )));

        // If expanded, show songs or empty line
        if is_expanded {
            if pl.songs.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  (empty)",
                    Style::default().fg(Color::DarkGray),
                )));
            } else {
                for (si, song) in pl.songs.iter().enumerate() {
                    let name = song.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
                    let song_style = if is_focused && state.selected_song_in_playlist == si {
                        Style::default().fg(Color::White).bg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    lines.push(Line::from(Span::styled(
                        format!("    {}", name),
                        song_style,
                    )));
                }
            }
        }
    }

    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Playlists ")
            .border_style(border_style),
    );
    f.render_widget(para, area);
}
