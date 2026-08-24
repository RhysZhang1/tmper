pub mod cover;
pub mod theme;
pub mod views;
pub mod widgets;
use std::cell::{Cell, RefCell};
use std::sync::Arc;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use serde::{Deserialize, Serialize};

use crate::constants::runtime;
use crate::lyrics::types::LyricTrack;
use crate::ui::theme::Theme;

const MIN_TERMINAL_WIDTH: u16 = 30;
const MIN_TERMINAL_HEIGHT: u16 = 8;

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

/// Cached result of the half-block cover render (image decode + Lanczos3
/// resize + Floyd-Steinberg dither), keyed by cover identity and render area.
/// Recomputing every frame at 30 FPS was a major UI-lag source on the player
/// view, even on terminals where a SIXEL/Kitty cover hides the blocks.
#[derive(Debug, Clone)]
pub struct CoverLinesCache {
    pub gen: u64,
    pub width: u16,
    pub height: u16,
    pub lines: Vec<ratatui::text::Line<'static>>,
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
    pub cover_lines_cache: RefCell<Option<CoverLinesCache>>,
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
            cover_lines_cache: RefCell::new(None),
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
    pub theme: Theme,
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
    /// Global `/` search mode (player-view track filter). Unlike the
    /// library's own search, this one lives on the player queue.
    pub search_mode: bool,
    pub search_query: String,
    pub notification: Option<(String, std::time::Instant)>,
    pub visible_rows: Cell<usize>,
    pub cover_rect: Cell<(u16, u16, u16, u16)>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
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
            search_mode: false,
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

    if terminal_is_too_small(f.area()) {
        state.cover_rect.set((0, 0, 0, 0));
        render_too_small(f, &state.theme);
        return;
    }

    // Help overlay — highest priority, always on top
    if state.view.show_help {
        crate::ui::widgets::help_popup::render_help(f, &state.theme, state.view.help_scroll);
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
            let popup_w = (msg.len() as u16 + 4).min(area.width.saturating_sub(4));
            let popup_h = 3u16;
            let x = (area.width.saturating_sub(popup_w)) / 2;
            let y = (area.height.saturating_sub(popup_h)) / 2;
            let popup = Rect::new(x, y, popup_w, popup_h);
            f.render_widget(Clear, popup);
            let para = Paragraph::new(msg.as_str())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .style(Style::default().fg(state.theme.success)),
                )
                .alignment(ratatui::layout::Alignment::Center);
            f.render_widget(para, popup);
        }
    }

    // Command mode: centered popup overlay
    if state.command_mode {
        let area = f.area();
        let popup_w = 56u16.min(area.width.saturating_sub(4));
        let popup_h = 14u16.min(area.height.saturating_sub(4));
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
                .fg(state.theme.warning)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Commands:",
            Style::default()
                .fg(state.theme.primary)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "  q quit  |  help  |  version  |  theme <name>",
            Style::default().fg(state.theme.secondary),
        )));
        lines.push(Line::from(Span::styled(
            "  seek <secs>  |  volume <0-100>  |  repeat <mode>",
            Style::default().fg(state.theme.secondary),
        )));
        lines.push(Line::from(Span::styled(
            "  view <name>  |  import <path>  |  export <name>",
            Style::default().fg(state.theme.secondary),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  view names: player library lyrics visualizer",
            Style::default().fg(state.theme.muted),
        )));
        lines.push(Line::from(Span::styled(
            "               playlists browser settings",
            Style::default().fg(state.theme.muted),
        )));
        lines.push(Line::from(Span::styled(
            "  repeat: sequential shuffle single",
            Style::default().fg(state.theme.muted),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Enter=执行  Esc=取消",
            Style::default().fg(state.theme.success),
        )));

        let para = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Command ")
                .border_style(Style::default().fg(state.theme.warning)),
        );
        f.render_widget(para, popup);
        return;
    }

    match state.view.active_view {
        ViewMode::Player => {
            let params = crate::ui::views::player_view::PlayerViewParams {
                theme: &state.theme,
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
                cover_gen: state.player.cover_gen.get(),
                cover_lines_cache: &state.player.cover_lines_cache,
                lyric_track: state.lyrics.lyric_track.as_ref(),
                current_lyric_index: state.lyrics.current_lyric_index,
                visualizer_data: &state.visualizer_data,
                playlist_state: &state.playlist_state,
                playing_index: state.player.playing_index,
                tracks: &state.player.tracks,
                active_playlist: state.active_playlist,
                search_active: state.search_mode,
                search_query: &state.search_query,
                selected_index: state.player.selected_index,
            };
            crate::ui::views::player_view::render_player_view(f, f.area(), &params);
        }
        ViewMode::Lyrics => {
            if let Some(ref track) = state.lyrics.lyric_track {
                crate::ui::views::lyrics_view::render_lyrics_view(
                    f,
                    f.area(),
                    &state.theme,
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
                &state.theme,
                &state.visualizer_data,
            );
        }
        ViewMode::Playlists => {
            crate::ui::views::playlist_view::render_playlist_view(
                f,
                f.area(),
                &state.theme,
                &state.playlist_state,
            );
        }
        ViewMode::Browser => {
            crate::ui::views::file_browser_view::render_file_browser(
                f,
                f.area(),
                &state.theme,
                &state.file_browser_state,
            );
        }
        ViewMode::Settings => {
            crate::ui::views::settings_view::render_settings_view(
                f,
                f.area(),
                &state.theme,
                &state.settings_state,
            );
        }
        ViewMode::Library => {
            crate::ui::views::library_view::render_library_view(
                f,
                f.area(),
                &state.theme,
                &state.library_state,
            );
        }
    }
}

fn terminal_is_too_small(area: Rect) -> bool {
    area.width < MIN_TERMINAL_WIDTH || area.height < MIN_TERMINAL_HEIGHT
}

fn render_too_small(f: &mut Frame, theme: &Theme) {
    let area = f.area();
    let message = format!(
        "Terminal too small\n{}×{}  need at least {}×{}",
        area.width, area.height, MIN_TERMINAL_WIDTH, MIN_TERMINAL_HEIGHT
    );
    let paragraph = Paragraph::new(message)
        .style(Style::default().fg(theme.warning))
        .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(paragraph, area);
}

pub fn format_duration(secs: f64) -> String {
    let total_secs = secs as u64;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{:02}:{:02}", mins, secs)
}

/// Indices of `tracks` whose title, artist, or path contains `query`
/// (case-insensitive substring). An empty/blank query matches everything.
pub(crate) fn search_matches(tracks: &[TrackDisplay], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..tracks.len()).collect();
    }
    tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            t.title.to_lowercase().contains(&q)
                || t.artist.to_lowercase().contains(&q)
                || t.path.to_string_lossy().to_lowercase().contains(&q)
        })
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn tiny_terminal_sizes_render_without_panicking() {
        for (width, height) in [(1, 1), (3, 3), (20, 5), (29, 7)] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).expect("test terminal");
            let state = UiState::default();
            terminal
                .draw(|frame| render(frame, &state))
                .expect("tiny terminal should render safely");
        }
    }

    #[test]
    fn minimum_supported_size_renders_main_view() {
        for view in [
            ViewMode::Player,
            ViewMode::Library,
            ViewMode::Lyrics,
            ViewMode::Visualizer,
            ViewMode::Playlists,
            ViewMode::Browser,
            ViewMode::Settings,
        ] {
            let backend = TestBackend::new(MIN_TERMINAL_WIDTH, MIN_TERMINAL_HEIGHT);
            let mut terminal = Terminal::new(backend).expect("test terminal");
            let mut state = UiState::default();
            state.view.active_view = view;
            terminal
                .draw(|frame| render(frame, &state))
                .expect("minimum supported terminal should render every view");
        }
    }
}
