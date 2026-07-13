pub mod views;
pub mod widgets;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph};
use ratatui::Frame;
use serde::{Deserialize, Serialize};

use crate::lyrics::types::LyricTrack;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatMode {
    Sequential,
    Shuffle,
    SingleTrack,
}

impl RepeatMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Sequential => "⟳ 顺序循环",
            Self::Shuffle => "🔀 随机播放",
            Self::SingleTrack => "🔂 单曲循环",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Player,
    Library,
    Lyrics,
    Visualizer,
    Playlists,
    Browser,
    Settings,
}

#[derive(Debug, Clone)]
pub struct TrackDisplay {
    pub path: std::path::PathBuf,
    pub title: String,
    pub artist: String,
    pub duration_secs: f64,
}

pub struct UiState {
    pub title: String,
    pub artist: String,
    pub position: f64,
    pub duration: f64,
    pub volume: f32,
    pub is_playing: bool,
    pub album: String,
    pub genre: String,
    pub year: String,
    pub codec: String,
    pub repeat_mode: RepeatMode,
    pub notification: Option<(String, std::time::Instant)>,
    pub search_query: String,
    pub lyric_track: Option<LyricTrack>,
    pub current_lyric_index: usize,
    pub lyrics_offset_ms: i64,
    pub visualizer_data: Vec<f32>,
    pub show_help: bool,
    pub active_view: ViewMode,
    pub playlist_state: crate::ui::views::playlist_view::PlaylistManagerState,
    pub file_browser_state: crate::ui::views::file_browser_view::FileBrowserState,
    pub library_state: crate::ui::views::library_view::LibraryState,
    pub settings_state: crate::ui::views::settings_view::SettingsState,
    pub playlist_name: String,
    pub active_playlist: Option<usize>,
    pub active_playlist_song: Option<usize>,
    pub tracks: Vec<TrackDisplay>,
    pub cover_art: Option<Vec<u8>>,
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
            album: String::new(),
            genre: String::new(),
            year: String::new(),
            codec: String::new(),
            repeat_mode: RepeatMode::Sequential,
            notification: None,
            search_query: String::new(),
            lyric_track: None,
            current_lyric_index: 0,
            lyrics_offset_ms: 0,
            visualizer_data: Vec::new(),
            show_help: false,
            active_view: ViewMode::Player,
            playlist_state: crate::ui::views::playlist_view::PlaylistManagerState::default(),
            file_browser_state: crate::ui::views::file_browser_view::FileBrowserState::default(),
            library_state: crate::ui::views::library_view::LibraryState::default(),
            settings_state: crate::ui::views::settings_view::SettingsState::default(),
            playlist_name: "Default".into(),
            active_playlist: None,
            active_playlist_song: None,
            tracks: Vec::new(),
            cover_art: None,
            selected_index: 0,
            playing_index: None,
            scroll_offset: 0,
        }
    }
}

pub fn render(f: &mut Frame, state: &UiState) {
    // Help overlay — highest priority, always on top
    if state.show_help {
        crate::ui::widgets::help_popup::render_help(f);
        return;
    }

    // Floating notification for mode changes (shown before everything else)
    let should_notify = if let Some((_, t)) = &state.notification {
        t.elapsed().as_secs_f64() < 0.5
    } else {
        false
    };
    if should_notify {
        if let Some((ref msg, _)) = &state.notification {
            let area = f.area();
            let popup_w = (msg.len() as u16 + 4).min(area.width - 4);
            let popup_h = 3u16;
            let x = (area.width.saturating_sub(popup_w)) / 2;
            let y = (area.height.saturating_sub(popup_h)) / 2;
            let popup = Rect::new(x, y, popup_w, popup_h);
            f.render_widget(Clear, popup);
            let para = Paragraph::new(msg.as_str())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .style(Style::default().fg(Color::Green)),
                )
                .alignment(ratatui::layout::Alignment::Center);
            f.render_widget(para, popup);
        }
    }

    if state.active_view == ViewMode::Player {
        crate::ui::views::player_view::render_player_view(f, f.area(), state);
        return;
    }
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
    if state.active_view == ViewMode::Settings {
        crate::ui::views::settings_view::render_settings_view(
            f,
            f.area(),
            &state.settings_state,
        );
        return;
    }
    if state.active_view == ViewMode::Library {
        crate::ui::views::library_view::render_library_view(f, f.area(), &state.library_state);
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
    let mode_label = state.repeat_mode.label();
    let label = format!("Vol: {}%  {}", vol_pct, mode_label);

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

    let mode = state.repeat_mode.label();

    let view_label = match state.active_view {
        ViewMode::Player => "Player",
        ViewMode::Lyrics => "Lyrics",
        ViewMode::Library => "Library",
        ViewMode::Visualizer => "Visualizer",
        ViewMode::Playlists => "Playlists",
        ViewMode::Browser => "Browser",
        ViewMode::Settings => "Settings",
    };

    let playlist_label = if let Some(idx) = state.active_playlist {
        state.playlist_state.playlists.get(idx)
            .map(|p| p.name.as_str())
            .unwrap_or(&state.playlist_name)
    } else {
        &state.playlist_name
    };
    let status = format!(
        " {} | {} tracks | {} | {}  [{}]",
        playlist_label,
        state.tracks.len(),
        dur_str,
        mode,
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
