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
/// Per-track display data stored in `PlayerCore.tracks`.
/// `title`, `artist`, `duration_secs` reserved for sidebar info display.
#[allow(dead_code)]
pub struct TrackDisplay {
    pub path: std::path::PathBuf,
    pub title: String,
    pub artist: String,
    pub duration_secs: f64,
}

/// Core player state — all fields related to the currently playing track.
#[derive(Debug, Clone)]
pub struct PlayerCore {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub year: String,
    pub codec: String,
    pub position: f64,
    pub duration: f64,
    pub is_playing: bool,
    pub tracks: Vec<TrackDisplay>,
    pub playing_index: Option<usize>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub cover_art: Option<Arc<Vec<u8>>>,
    pub show_cover_art: bool,
    pub cover_gen: Cell<u64>,
}

impl Default for PlayerCore {
    fn default() -> Self {
        Self {
            title: "No track".into(),
            artist: "—".into(),
            album: String::new(),
            genre: String::new(),
            year: String::new(),
            codec: String::new(),
            position: 0.0,
            duration: 0.0,
            is_playing: false,
            tracks: Vec::new(),
            playing_index: None,
            selected_index: 0,
            scroll_offset: 0,
            cover_art: None,
            show_cover_art: true,
            cover_gen: Cell::new(0),
        }
    }
}

impl PlayerCore {
    /// Reset transient playback state on track change.
    pub fn reset_on_track_change(&mut self) {
        self.position = 0.0;
        self.is_playing = false;
    }
}

/// Lyrics state — isolated from player core.
#[derive(Debug, Clone, Default)]
pub struct LyricsState {
    pub lyric_track: Option<LyricTrack>,
    pub current_lyric_index: usize,
    pub lyrics_offset_ms: i64,
}

/// View-related transient UI state.
#[derive(Debug, Clone)]
pub struct ViewState {
    pub active_view: ViewMode,
    pub show_help: bool,
    pub help_scroll: usize,
    pub last_help_toggle: Option<std::time::Instant>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            active_view: ViewMode::Player,
            show_help: false,
            help_scroll: 0,
            last_help_toggle: None,
        }
    }
}

impl ViewState {
    /// Dismiss overlays on track change.
    pub fn reset_on_track_change(&mut self) {
        self.show_help = false;
    }
}

pub struct UiState {
    pub player: PlayerCore,
    pub volume: f32,
    pub repeat_mode: RepeatMode,
    pub lyrics: LyricsState,
    pub visualizer_data: Vec<f32>,
    pub view: ViewState,
    pub playlist_state: crate::ui::views::playlist_view::PlaylistManagerState,
    pub file_browser_state: crate::ui::views::file_browser_view::FileBrowserState,
    pub library_state: crate::ui::views::library_view::LibraryState,
    pub settings_state: crate::ui::views::settings_view::SettingsState,
    /// Cross-view playlist playback context.
    pub active_playlist: Option<usize>,
    pub active_playlist_song: Option<usize>,
    pub playlist_name: String,
    pub command_mode: bool,
    pub command_buffer: String,
    pub search_query: String,
    pub notification: Option<(String, std::time::Instant)>,
    pub visible_rows: Cell<usize>,
    pub cover_rect: Cell<(u16, u16, u16, u16)>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            player: PlayerCore::default(),
            volume: 0.8,
            repeat_mode: RepeatMode::Sequential,
            lyrics: LyricsState::default(),
            visualizer_data: Vec::new(),
            view: ViewState::default(),
            playlist_state: crate::ui::views::playlist_view::PlaylistManagerState::default(),
            file_browser_state: crate::ui::views::file_browser_view::FileBrowserState::default(),
            library_state: crate::ui::views::library_view::LibraryState::default(),
            settings_state: crate::ui::views::settings_view::SettingsState::default(),
            active_playlist: None,
            active_playlist_song: None,
            playlist_name: "Default".into(),
            command_mode: false,
            command_buffer: String::new(),
            search_query: String::new(),
            notification: None,
            visible_rows: Cell::new(20),
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
    if state.view.show_help {
        crate::ui::widgets::help_popup::render_help(f, state.view.help_scroll);
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

    match state.view.active_view {
        ViewMode::Player => {
            let params = crate::ui::views::player_view::PlayerViewParams {
                title: &state.player.title,
                artist: &state.player.artist,
                position: state.player.position,
                duration: state.player.duration,
                volume: state.volume,
                is_playing: state.player.is_playing,
                album: &state.player.album,
                genre: &state.player.genre,
                year: &state.player.year,
                codec: &state.player.codec,
                repeat_mode: state.repeat_mode,
                cover_art: state.player.cover_art.as_ref(),
                show_cover_art: state.player.show_cover_art,
                cover_rect: &state.cover_rect,
                lyric_track: state.lyrics.lyric_track.as_ref(),
                current_lyric_index: state.lyrics.current_lyric_index,
                visualizer_data: &state.visualizer_data,
                playlist_state: &state.playlist_state,
                playing_index: state.player.playing_index,
                tracks: &state.player.tracks,
                active_playlist: state.active_playlist,
            };
            crate::ui::views::player_view::render_player_view(f, f.area(), &params);
        }
        ViewMode::Lyrics => {
            if let Some(ref track) = state.lyrics.lyric_track {
                crate::ui::views::lyrics_view::render_lyrics_view(
                    f,
                    f.area(),
                    track,
                    state.lyrics.current_lyric_index,
                    state.lyrics.lyrics_offset_ms,
                );
            }
        }
        ViewMode::Visualizer => {
            crate::ui::widgets::visualizer_panel::render_visualizer(
                f,
                f.area(),
                &state.visualizer_data,
            );
        }
        ViewMode::Playlists => {
            crate::ui::views::playlist_view::render_playlist_view(
                f,
                f.area(),
                &state.playlist_state,
            );
        }
        ViewMode::Browser => {
            crate::ui::views::file_browser_view::render_file_browser(
                f,
                f.area(),
                &state.file_browser_state,
            );
        }
        ViewMode::Settings => {
            crate::ui::views::settings_view::render_settings_view(
                f,
                f.area(),
                &state.settings_state,
            );
        }
        ViewMode::Library => {
            crate::ui::views::library_view::render_library_view(
                f,
                f.area(),
                &state.library_state,
            );
        }
    }
}

pub fn format_duration(secs: f64) -> String {
    let total_secs = secs as u64;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{:02}:{:02}", mins, secs)
}
