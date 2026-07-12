use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum LibraryPanel {
    Artists,
    Albums,
    Tracks,
}

#[allow(dead_code)]
pub struct LibraryState {
    pub artists: Vec<String>,
    pub albums: Vec<String>,
    pub tracks: Vec<String>,
    pub artist_index: usize,
    pub album_index: usize,
    pub track_index: usize,
    pub focused: LibraryPanel,
}

impl Default for LibraryState {
    fn default() -> Self {
        Self {
            artists: Vec::new(),
            albums: Vec::new(),
            tracks: Vec::new(),
            artist_index: 0,
            album_index: 0,
            track_index: 0,
            focused: LibraryPanel::Artists,
        }
    }
}

#[allow(dead_code)]
pub fn render_library_view(f: &mut Frame, area: Rect, state: &LibraryState) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(25),
            Constraint::Percentage(55),
        ])
        .split(area);

    render_panel(
        f,
        columns[0],
        "Artists",
        &state.artists,
        state.artist_index,
        state.focused == LibraryPanel::Artists,
    );
    render_panel(
        f,
        columns[1],
        "Albums",
        &state.albums,
        state.album_index,
        state.focused == LibraryPanel::Albums,
    );
    render_panel(
        f,
        columns[2],
        "Tracks",
        &state.tracks,
        state.track_index,
        state.focused == LibraryPanel::Tracks,
    );
}

#[allow(dead_code)]
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
