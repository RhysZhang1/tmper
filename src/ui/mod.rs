use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph};
use ratatui::Frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum RepeatMode {
    Off,
    Track,
    Playlist,
}

impl RepeatMode {
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Off => "🔁",
            Self::Track => "🔂",
            Self::Playlist => "🔁",
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TrackDisplay {
    pub path: std::path::PathBuf,
    pub title: String,
    pub artist: String,
    pub duration_secs: f64,
    pub is_playing: bool,
}

#[allow(dead_code)]
pub struct UiState {
    pub title: String,
    pub artist: String,
    pub position: f64,
    pub duration: f64,
    pub volume: f32,
    pub is_playing: bool,
    pub repeat_mode: RepeatMode,
    pub shuffle: bool,
    pub search_query: String,
    pub playlist_name: String,
    pub tracks: Vec<TrackDisplay>,
    pub selected_index: usize,
    pub playing_index: Option<usize>,
    pub scroll_offset: usize,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            title: "No track".into(),
            artist: "—".into(),
            position: 0.0,
            duration: 0.0,
            volume: 0.8,
            is_playing: false,
            repeat_mode: RepeatMode::Off,
            shuffle: false,
            search_query: String::new(),
            playlist_name: "Default".into(),
            tracks: Vec::new(),
            selected_index: 0,
            playing_index: None,
            scroll_offset: 0,
        }
    }
}

#[allow(dead_code)]
pub fn render(f: &mut Frame, state: &UiState) {
    let area = f.area();

    // Split: title bar + progress bar + playlist + status bar
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title bar
            Constraint::Length(1), // progress bar
            Constraint::Min(3),    // playlist area
            Constraint::Length(1), // status bar
        ])
        .split(area);

    render_title_bar(f, main_layout[0], state);
    render_progress_bar(f, main_layout[1], state);
    render_playlist(f, main_layout[2], state);
    render_status_bar(f, main_layout[3], state);
}

fn render_title_bar(f: &mut Frame, area: Rect, state: &UiState) {
    let play_icon = if state.is_playing { "▶" } else { "⏸" };
    let pos_str = format_duration(state.position);
    let dur_str = format_duration(state.duration);

    let line = Line::from(vec![
        Span::styled(play_icon, Style::default().fg(Color::Green)),
        Span::raw(" "),
        Span::styled(
            format!("{} — {}", state.title, state.artist),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{} / {}", pos_str, dur_str),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let para = Paragraph::new(line);
    f.render_widget(para, area);
}

fn render_progress_bar(f: &mut Frame, area: Rect, state: &UiState) {
    let progress = if state.duration > 0.0 {
        (state.position / state.duration).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let vol_pct = (state.volume * 100.0) as u32;
    let shuffle_icon = if state.shuffle { "🔀" } else { "" };
    let repeat_icon = state.repeat_mode.icon();
    let label = format!("Vol: {}%  {} {}", vol_pct, repeat_icon, shuffle_icon);

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::NONE))
        .gauge_style(Style::default().fg(Color::Magenta))
        .label(label)
        .ratio(progress);

    f.render_widget(gauge, area);
}

fn render_playlist(f: &mut Frame, area: Rect, state: &UiState) {
    // Filter tracks by search query
    let filtered: Vec<(usize, &TrackDisplay)> = state
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            if state.search_query.is_empty() {
                return true;
            }
            let q = state.search_query.to_lowercase();
            t.title.to_lowercase().contains(&q) || t.artist.to_lowercase().contains(&q)
        })
        .collect();

    let visible_height = area.height as usize;
    let total_tracks = filtered.len();

    // Calculate visible range
    let start = state.scroll_offset;
    let end = (start + visible_height).min(total_tracks);

    let items: Vec<ListItem> = (start..end)
        .map(|i| {
            let (orig_idx, track) = &filtered[i];
            let dur = format_duration(track.duration_secs);

            let is_current = state.playing_index == Some(*orig_idx);
            let is_selected = state.selected_index == *orig_idx;

            let prefix = if is_current { "▶ " } else { "  " };

            let line_text = format!(
                "{}{}. {} — {}    {}",
                prefix,
                *orig_idx + 1,
                track.title,
                track.artist,
                dur
            );

            let style = if is_current {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(Color::White).bg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Gray)
            };

            ListItem::new(line_text).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::NONE));

    f.render_widget(list, area);
}

fn render_status_bar(f: &mut Frame, area: Rect, state: &UiState) {
    let total_duration: f64 = state.tracks.iter().map(|t| t.duration_secs).sum();
    let dur_str = format_duration(total_duration);

    let shuffle = if state.shuffle { "🔀" } else { "" };
    let repeat = state.repeat_mode.icon();

    let status = format!(
        "Playlist: {} | {} tracks | {} | {} {}",
        state.playlist_name,
        state.tracks.len(),
        dur_str,
        repeat,
        shuffle,
    );

    let para = Paragraph::new(status).style(Style::default().fg(Color::DarkGray));
    f.render_widget(para, area);
}

pub fn format_duration(secs: f64) -> String {
    let total_secs = secs as u64;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{:02}:{:02}", mins, secs)
}
