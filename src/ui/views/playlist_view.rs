use std::path::PathBuf;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone)]
pub struct PlaylistData {
    pub name: String,
    pub songs: Vec<PathBuf>,
}

// ═══════════════════════════════════════════════════════════════════════
// PlaylistFlatModel — single source of truth for flat-line playlist layout
// ═══════════════════════════════════════════════════════════════════════

/// What a cursor line in the flat playlist view points to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineTarget {
    /// The "..." create-new-playlist row (line 0).
    AddNew,
    /// A playlist name row.
    PlaylistName(usize),
    /// A song row within an expanded playlist.
    Song { playlist: usize, song_index: usize },
    /// The "(empty)" placeholder row.
    Empty(usize),
}

/// Immutable snapshot for building flat-line lists. Holds references to the
/// underlying playlist data so renderers and handlers share one line-numbering scheme.
pub struct PlaylistFlatModel<'a> {
    pub playlists: &'a [PlaylistData],
    pub expanded: Option<usize>,
}

impl<'a> PlaylistFlatModel<'a> {
    pub fn new(playlists: &'a [PlaylistData], expanded: Option<usize>) -> Self {
        Self {
            playlists,
            expanded,
        }
    }

    /// Total number of visible lines (including the "..." row).
    pub fn total_lines(&self) -> usize {
        let mut count = 1usize; // "..."
        for (i, pl) in self.playlists.iter().enumerate() {
            count += 1; // playlist name
            if Some(i) == self.expanded {
                count += pl.songs.len().max(1); // songs or "(empty)"
            }
        }
        count
    }

    /// Return the flat-line index of a playlist's name row.
    pub fn line_of_playlist(&self, playlist_idx: usize) -> Option<usize> {
        let mut line = 1usize;
        for i in 0..playlist_idx {
            if i >= self.playlists.len() {
                return None;
            }
            line += 1;
            if Some(i) == self.expanded {
                line += self.playlists[i].songs.len().max(1);
            }
        }
        if playlist_idx < self.playlists.len() {
            Some(line)
        } else {
            None
        }
    }

    /// Resolve a flat-line cursor position to what it points to.
    pub fn resolve(&self, cursor: usize) -> LineTarget {
        if cursor == 0 {
            return LineTarget::AddNew;
        }
        let mut line = 1usize;
        for (i, pl) in self.playlists.iter().enumerate() {
            if cursor == line {
                return LineTarget::PlaylistName(i);
            }
            line += 1;
            if Some(i) == self.expanded {
                let song_count = pl.songs.len().max(1);
                if cursor >= line && cursor < line + song_count {
                    if pl.songs.is_empty() {
                        return LineTarget::Empty(i);
                    }
                    return LineTarget::Song {
                        playlist: i,
                        song_index: cursor - line,
                    };
                }
                line += song_count;
            }
        }
        LineTarget::AddNew // fallback for out-of-bounds
    }

    /// Build all flat lines for rendering, with styles.
    pub fn build_styled_lines(
        &self,
        is_focused: bool,
        insert_mode: &InsertMode,
        cursor: usize,
        playing_song_check: impl Fn(&PathBuf) -> bool,
    ) -> Vec<(String, ratatui::style::Style)> {
        let mut lines: Vec<(String, Style)> = Vec::with_capacity(self.total_lines());

        // Row 0: "..."
        let is_cursor = is_focused && cursor == 0;
        let (text, style) = if let InsertMode::Typing(ref s) = insert_mode {
            (
                format!("... {}", s),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (
                "...".to_string(),
                if is_cursor {
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Yellow)
                },
            )
        };
        lines.push((text, style));

        let mut line_idx = 1usize;
        for (i, pl) in self.playlists.iter().enumerate() {
            let is_expanded = self.expanded == Some(i);
            let is_cursor = is_focused && cursor == line_idx;
            let icon = if is_expanded { "▼" } else { "▶" };
            lines.push((
                format!("{} {}", icon, pl.name),
                if is_cursor {
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                },
            ));
            line_idx += 1;

            if is_expanded {
                if pl.songs.is_empty() {
                    let is_cursor = is_focused && cursor == line_idx;
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
                        let is_cursor = is_focused && cursor == line_idx;
                        let name = song.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
                        let is_playing = playing_song_check(song);
                        let prefix = if is_playing { "◄ " } else { "   " };
                        lines.push((
                            format!("{}{}", prefix, name),
                            if is_cursor {
                                Style::default().fg(Color::White).bg(Color::Cyan)
                            } else if is_playing {
                                Style::default()
                                    .fg(Color::Green)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::DarkGray)
                            },
                        ));
                        line_idx += 1;
                    }
                }
            }
        }
        lines
    }
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
    /// Key 5 (full playlist manager) — cursor in flat-model coordinates (0 = "…").
    pub selected_playlist: usize,
    pub selected_library_song: usize,
    pub focused: PlaylistPanel,
    pub insert_mode: InsertMode,
    pub notification: Option<(String, std::time::Instant)>,
    pub expanded_playlist: Option<usize>,
    pub scroll_library: usize,
    pub scroll_playlists: usize,
    /// Key 1 (player sidebar) — independent cursor, no "…" row, 0-based.
    pub sidebar_selected: usize,
    pub sidebar_scroll: usize,
}

impl Default for PlaylistManagerState {
    fn default() -> Self {
        Self {
            playlists: Vec::new(),
            library_paths: Vec::new(),
            selected_playlist: 0,
            selected_library_song: 0,
            focused: PlaylistPanel::Library,
            insert_mode: InsertMode::Off,
            notification: None,
            expanded_playlist: None,
            scroll_library: 0,
            scroll_playlists: 0,
            sidebar_selected: 0,
            sidebar_scroll: 0,
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
        let popup_w = UnicodeWidthStr::width(msg.as_str()) as u16 + 4;
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

    let model = PlaylistFlatModel::new(&state.playlists, state.expanded_playlist);
    let lines = model.build_styled_lines(
        is_focused,
        &state.insert_mode,
        state.selected_playlist,
        |_| false,
    );

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
