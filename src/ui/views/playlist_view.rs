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

#[allow(dead_code)]
pub struct PlaylistManagerState {
    pub playlists: Vec<PlaylistData>,
    pub library_paths: Vec<PathBuf>,
    pub cursor: usize,
    pub selected_playlist: usize,
    pub selected_song_in_playlist: usize,
    pub selected_library_song: usize,
    pub focused: PlaylistPanel,
    pub insert_mode: InsertMode,
    pub notification: Option<(String, std::time::Instant)>,
    pub expanded_playlist: Option<usize>,
    pub scroll_library: usize,
    pub scroll_playlists: usize,
}

impl Default for PlaylistManagerState {
    fn default() -> Self {
        Self {
            playlists: Vec::new(),
            library_paths: Vec::new(),
            cursor: 0,
            selected_playlist: 0,
            selected_song_in_playlist: 0,
            selected_library_song: 0,
            focused: PlaylistPanel::Library,
            insert_mode: InsertMode::Off,
            notification: None,
            expanded_playlist: None,
            scroll_library: 0,
            scroll_playlists: 0,
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

    // ISSUE 4: cursor always visible
    let vis_h = area.height.saturating_sub(2) as usize;
    let start = state.scroll_library;
    let end = (start + vis_h).min(state.library_paths.len());
    let items: Vec<ListItem> = state.library_paths[start..end]
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
            let style = if (start + i) == state.selected_library_song && is_focused {
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

    // Build flat line list — cursor index = state.selected_playlist for ALL rows
    let mut lines: Vec<(String, Style)> = Vec::new();
    let mut line_idx: usize = 0;

    // Row 0: "..." for creating playlist
    let is_cursor = is_focused && state.selected_playlist == line_idx;
    if matches!(state.insert_mode, InsertMode::Typing(_)) {
        let name = match &state.insert_mode {
            InsertMode::Typing(s) => format!("... {}", s),
            _ => String::new(),
        };
        lines.push((
            name,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        lines.push((
            "...".to_string(),
            if is_cursor {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Yellow)
            },
        ));
    }
    line_idx += 1;

    // Playlist rows
    for (i, pl) in state.playlists.iter().enumerate() {
        let is_expanded = state.expanded_playlist == Some(i);
        let is_cursor = is_focused && state.selected_playlist == line_idx;

        let icon = if is_expanded { "▼" } else { "▶" };
        let style = if is_cursor {
            Style::default()
                .fg(Color::White)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        lines.push((format!("{} {}", icon, pl.name), style));
        line_idx += 1;

        // Song rows (if expanded)
        if is_expanded {
            if pl.songs.is_empty() {
                let is_cursor = is_focused && state.selected_playlist == line_idx;
                lines.push((
                    "  (empty)".to_string(),
                    if is_cursor {
                        Style::default().fg(Color::White).bg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ));
                line_idx += 1;
            } else {
                for song in &pl.songs {
                    let is_cursor = is_focused && state.selected_playlist == line_idx;
                    let name = song.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
                    lines.push((
                        format!("    {}", name),
                        if is_cursor {
                            Style::default().fg(Color::White).bg(Color::Cyan)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        },
                    ));
                    line_idx += 1;
                }
            }
        }
    }

    // Apply scroll
    let vis_h = area.height.saturating_sub(2) as usize;
    let scroll = state.scroll_playlists.min(lines.len().saturating_sub(1));
    let end = (scroll + vis_h).min(lines.len());
    let visible: Vec<Line> = lines[scroll..end]
        .iter()
        .map(|(t, s)| Line::from(Span::styled(t.clone(), *s)))
        .collect();

    let para = Paragraph::new(visible).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Playlists ")
            .border_style(border_style),
    );
    f.render_widget(para, area);
}
