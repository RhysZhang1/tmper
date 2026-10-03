use std::path::PathBuf;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

use crate::ui::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserPanel {
    Library,
    Filesystem,
}

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
    pub scan_status: Option<String>,
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
            scan_status: None,
        }
    }
}

pub fn render_file_browser(f: &mut Frame, area: Rect, theme: &Theme, state: &FileBrowserState) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    render_library_panel(f, cols[0], theme, state);
    render_filesystem_panel(f, cols[1], theme, state);
}

fn render_library_panel(f: &mut Frame, area: Rect, theme: &Theme, state: &FileBrowserState) {
    let vis_h = area.height.saturating_sub(2) as usize;
    let start = state.scroll_library;

    let is_focused = state.focused == BrowserPanel::Library;
    let border_style = if is_focused {
        Style::default().fg(theme.primary)
    } else {
        Style::default().fg(theme.muted)
    };

    let end = (start + vis_h).min(state.library_paths.len());
    let items: Vec<ListItem> = (start..end)
        .map(|i| {
            let p = &state.library_paths[i];
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("?");
            let prefix = if p.is_dir() { "📁 " } else { "🎵 " };
            let style = if i == state.selected_library_index && is_focused {
                Style::default()
                    .fg(theme.text)
                    .bg(theme.muted)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.secondary)
            };
            ListItem::new(Line::from(Span::styled(format!("{prefix}{name}"), style)))
        })
        .collect();

    let title = state
        .scan_status
        .as_deref()
        .map(|status| format!(" Library — {status} "))
        .unwrap_or_else(|| " Library — a: add/rescan directory ".to_string());
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(border_style),
    );
    f.render_widget(list, area);
}

fn render_filesystem_panel(f: &mut Frame, area: Rect, theme: &Theme, state: &FileBrowserState) {
    let is_focused = state.focused == BrowserPanel::Filesystem;
    let border_style = if is_focused {
        Style::default().fg(theme.primary)
    } else {
        Style::default().fg(theme.muted)
    };

    let dir_str = state.current_dir.to_string_lossy().to_string();
    let vis_h = area.height.saturating_sub(2) as usize;
    let mut items: Vec<ListItem> = Vec::new();

    let total_items = state.fs_items.len();
    let scroll = state.scroll_fs.min(total_items.saturating_sub(1));
    let end = (scroll + vis_h).min(total_items);

    for (i, item) in state
        .fs_items
        .iter()
        .enumerate()
        .skip(scroll)
        .take(end - scroll)
    {
        let style = if is_focused && state.selected_fs_index == i {
            Style::default()
                .fg(theme.text)
                .bg(theme.muted)
                .add_modifier(Modifier::BOLD)
        } else {
            match item {
                FsItem::Dir(_) => Style::default().fg(theme.primary),
                FsItem::Audio(_) => Style::default().fg(theme.success),
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Render the view into a 80×24 buffer and return all cell symbols.
    fn render(state: &FileBrowserState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| render_file_browser(f, f.area(), &Theme::default(), state))
            .unwrap();
        let buf = terminal.backend().buffer();
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn test_render_empty_has_panel_titles() {
        let out = render(&FileBrowserState::default());
        assert!(out.contains(" Library "), "missing Library title");
    }

    #[test]
    fn test_render_populated_panels() {
        let mut state = FileBrowserState {
            current_dir: PathBuf::from("/music"),
            home_dir: PathBuf::from("/"),
            scroll_library: 0,
            scroll_fs: 0,
            dirs: vec![PathBuf::from("/music/sub")],
            audio_files: vec![PathBuf::from("/music/track.mp3")],
            selected_fs_index: 0,
            library_paths: vec![PathBuf::from("/music/alpha.flac")],
            selected_library_index: 0,
            focused: BrowserPanel::Library,
            fs_items: vec![FsItem::Dir("sub".into()), FsItem::Audio("track".into())],
            scan_status: None,
        };
        let out = render(&mut state);
        // Library panel shows the file stem.
        assert!(out.contains("alpha"), "library row rendered");
        // Filesystem panel shows icons and current dir in the title.
        assert!(out.contains("📁"), "dir icon rendered");
        assert!(out.contains("🎵"), "audio icon rendered");
        assert!(out.contains("/music"), "current dir shown in title");
    }

    #[test]
    fn test_render_fs_panel_focused_style_no_panic() {
        let mut state = FileBrowserState::default();
        state.current_dir = PathBuf::from("/tmp");
        state.dirs = vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")];
        state.fs_items = vec![FsItem::Dir("a".into()), FsItem::Dir("b".into())];
        state.focused = BrowserPanel::Filesystem;
        state.selected_fs_index = 1;
        let out = render(&state);
        assert!(out.contains("a") && out.contains("b"));
    }
}
