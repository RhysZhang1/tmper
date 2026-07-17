pub mod cover;
pub mod views;
pub mod widgets;
use std::cell::Cell;
use std::sync::Arc;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use serde::{Deserialize, Serialize};

use crate::constants::runtime;
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
#[allow(dead_code)]
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
    pub cover_art: Option<Arc<Vec<u8>>>,
    pub show_cover_art: bool,
    pub selected_index: usize,
    pub playing_index: Option<usize>,
    pub scroll_offset: usize,
    pub command_mode: bool,
    pub command_buffer: String,
    pub help_scroll: usize,
    /// Actual visible rows computed from terminal size during render.
    /// Updated each frame; read by scroll handlers to avoid hardcoded limits.
    pub visible_rows: Cell<usize>,
    /// Monotonic counter bumped each time cover art changes.
    pub cover_gen: Cell<u64>,
    /// Inner rect of cover art area (x, y, w, h in chars) — set during render.
    pub cover_rect: Cell<(u16, u16, u16, u16)>,
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
            show_cover_art: true,
            selected_index: 0,
            playing_index: None,
            scroll_offset: 0,
            command_mode: false,
            command_buffer: String::new(),
            help_scroll: 0,
            visible_rows: Cell::new(20),
            cover_gen: Cell::new(0),
            cover_rect: Cell::new((0, 0, 0, 0)),
        }
    }
}

pub fn render(f: &mut Frame, state: &UiState) {
    // Update visible row count from actual terminal size
    state
        .visible_rows
        .set(f.area().height.saturating_sub(2) as usize);

    // Help overlay — highest priority, always on top
    if state.show_help {
        crate::ui::widgets::help_popup::render_help(f, state.help_scroll);
        return;
    }

    // Floating notification for mode changes (shown before everything else)
    let should_notify = if let Some((_, t)) = &state.notification {
        t.elapsed().as_secs_f64() < runtime::NOTIFICATION_DURATION_SECS
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

    // Command mode: centered popup overlay
    if state.command_mode {
        let area = f.area();
        let popup_w = 56u16.min(area.width - 4);
        let popup_h = 14u16.min(area.height - 4);
        let x = (area.width.saturating_sub(popup_w)) / 2;
        let y = (area.height.saturating_sub(popup_h)) / 2;
        let popup = Rect::new(x, y, popup_w, popup_h);
        f.render_widget(Clear, popup);

        let mut lines: Vec<Line> = Vec::new();
        let cursor = if state.command_buffer.len().is_multiple_of(2) {
            "▊"
        } else {
            ""
        };
        lines.push(Line::from(Span::styled(
            format!(":{} {}", state.command_buffer, cursor),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Commands:",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "  q quit  |  help  |  version  |  theme <name>",
            Style::default().fg(Color::Gray),
        )));
        lines.push(Line::from(Span::styled(
            "  seek <secs>  |  volume <0-100>  |  repeat <mode>",
            Style::default().fg(Color::Gray),
        )));
        lines.push(Line::from(Span::styled(
            "  view <name>  |  import <path>  |  export <name>",
            Style::default().fg(Color::Gray),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  view names: player library lyrics visualizer",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "               playlists browser settings",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "  repeat: sequential shuffle single",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Enter=执行  Esc=取消",
            Style::default().fg(Color::Green),
        )));

        let para = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Command ")
                .border_style(Style::default().fg(Color::Yellow)),
        );
        f.render_widget(para, popup);
        return;
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
        crate::ui::views::settings_view::render_settings_view(f, f.area(), &state.settings_state);
        return;
    }
    if state.active_view == ViewMode::Library {
        crate::ui::views::library_view::render_library_view(f, f.area(), &state.library_state);
    }
}

pub fn format_duration(secs: f64) -> String {
    let total_secs = secs as u64;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{:02}:{:02}", mins, secs)
}
