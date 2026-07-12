pub mod views;
pub mod widgets;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::lyrics::types::LyricTrack;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ViewMode {
    Player,
    Library,
    Lyrics,
    Visualizer,
    Playlists,
    Browser,
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
    pub lyric_track: Option<LyricTrack>,
    pub current_lyric_index: usize,
    pub lyrics_offset_ms: i64,
    pub visualizer_data: Vec<f32>,
    pub show_help: bool,
    pub active_view: ViewMode,
    pub playlist_state: crate::ui::views::playlist_view::PlaylistManagerState,
    pub file_browser_state: crate::ui::views::file_browser_view::FileBrowserState,
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
            lyric_track: None,
            current_lyric_index: 0,
            lyrics_offset_ms: 0,
            visualizer_data: Vec::new(),
            show_help: false,
            active_view: ViewMode::Player,
            playlist_state: crate::ui::views::playlist_view::PlaylistManagerState::default(),
            file_browser_state: crate::ui::views::file_browser_view::FileBrowserState::default(),
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
    if state.active_view == ViewMode::Lyrics {
        if let Some(ref track) = state.lyric_track {
            crate::ui::views::lyrics_view::render_lyrics_view(
                f,
                f.area(),
                track,
                state.current_lyric_index,
                state.lyrics_offset_ms,
            );
            return;
        }
    }
    if state.active_view == ViewMode::Visualizer {
        crate::ui::widgets::visualizer_panel::render_visualizer(
            f,
            f.area(),
            &state.visualizer_data,
        );
        return;
    }
    if state.active_view == ViewMode::Playlists {
        crate::ui::views::playlist_view::render_playlist_view(f, f.area(), &state.playlist_state);
        return;
    }
    if state.active_view == ViewMode::Browser {
        crate::ui::views::file_browser_view::render_file_browser(
            f,
            f.area(),
            &state.file_browser_state,
        );
        return;
    }
    if state.active_view == ViewMode::Library {
        // Show a simple library placeholder for now
        let para = ratatui::widgets::Paragraph::new(
            "Library browser — coming soon\n\nUse config/config.toml to set music_dirs",
        )
        .block(
            ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .title(" Library "),
        )
        .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(para, f.area());
        return;
    }

    let area = f.area();

    let has_lyrics = state.lyric_track.is_some();

    // Layout: title + progress + [lyrics] + playlist + status
    let mut constraints = vec![
        Constraint::Length(1), // title bar
        Constraint::Length(1), // progress bar
    ];
    if has_lyrics {
        constraints.push(Constraint::Length(6)); // lyrics panel
    }
    constraints.push(Constraint::Min(3)); // playlist
    constraints.push(Constraint::Length(1)); // status bar

    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut idx = 0;
    render_title_bar(f, main_layout[idx], state);
    idx += 1;
    render_progress_bar(f, main_layout[idx], state);
    idx += 1;

    if has_lyrics {
        if let Some(ref track) = state.lyric_track {
            crate::ui::widgets::lyrics_panel::render_lyrics(
                f,
                main_layout[idx],
                track,
                state.current_lyric_index,
            );
        }
        idx += 1;
    }

    render_playlist(f, main_layout[idx], state);
    idx += 1;
    render_status_bar(f, main_layout[idx], state);

    // Help overlay always on top
    if state.show_help {
        crate::ui::widgets::help_popup::render_help(f);
    }
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

    let view_label = match state.active_view {
        ViewMode::Player => "Player",
        ViewMode::Lyrics => "Lyrics",
        ViewMode::Library => "Library",
        ViewMode::Visualizer => "Visualizer",
        ViewMode::Playlists => "Playlists",
        ViewMode::Browser => "Browser",
    };

    let status = format!(
        "Playlist: {} | {} tracks | {} | {} {}  [{}]",
        state.playlist_name,
        state.tracks.len(),
        dur_str,
        repeat,
        shuffle,
        view_label,
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
