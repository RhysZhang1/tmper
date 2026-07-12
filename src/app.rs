use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{Event as CrosstermEvent, EventStream, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures_util::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::audio::engine::AudioEngine;
use crate::cli::{Cli, Command};
use crate::config::Config;
use crate::event::AppEvent;
use crate::input::handler::KeyHandler;
use crate::lyrics::engine::LyricEngine;
use crate::metadata::reader::read_metadata;
use crate::ui::views::file_browser_view::{BrowserPanel, FsItem};
use crate::ui::views::playlist_view::{InsertMode, PlaylistPanel};
use crate::ui::{self, RepeatMode, TrackDisplay, UiState, ViewMode};
use crate::visualizer::fft::FftAnalyzer;
use crate::visualizer::processor::SpectrumProcessor;
use serde::{Deserialize, Serialize};

pub struct App {
    ui_state: UiState,
    engine: AudioEngine,
    should_quit: bool,
    search_mode: bool,
    key_handler: KeyHandler,
    fft_running: Arc<Mutex<bool>>,
    fft_data: Arc<Mutex<Vec<f32>>>,
}

#[derive(Serialize, Deserialize)]
struct SavedState {
    volume: f32,
    repeat_mode: String,
    shuffle: bool,
    lyrics_offset_ms: i64,
    last_track_path: Option<String>,
}

impl App {
    pub fn new(config: &Config) -> crate::error::AppResult<Self> {
        let engine = AudioEngine::new()?;
        Ok(Self {
            ui_state: UiState {
                volume: config.playback.default_volume,
                ..Default::default()
            },
            engine,
            should_quit: false,
            search_mode: false,
            key_handler: KeyHandler::new(200),
            fft_running: Arc::new(Mutex::new(false)),
            fft_data: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub async fn run(&mut self, cli: Cli) -> crate::error::AppResult<()> {
        enable_raw_mode().map_err(|e| {
            crate::error::AppError::Config(format!("Failed to enable raw mode: {e}"))
        })?;

        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen).map_err(|e| {
            crate::error::AppError::Config(format!("Failed to enter alternate screen: {e}"))
        })?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).map_err(|e| {
            crate::error::AppError::Config(format!("Failed to create terminal: {e}"))
        })?;

        self.load_library_paths();
        if let Some(Command::Play { file }) = cli.command {
            self.load_and_play(&file);
            self.start_fft();
        }

        let mut reader = EventStream::new();
        let mut tick_interval = tokio::time::interval(Duration::from_millis(100));

        loop {
            tokio::select! {
                crossterm_event = reader.next() => {
                    match crossterm_event {
                        Some(Ok(event)) => {
                            match event {
                                CrosstermEvent::Key(key) => {
                                    if let Some(app_event) = self.key_handler.process(key) {
                                        self.handle_event(app_event);
                                    }
                                }
                                CrosstermEvent::Resize(_, _) => {}
                                _ => {}
                            }
                        }
                        Some(Err(e)) => {
                            tracing::error!("Crossterm event error: {e}");
                        }
                        None => break,
                    }
                }
                _ = tick_interval.tick() => {
                    self.handle_event(AppEvent::Tick);
                }
            }

            if self.should_quit {
                break;
            }

            if let Err(e) = terminal.draw(|f| ui::render(f, &self.ui_state)) {
                tracing::error!("Render error: {e}");
            }
        }

        // Stop FFT
        *self.fft_running.lock().expect("fft_running mutex poisoned") = false;
        disable_raw_mode().ok();
        execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();

        Ok(())
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Quit => {
                self.save_state();
                self.should_quit = true;
            }
            AppEvent::JumpTop => {
                self.ui_state.selected_index = 0;
                self.ui_state.scroll_offset = 0;
            }
            AppEvent::JumpBottom => {
                let len = self.ui_state.tracks.len();
                if len > 0 {
                    self.ui_state.selected_index = len.saturating_sub(1);
                }
            }
            AppEvent::VisualizerData(bars) => {
                self.ui_state.visualizer_data = bars;
            }
            AppEvent::RemoveSelected => {
                let idx = self.ui_state.selected_index;
                if idx < self.ui_state.tracks.len() {
                    if self.ui_state.playing_index == Some(idx) {
                        self.ui_state.is_playing = false;
                        self.engine.stop();
                        self.ui_state.playing_index = None;
                        self.ui_state.lyric_track = None;
                    }
                    self.ui_state.tracks.remove(idx);
                    if self.ui_state.selected_index >= self.ui_state.tracks.len() {
                        self.ui_state.selected_index = self.ui_state.tracks.len().saturating_sub(1);
                    }
                    if let Some(pi) = self.ui_state.playing_index {
                        if pi > idx {
                            self.ui_state.playing_index = Some(pi - 1);
                        }
                    }
                }
            }
            AppEvent::Key(key) => {
                // ── View switching: works in ALL views ──
                match key.code {
                    KeyCode::Char('1') => self.ui_state.active_view = ViewMode::Player,
                    KeyCode::Char('2') => {
                        self.ui_state.active_view =
                            if self.ui_state.active_view == ViewMode::Library {
                                ViewMode::Player
                            } else {
                                ViewMode::Library
                            };
                    }
                    KeyCode::Char('3') => {
                        self.ui_state.active_view = if self.ui_state.active_view == ViewMode::Lyrics
                        {
                            ViewMode::Player
                        } else {
                            ViewMode::Lyrics
                        };
                    }
                    KeyCode::Char('4') => {
                        self.ui_state.active_view =
                            if self.ui_state.active_view == ViewMode::Visualizer {
                                ViewMode::Player
                            } else {
                                ViewMode::Visualizer
                            };
                    }
                    KeyCode::Char('5') => {
                        if self.ui_state.active_view != ViewMode::Playlists {
                            self.enter_playlist_view();
                        }
                        self.ui_state.active_view =
                            if self.ui_state.active_view == ViewMode::Playlists {
                                ViewMode::Player
                            } else {
                                ViewMode::Playlists
                            };
                    }
                    KeyCode::Char('6') => {
                        if self.ui_state.active_view != ViewMode::Browser {
                            self.enter_file_browser();
                        }
                        self.ui_state.active_view =
                            if self.ui_state.active_view == ViewMode::Browser {
                                ViewMode::Player
                            } else {
                                ViewMode::Browser
                            };
                    }
                    KeyCode::Char('0') => {
                        self.ui_state.show_help = !self.ui_state.show_help;
                    }
                    KeyCode::Esc => {
                        if self.ui_state.show_help {
                            self.ui_state.show_help = false;
                        } else {
                            self.search_mode = false;
                            self.ui_state.search_query.clear();
                        }
                    }
                    _ => {
                        // View-specific interception for non-number keys
                        match self.ui_state.active_view {
                            ViewMode::Playlists => {
                                self.handle_playlist_key(&key);
                                return;
                            }
                            ViewMode::Browser => {
                                self.handle_file_browser_key(&key);
                                return;
                            }
                            _ => {}
                        }
                    }
                }

                // ── Normal keys (only when not in a view-specific mode) ──
                let visible_h = 10u16;
                match key.code {
                    // ── Playback ──
                    KeyCode::Char(' ') => {
                        if self.engine.is_playing() || self.ui_state.is_playing {
                            self.engine.pause();
                            self.ui_state.is_playing = false;
                        } else {
                            self.engine.resume();
                            self.ui_state.is_playing = true;
                        }
                    }
                    KeyCode::Char('-') => {
                        let new_vol = (self.ui_state.volume - 0.05).max(0.0);
                        self.ui_state.volume = new_vol;
                        self.engine.set_volume(new_vol);
                    }
                    KeyCode::Char('=') => {
                        let new_vol = (self.ui_state.volume + 0.05).min(1.0);
                        self.ui_state.volume = new_vol;
                        self.engine.set_volume(new_vol);
                    }

                    // ── Navigation ──
                    KeyCode::Char('j') | KeyCode::Down => {
                        self.move_selection(1, visible_h);
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        self.move_selection(-1, visible_h);
                    }
                    KeyCode::Char('G') => {
                        let len = self.ui_state.tracks.len();
                        if len > 0 {
                            self.ui_state.selected_index = len.saturating_sub(1);
                        }
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let half = (visible_h / 2).max(1) as i32;
                        self.move_selection(-half, visible_h);
                        self.move_scroll(-half);
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let half = (visible_h / 2).max(1) as i32;
                        self.move_selection(half, visible_h);
                        self.move_scroll(half);
                    }
                    KeyCode::Char('g') | KeyCode::Char('d') => {}

                    // ── Track change ──
                    KeyCode::Char('n') => {
                        if let Some(idx) = self.ui_state.playing_index {
                            if idx + 1 < self.ui_state.tracks.len() {
                                self.ui_state.selected_index = idx + 1;
                                let path = self.ui_state.tracks[idx + 1].path.clone();
                                self.load_and_play(&path);
                            }
                        }
                    }
                    KeyCode::Char('p') => {
                        if let Some(idx) = self.ui_state.playing_index {
                            if idx > 0 {
                                self.ui_state.selected_index = idx - 1;
                                let path = self.ui_state.tracks[idx - 1].path.clone();
                                self.load_and_play(&path);
                            }
                        }
                    }

                    // ── Mode ──
                    KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.ui_state.lyrics_offset_ms = 0;
                    }
                    KeyCode::Char('r') => {
                        self.ui_state.repeat_mode = match self.ui_state.repeat_mode {
                            RepeatMode::Off => RepeatMode::Track,
                            RepeatMode::Track => RepeatMode::Playlist,
                            RepeatMode::Playlist => RepeatMode::Off,
                        };
                    }
                    KeyCode::Char('R') => {
                        self.ui_state.shuffle = !self.ui_state.shuffle;
                    }

                    // ── Play selected ──
                    KeyCode::Enter => {
                        self.play_selected();
                    }

                    // ── Lyrics offset ──
                    KeyCode::Char('[') => self.ui_state.lyrics_offset_ms -= 500,
                    KeyCode::Char(']') => self.ui_state.lyrics_offset_ms += 500,
                    KeyCode::Char('{') => self.ui_state.lyrics_offset_ms -= 2000,
                    KeyCode::Char('}') => self.ui_state.lyrics_offset_ms += 2000,

                    // ── Search ──
                    KeyCode::Char('/') => {
                        self.search_mode = true;
                        self.ui_state.search_query.clear();
                    }
                    KeyCode::Esc => {
                        if self.ui_state.show_help {
                            self.ui_state.show_help = false;
                        } else {
                            self.search_mode = false;
                            self.ui_state.search_query.clear();
                        }
                    }

                    _ => {
                        if self.search_mode {
                            if let KeyCode::Char(c) = key.code {
                                if c != '/' {
                                    self.ui_state.search_query.push(c);
                                }
                            } else if key.code == KeyCode::Backspace {
                                self.ui_state.search_query.pop();
                            }
                        }
                    }
                }
            }
            AppEvent::Tick => {
                let pos = self.engine.position_secs();
                self.ui_state.position = pos;

                if let Some(dur) = self.engine.duration_secs() {
                    self.ui_state.duration = dur;
                }

                // Read FFT data
                if let Ok(data) = self.fft_data.lock() {
                    if !data.is_empty() {
                        self.ui_state.visualizer_data = data.clone();
                    } else if !self.ui_state.is_playing {
                        // Decay on pause
                        let mut bars = self.ui_state.visualizer_data.clone();
                        for v in bars.iter_mut() {
                            *v *= 0.9;
                            if *v < 0.01 {
                                *v = 0.0;
                            }
                        }
                        self.ui_state.visualizer_data = bars;
                    }
                }

                self.sync_lyrics(pos);

                if self.ui_state.is_playing
                    && self.ui_state.duration > 0.0
                    && pos >= self.ui_state.duration
                {
                    self.on_track_ended();
                }
            }
        }
    }

    fn move_selection(&mut self, delta: i32, visible_h: u16) {
        if self.ui_state.tracks.is_empty() {
            return;
        }
        let len = self.ui_state.tracks.len() as i32;
        let new_idx = (self.ui_state.selected_index as i32 + delta).clamp(0, len - 1);
        self.ui_state.selected_index = new_idx as usize;

        let scroll = self.ui_state.scroll_offset as i32;
        let vis = visible_h as i32;
        if new_idx < scroll {
            self.ui_state.scroll_offset = new_idx.max(0) as usize;
        } else if new_idx >= scroll + vis {
            self.ui_state.scroll_offset = (new_idx - vis + 1).max(0) as usize;
        }
    }

    fn move_scroll(&mut self, delta: i32) {
        let new_scroll = (self.ui_state.scroll_offset as i32 + delta).max(0);
        self.ui_state.scroll_offset = new_scroll as usize;
    }

    fn play_selected(&mut self) {
        if self.ui_state.selected_index < self.ui_state.tracks.len() {
            let path = self.ui_state.tracks[self.ui_state.selected_index]
                .path
                .clone();
            self.load_and_play(&path);
        }
    }

    fn on_track_ended(&mut self) {
        if let Some(idx) = self.ui_state.playing_index {
            match self.ui_state.repeat_mode {
                RepeatMode::Track => {
                    if idx < self.ui_state.tracks.len() {
                        let path = self.ui_state.tracks[idx].path.clone();
                        self.load_and_play(&path);
                        return;
                    }
                }
                RepeatMode::Playlist => {
                    let next = (idx + 1) % self.ui_state.tracks.len();
                    self.ui_state.selected_index = next;
                    let path = self.ui_state.tracks[next].path.clone();
                    self.load_and_play(&path);
                    return;
                }
                RepeatMode::Off => {
                    if idx + 1 < self.ui_state.tracks.len() {
                        self.ui_state.selected_index = idx + 1;
                        let path = self.ui_state.tracks[idx + 1].path.clone();
                        self.load_and_play(&path);
                        return;
                    }
                }
            }
        }

        self.ui_state.is_playing = false;
        self.engine.stop();
    }

    fn start_fft(&mut self) {
        *self.fft_running.lock().expect("fft_running mutex poisoned") = true;
        let pcm_buf = self.engine.pcm_buffer.clone();
        let running = self.fft_running.clone();
        let fft_data = self.fft_data.clone();

        tokio::task::spawn_blocking(move || {
            let fft_size = 2048;
            let mut analyzer = FftAnalyzer::new(fft_size);
            let mut processor = SpectrumProcessor::new(32, 0.35);

            while *running.lock().unwrap() {
                let samples: Vec<f32> = {
                    let buf = pcm_buf.lock().unwrap();
                    if buf.len() < fft_size {
                        drop(buf);
                        std::thread::sleep(Duration::from_millis(16));
                        continue;
                    }
                    buf.iter().take(fft_size).copied().collect()
                };

                if samples.len() < fft_size {
                    std::thread::sleep(Duration::from_millis(16));
                    continue;
                }

                let magnitudes = analyzer.process(&samples);
                let bars = processor.process(&magnitudes, 44100);

                if let Ok(mut data) = fft_data.lock() {
                    *data = bars;
                }

                std::thread::sleep(Duration::from_millis(32));
            }
        });
    }
    fn load_lyrics_for_current(&mut self) {
        if let Some(idx) = self.ui_state.playing_index {
            if idx < self.ui_state.tracks.len() {
                let path = &self.ui_state.tracks[idx].path.clone();
                match LyricEngine::load(path) {
                    Ok(Some(track)) => {
                        tracing::info!("Lyrics loaded: {} lines", track.lines.len());
                        self.ui_state.lyric_track = Some(track);
                        self.ui_state.current_lyric_index = 0;
                    }
                    Ok(None) => {
                        self.ui_state.lyric_track = None;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load lyrics: {e}");
                        self.ui_state.lyric_track = None;
                    }
                }
            }
        }
    }

    fn sync_lyrics(&mut self, position_secs: f64) {
        if let Some(ref track) = self.ui_state.lyric_track {
            if track.lines.is_empty() {
                return;
            }
            let adjusted_pos = position_secs + self.ui_state.lyrics_offset_ms as f64 / 1000.0;
            let idx = LyricEngine::sync(
                track,
                adjusted_pos.max(0.0),
                self.ui_state.current_lyric_index,
            );
            self.ui_state.current_lyric_index = idx;
        }
    }

    fn save_state(&self) {
        let state_path = crate::paths::data_dir().join("state.json");

        if let Some(parent) = state_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let saved = SavedState {
            volume: self.ui_state.volume,
            repeat_mode: format!("{:?}", self.ui_state.repeat_mode),
            shuffle: self.ui_state.shuffle,
            lyrics_offset_ms: self.ui_state.lyrics_offset_ms,
            last_track_path: self
                .ui_state
                .playing_index
                .and_then(|i| self.ui_state.tracks.get(i))
                .map(|t| t.path.to_string_lossy().to_string()),
        };

        if let Ok(json) = serde_json::to_string_pretty(&saved) {
            if let Err(e) = std::fs::write(&state_path, json) {
                tracing::warn!("Failed to save state: {e}");
            }
        }
    }

    #[allow(dead_code)]
    fn enter_playlist_view(&mut self) {
        // Populate library paths from current tracks
        self.ui_state.playlist_state.library_paths = self
            .ui_state
            .tracks
            .iter()
            .map(|t| t.path.clone())
            .collect();
    }

    #[allow(dead_code)]
    fn enter_file_browser(&mut self) {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"));
        self.ui_state.file_browser_state.current_dir = home;
        self.refresh_file_browser();
        self.ui_state.file_browser_state.library_paths = self
            .ui_state
            .tracks
            .iter()
            .map(|t| t.path.clone())
            .collect();
    }

    fn refresh_file_browser(&mut self) {
        let dir = self.ui_state.file_browser_state.current_dir.clone();
        let mut dirs = Vec::new();
        let mut audios = Vec::new();
        let mut items = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&dir) {
            let mut all: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            all.sort_by_key(|e| e.file_name());
            for entry in all {
                let path = entry.path();
                if path.is_dir() {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?")
                        .to_string();
                    dirs.push(path.clone());
                    items.push(FsItem::Dir(name));
                } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    let ext_lower = ext.to_lowercase();
                    if [
                        "mp3", "flac", "ogg", "opus", "wav", "aac", "m4a", "ape", "wv", "aiff",
                        "wma",
                    ]
                    .contains(&ext_lower.as_str())
                    {
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("?")
                            .to_string();
                        audios.push(path.clone());
                        items.push(FsItem::Audio(name));
                    }
                }
            }
        }
        self.ui_state.file_browser_state.dirs = dirs;
        self.ui_state.file_browser_state.audio_files = audios;
        self.ui_state.file_browser_state.fs_items = items;
        self.ui_state.file_browser_state.selected_fs_index = 0;
    }

    fn handle_playlist_key(&mut self, key: &crossterm::event::KeyEvent) {
        let state = &mut self.ui_state.playlist_state;

        // Handle insert mode
        if let InsertMode::Typing(ref s) = state.insert_mode {
            match key.code {
                KeyCode::Enter => {
                    let name = s.clone();
                    if !name.is_empty() {
                        state
                            .playlists
                            .push(crate::ui::views::playlist_view::PlaylistData {
                                name,
                                songs: Vec::new(),
                                expanded: false,
                            });
                    }
                    state.insert_mode = InsertMode::Off;
                }
                KeyCode::Esc => {
                    state.insert_mode = InsertMode::Off;
                }
                KeyCode::Backspace => {
                    let mut new_s = s.clone();
                    new_s.pop();
                    state.insert_mode = InsertMode::Typing(new_s);
                }
                KeyCode::Char(c) => {
                    let mut new_s = s.clone();
                    new_s.push(c);
                    state.insert_mode = InsertMode::Typing(new_s);
                }
                _ => {}
            }
            return;
        }

        // Dismiss notification
        state.notification = None;

        match key.code {
            KeyCode::Tab | KeyCode::Char('l') => {
                state.focused = match state.focused {
                    PlaylistPanel::Library => PlaylistPanel::Playlists,
                    PlaylistPanel::Playlists => PlaylistPanel::Library,
                };
            }
            KeyCode::Char('h') => {
                state.focused = PlaylistPanel::Library;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                match state.focused {
                    PlaylistPanel::Library => {
                        let max = state.library_paths.len().saturating_sub(1);
                        state.selected_library_song = (state.selected_library_song + 1).min(max);
                    }
                    PlaylistPanel::Playlists => {
                        let max = state.playlists.len(); // 0 for "..."
                        let sel = state.selected_playlist;
                        // Check if we're inside an expanded playlist's songs
                        if let Some(ep) = state.expanded_playlist {
                            if sel > ep + 1 && sel <= ep + 1 + state.playlists[ep].songs.len() {
                                let song_max = state.playlists[ep].songs.len().saturating_sub(1);
                                state.selected_song_in_playlist =
                                    (state.selected_song_in_playlist + 1).min(song_max);
                                return;
                            }
                        }
                        state.selected_playlist = (sel + 1).min(max);
                        state.selected_song_in_playlist = 0;
                    }
                }
            }
            KeyCode::Char('k') | KeyCode::Up => match state.focused {
                PlaylistPanel::Library => {
                    state.selected_library_song = state.selected_library_song.saturating_sub(1);
                }
                PlaylistPanel::Playlists => {
                    if let Some(ep) = state.expanded_playlist {
                        let sel = state.selected_playlist;
                        if sel > ep + 1 && sel <= ep + 1 + state.playlists[ep].songs.len() {
                            state.selected_song_in_playlist =
                                state.selected_song_in_playlist.saturating_sub(1);
                            return;
                        }
                    }
                    state.selected_playlist = state.selected_playlist.saturating_sub(1);
                    state.selected_song_in_playlist = 0;
                }
            },
            KeyCode::Enter => {
                match state.focused {
                    PlaylistPanel::Library => {
                        // Add song to expanded playlist
                        if let Some(ep) = state.expanded_playlist {
                            let lib_idx = state.selected_library_song;
                            if lib_idx < state.library_paths.len() {
                                let path = state.library_paths[lib_idx].clone();
                                if !state.playlists[ep].songs.contains(&path) {
                                    state.playlists[ep].songs.push(path);
                                }
                            }
                        } else {
                            state.notification =
                                Some(("请先展开一个歌单".to_string(), std::time::Instant::now()));
                        }
                    }
                    PlaylistPanel::Playlists => {
                        if state.selected_playlist == 0 {
                            // "..." — create new playlist
                            state.insert_mode = InsertMode::Typing(String::new());
                        } else {
                            let pl_idx = state.selected_playlist.saturating_sub(1);
                            if pl_idx < state.playlists.len() {
                                // Check if selecting a song inside expanded playlist
                                if let Some(ep) = state.expanded_playlist {
                                    if ep == pl_idx && state.selected_playlist > ep + 1 {
                                        let song_idx = state.selected_song_in_playlist;
                                        if song_idx < state.playlists[ep].songs.len() {
                                            state.playlists[ep].songs.remove(song_idx);
                                            return;
                                        }
                                    }
                                }
                                // Toggle expand/collapse
                                if state.expanded_playlist == Some(pl_idx) {
                                    state.expanded_playlist = None;
                                } else {
                                    state.expanded_playlist = Some(pl_idx);
                                }
                                state.selected_song_in_playlist = 0;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_file_browser_key(&mut self, key: &crossterm::event::KeyEvent) {
        let state = &mut self.ui_state.file_browser_state;
        match key.code {
            KeyCode::Tab | KeyCode::Char('l') => {
                state.focused = match state.focused {
                    BrowserPanel::Library => BrowserPanel::Filesystem,
                    BrowserPanel::Filesystem => BrowserPanel::Library,
                };
            }
            KeyCode::Char('h') => state.focused = BrowserPanel::Library,
            KeyCode::Char('j') | KeyCode::Down => {
                match state.focused {
                    BrowserPanel::Library => {
                        let max = state.library_paths.len().saturating_sub(1);
                        state.selected_library_index = (state.selected_library_index + 1).min(max);
                        let vis_h = 10u16;
                        let sel = state.selected_library_index as i32;
                        let scroll = state.scroll_library as i32;
                        if sel >= scroll + vis_h as i32 {
                            state.scroll_library = (sel - vis_h as i32 + 1).max(0) as usize;
                        }
                    }
                    BrowserPanel::Filesystem => {
                        let max = state.fs_items.len(); // total items
                        state.selected_fs_index = (state.selected_fs_index + 1).min(max);
                        // Auto scroll
                        let vis_h = 10u16;
                        let sel = state.selected_fs_index as i32;
                        let scroll = state.scroll_fs as i32;
                        if sel >= scroll + vis_h as i32 {
                            state.scroll_fs = (sel - vis_h as i32 + 1).max(0) as usize;
                        }
                    }
                }
            }
            KeyCode::Char('k') | KeyCode::Up => match state.focused {
                BrowserPanel::Library => {
                    state.selected_library_index = state.selected_library_index.saturating_sub(1);
                }
                BrowserPanel::Filesystem => {
                    state.selected_fs_index = state.selected_fs_index.saturating_sub(1);
                }
            },
            KeyCode::Enter => {
                match state.focused {
                    BrowserPanel::Library => {
                        let idx = state.selected_library_index;
                        let lib_len = state.library_paths.len();
                        if idx < lib_len {
                            let path = state.library_paths.remove(idx);
                            self.ui_state.tracks.retain(|t| t.path != path);
                            let new_len = state.library_paths.len();
                            state.selected_library_index = idx.min(new_len.saturating_sub(1));
                            self.save_library_paths();
                        }
                    }
                    BrowserPanel::Filesystem => {
                        if state.selected_fs_index == 0 {
                            // ".." — go up
                            if let Some(parent) =
                                state.current_dir.parent().map(|p| p.to_path_buf())
                            {
                                state.current_dir = parent;
                                self.refresh_file_browser();
                            }
                        } else {
                            let fs_idx = state.selected_fs_index.saturating_sub(1);
                            if fs_idx < state.fs_items.len() {
                                match &state.fs_items[fs_idx] {
                                    FsItem::Dir(_) => {
                                        state.current_dir = state.dirs[fs_idx].clone();
                                        self.refresh_file_browser();
                                    }
                                    FsItem::Audio(_) => {
                                        let path = state.audio_files[fs_idx].clone();
                                        if !state.library_paths.contains(&path) {
                                            state.library_paths.push(path.clone());
                                            self.load_and_play_collect(&path);
                                            self.save_library_paths();
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                if state.focused == BrowserPanel::Filesystem {
                    if let Some(parent) = state.current_dir.parent().map(|p| p.to_path_buf()) {
                        if parent.starts_with(&state.home_dir) || parent == state.home_dir {
                            state.current_dir = parent;
                            self.refresh_file_browser();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn save_library_paths(&self) {
        let path = crate::paths::data_dir().join("library.json");
        let paths: Vec<String> = self
            .ui_state
            .file_browser_state
            .library_paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        if let Ok(json) = serde_json::to_string_pretty(&paths) {
            let _ = std::fs::write(&path, json);
        }
    }

    fn load_library_paths(&mut self) {
        let path = crate::paths::data_dir().join("library.json");
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(paths) = serde_json::from_str::<Vec<String>>(&content) {
                for p_str in paths {
                    let pb = std::path::PathBuf::from(&p_str);
                    if pb.exists() && !self.ui_state.file_browser_state.library_paths.contains(&pb)
                    {
                        self.load_and_play_collect(&pb);
                    }
                }
            }
        }
    }

    fn load_and_play_collect(&mut self, path: &std::path::PathBuf) {
        if let Ok(info) = read_metadata(path) {
            let title = info.title.clone();
            let artist = info
                .artist
                .clone()
                .unwrap_or_else(|| "Unknown Artist".into());
            let duration = info.duration.as_secs_f64();
            if !self.ui_state.tracks.iter().any(|t| t.path == info.path) {
                self.ui_state.tracks.push(TrackDisplay {
                    path: info.path.clone(),
                    title,
                    artist,
                    duration_secs: duration,
                    is_playing: false,
                });
            }
        }
    }

    fn load_and_play(&mut self, path: &PathBuf) {
        match read_metadata(path) {
            Ok(info) => {
                let title = info.title.clone();
                let artist = info
                    .artist
                    .clone()
                    .unwrap_or_else(|| "Unknown Artist".into());
                let duration = info.duration.as_secs_f64();

                if !self.ui_state.tracks.iter().any(|t| t.path == info.path) {
                    self.ui_state.tracks.push(TrackDisplay {
                        path: info.path.clone(),
                        title: title.clone(),
                        artist: artist.clone(),
                        duration_secs: duration,
                        is_playing: false,
                    });
                }

                if let Some(idx) = self
                    .ui_state
                    .tracks
                    .iter()
                    .position(|t| t.path == info.path)
                {
                    self.ui_state.playing_index = Some(idx);
                    self.ui_state.selected_index = idx;
                }

                match self.engine.play_file(path) {
                    Ok(()) => {
                        self.ui_state.title = title;
                        self.ui_state.artist = artist;
                        self.ui_state.position = 0.0;
                        self.ui_state.duration = duration;
                        self.ui_state.is_playing = true;
                        self.engine.set_volume(self.ui_state.volume);
                        self.load_lyrics_for_current();
                        self.start_fft();
                        tracing::info!("Now playing: {:?}", path);
                    }
                    Err(e) => {
                        tracing::error!("Failed to play file: {e}");
                        self.ui_state.title = format!("Error: {e}");
                        self.ui_state.is_playing = false;
                    }
                }
            }
            Err(e) => {
                tracing::error!("Failed to read metadata: {e}");
            }
        }
    }
}
