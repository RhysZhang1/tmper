use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum LibraryPanel {
    Artists,
    Albums,
    Tracks,
}

pub struct LibraryState {
    pub artists: Vec<String>,
    pub albums: Vec<String>,
    pub track_paths: Vec<String>,
    pub track_titles: Vec<String>,
    pub artist_index: usize,
    pub album_index: usize,
    pub track_index: usize,
    pub focused: LibraryPanel,
    pub scroll_artists: usize,
    pub scroll_albums: usize,
    pub scroll_tracks: usize,
    pub db_loaded: bool,
    pub search_mode: bool,
    pub search_query: String,
}

impl Default for LibraryState {
    fn default() -> Self {
        Self {
            artists: Vec::new(),
            albums: Vec::new(),
            track_paths: Vec::new(),
            track_titles: Vec::new(),
            artist_index: 0,
            album_index: 0,
            track_index: 0,
            focused: LibraryPanel::Artists,
            scroll_artists: 0,
            scroll_albums: 0,
            scroll_tracks: 0,
            db_loaded: false,
            search_mode: false,
            search_query: String::new(),
        }
    }
}

pub fn render_library_view(f: &mut Frame, area: Rect, state: &LibraryState) {
    // Split area: main content + optional search bar
    let has_search = state.search_mode;
    let (main_area, search_area) = if has_search {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(3)])
            .split(area);
        (split[0], Some(split[1]))
    } else {
        (area, None)
    };

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(25),
            Constraint::Percentage(55),
        ])
        .split(main_area);

    render_panel(
        f,
        columns[0],
        "Artists",
        &state.artists,
        state.artist_index,
        state.focused == LibraryPanel::Artists && !has_search,
    );
    render_panel(
        f,
        columns[1],
        "Albums",
        &state.albums,
        state.album_index,
        state.focused == LibraryPanel::Albums && !has_search,
    );
    render_panel(
        f,
        columns[2],
        "Tracks",
        &state.track_titles,
        state.track_index,
        state.focused == LibraryPanel::Tracks,
    );

    // Search bar
    if let Some(sa) = search_area {
        let cursor = if state.search_query.len().is_multiple_of(2) { "|" } else { "" };
        let label = format!(" Search: {}{} (Enter to search, Backspace to exit) ", state.search_query, cursor);
        let para = Paragraph::new(Line::from(Span::styled(
            label,
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )))
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)));
        f.render_widget(para, sa);
    }
}

fn render_panel(
    f: &mut Frame,
    area: Rect,
    title: &str,
    items: &[String],
    selected: usize,
    is_focused: bool,
) {
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let style = if i == selected && is_focused {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            ListItem::new(Line::from(Span::styled(item.clone(), style)))
        })
        .collect();

    let list = List::new(list_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(border_style),
    );

    f.render_widget(list, area);
}
