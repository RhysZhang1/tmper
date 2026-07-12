use std::path::PathBuf;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserPanel {
    Library,
    Filesystem,
}

#[allow(dead_code)]
pub struct FileBrowserState {
    pub current_dir: PathBuf,
    pub home_dir: PathBuf,
    pub scroll_library: usize,
    pub scroll_fs: usize,
    pub dirs: Vec<PathBuf>,
    pub audio_files: Vec<PathBuf>,
    pub selected_fs_index: usize,
    pub library_paths: Vec<PathBuf>,
    pub selected_library_index: usize,
    pub focused: BrowserPanel,
    pub fs_items: Vec<FsItem>,
}

#[derive(Debug, Clone)]
pub enum FsItem {
    Dir(String),
    Audio(String),
}

impl Default for FileBrowserState {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        Self {
            current_dir: home.clone(),
            home_dir: home,
            scroll_library: 0,
            scroll_fs: 0,
            dirs: Vec::new(),
            audio_files: Vec::new(),
            selected_fs_index: 0,
            library_paths: Vec::new(),
            selected_library_index: 0,
            focused: BrowserPanel::Filesystem,
            fs_items: Vec::new(),
        }
    }
}

pub fn render_file_browser(f: &mut Frame, area: Rect, state: &FileBrowserState) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    render_library_panel(f, cols[0], state);
    render_filesystem_panel(f, cols[1], state);
}

fn render_library_panel(f: &mut Frame, area: Rect, state: &FileBrowserState) {
    let vis_h = area.height.saturating_sub(2) as usize;
    let start = state.scroll_library;
    let _end = (start + vis_h).min(state.library_paths.len());
    let is_focused = state.focused == BrowserPanel::Library;
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
            let style = if (start + i) == state.selected_library_index && is_focused {
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

fn render_filesystem_panel(f: &mut Frame, area: Rect, state: &FileBrowserState) {
    let is_focused = state.focused == BrowserPanel::Filesystem;
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let dir_str = state.current_dir.to_string_lossy().to_string();
    let vis_h = area.height.saturating_sub(2) as usize;
    let mut items: Vec<ListItem> = Vec::new();

    let total_items = state.fs_items.len() + 1; // +1 for ".."
    let scroll = state.scroll_fs.min(total_items.saturating_sub(1));
    let end = (scroll + vis_h).min(total_items);

    // Add ".." for parent directory if visible
    if scroll == 0 {
        items.push(ListItem::new(Line::from(Span::styled(
            "📁 ..",
            Style::default().fg(Color::Yellow),
        ))));
    }

    // Dirs first, then audio files
    let start_item = scroll.saturating_sub(1);
    let count = end
        .saturating_sub(if scroll == 0 {
            1
        } else {
            scroll.saturating_sub(1)
        })
        .min(vis_h);
    for (i, item) in state
        .fs_items
        .iter()
        .enumerate()
        .skip(start_item)
        .take(count)
    {
        let actual_idx = i + 1; // +1 for ".."
        let style = if is_focused && state.selected_fs_index == actual_idx {
            Style::default()
                .fg(Color::White)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            match item {
                FsItem::Dir(_) => Style::default().fg(Color::Cyan),
                FsItem::Audio(_) => Style::default().fg(Color::Green),
            }
        };
        let prefix = match item {
            FsItem::Dir(_) => "📁 ",
            FsItem::Audio(_) => "🎵 ",
        };
        let name = match item {
            FsItem::Dir(n) | FsItem::Audio(n) => n.as_str(),
        };
        items.push(ListItem::new(Line::from(Span::styled(
            format!("{}{}", prefix, name),
            style,
        ))));
    }

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ", dir_str))
            .border_style(border_style),
    );
    f.render_widget(list, area);
}
