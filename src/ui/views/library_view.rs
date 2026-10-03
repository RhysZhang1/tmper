use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::ui::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

pub fn render_library_view(f: &mut Frame, area: Rect, theme: &Theme, state: &LibraryState) {
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
        theme,
        PanelParams {
            title: "Artists",
            items: &state.artists,
            selected: state.artist_index,
            scroll: state.scroll_artists,
            is_focused: state.focused == LibraryPanel::Artists && !has_search,
        },
    );
    render_panel(
        f,
        columns[1],
        theme,
        PanelParams {
            title: "Albums",
            items: &state.albums,
            selected: state.album_index,
            scroll: state.scroll_albums,
            is_focused: state.focused == LibraryPanel::Albums && !has_search,
        },
    );
    render_panel(
        f,
        columns[2],
        theme,
        PanelParams {
            title: "Tracks",
            items: &state.track_titles,
            selected: state.track_index,
            scroll: state.scroll_tracks,
            is_focused: state.focused == LibraryPanel::Tracks,
        },
    );

    // Search bar
    if let Some(sa) = search_area {
        let cursor = if state.search_query.len().is_multiple_of(2) {
            "|"
        } else {
            ""
        };
        let label = format!(
            " Search: {}{} (Enter to search, Backspace to exit) ",
            state.search_query, cursor
        );
        let para = Paragraph::new(Line::from(Span::styled(
            label,
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        )))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.warning)),
        );
        f.render_widget(para, sa);
    }
}

/// Read-only inputs for one column panel, gathered into a struct so the
/// renderer keeps taking explicit read-only parameters (the pattern used by
/// `PlayerViewParams`) instead of threading eight positional arguments.
struct PanelParams<'a> {
    title: &'a str,
    items: &'a [String],
    selected: usize,
    scroll: usize,
    is_focused: bool,
}

fn render_panel(f: &mut Frame, area: Rect, theme: &Theme, params: PanelParams<'_>) {
    let PanelParams {
        title,
        items,
        selected,
        scroll,
        is_focused,
    } = params;

    let border_style = if is_focused {
        Style::default().fg(theme.primary)
    } else {
        Style::default().fg(theme.muted)
    };

    // Slice to the visible window, as the file browser and playlist panels do.
    // Rendering every item and letting `List` clip them meant the cursor could
    // walk off the bottom of the panel and disappear, while the handler kept
    // dutifully maintaining `scroll_*` that nothing read.
    //
    // The maintained offset is honoured when it already shows the selection,
    // and corrected when it does not: the panel's real height shrinks while the
    // search bar is up, so an offset computed from the terminal height can be
    // a few rows stale. Keeping the selection on screen is the invariant that
    // matters, and it is cheap to guarantee here.
    let visible = area.height.saturating_sub(2) as usize;
    let total = items.len();
    let scroll = if visible == 0 || total <= visible {
        0
    } else if selected < scroll {
        selected
    } else if selected >= scroll + visible {
        selected - visible + 1
    } else {
        scroll
    };
    let scroll = scroll.min(total.saturating_sub(1));
    let end = (scroll + visible).min(total);

    let list_items: Vec<ListItem> = items[scroll..end]
        .iter()
        .enumerate()
        .map(|(offset, item)| {
            let index = scroll + offset;
            let style = if index == selected && is_focused {
                Style::default()
                    .fg(theme.text)
                    .bg(theme.muted)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.secondary)
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Render the view into a 80×24 buffer and return all cell symbols.
    fn render(state: &LibraryState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| render_library_view(f, f.area(), &Theme::default(), state))
            .unwrap();
        let buf = terminal.backend().buffer();
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn test_render_empty_has_panel_titles() {
        let out = render(&LibraryState::default());
        for title in ["Artists", "Albums", "Tracks"] {
            assert!(out.contains(title), "missing panel title {title}");
        }
    }

    #[test]
    fn test_render_populated_panels() {
        let s = LibraryState {
            artists: vec!["Artist A".into()],
            albums: vec!["Album 1".into()],
            track_paths: vec!["/a.flac".into()],
            track_titles: vec!["Song X".into()],
            artist_index: 0,
            album_index: 0,
            track_index: 0,
            focused: LibraryPanel::Tracks,
            scroll_artists: 0,
            scroll_albums: 0,
            scroll_tracks: 0,
            db_loaded: true,
            search_mode: false,
            search_query: String::new(),
        };
        let out = render(&s);
        assert!(out.contains("Artist A"), "artist row rendered");
        assert!(out.contains("Album 1"), "album row rendered");
        assert!(out.contains("Song X"), "track row rendered");
    }

    #[test]
    fn test_render_search_bar() {
        let s = LibraryState {
            search_mode: true,
            search_query: "abc".into(),
            ..Default::default()
        };
        let out = render(&s);
        assert!(out.contains("abc"), "query shown in search bar");
        assert!(out.contains("Search"), "search prompt visible");
    }

    #[test]
    fn test_render_search_cursor_blink() {
        // Even-length query → block cursor `|`; odd → empty (blink alternation).
        let even = LibraryState {
            search_mode: true,
            search_query: "ab".into(),
            ..Default::default()
        };
        assert!(render(&even).contains("|"));

        let odd = LibraryState {
            search_mode: true,
            search_query: "abc".into(),
            ..Default::default()
        };
        let out = render(&odd);
        assert!(
            !out.contains("abc|"),
            "no trailing cursor for odd-length query"
        );
    }

    /// The row the cursor is on has to be inside the rendered window. Every
    /// item used to be handed to `List` and clipped by it, so the cursor simply
    /// walked off the bottom of the panel and vanished — while the handler kept
    /// maintaining `scroll_*` offsets that nothing read.
    #[test]
    fn test_selected_row_stays_visible_in_a_long_list() {
        let artists: Vec<String> = (0..60).map(|i| format!("Artist {i:02}")).collect();
        let mut s = LibraryState {
            artists,
            artist_index: 55,
            scroll_artists: 0, // as if the handler never advanced it
            ..Default::default()
        };

        assert!(
            render(&s).contains("Artist 55"),
            "selected row must be rendered even with a stale scroll offset"
        );

        // With the offset caught up, the window is sliced for real.
        s.scroll_artists = 40;
        let out = render(&s);
        assert!(out.contains("Artist 55"), "still visible after scrolling");
        assert!(!out.contains("Artist 00"), "off-window rows are not drawn");
    }

    /// Scrolling past the selection pulls it back into view.
    #[test]
    fn test_scroll_ahead_of_selection_pulls_it_back() {
        let s = LibraryState {
            artists: (0..60).map(|i| format!("Artist {i:02}")).collect(),
            artist_index: 2,
            scroll_artists: 50, // window has scrolled away from the cursor
            ..Default::default()
        };
        assert!(render(&s).contains("Artist 02"));
    }
}
