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
use crate::ui::{self, RepeatMode, TrackDisplay, UiState, ViewMode};
use crate::visualizer::fft::FftAnalyzer;
use crate::visualizer::processor::SpectrumProcessor;

pub struct App {
    ui_state: UiState,
    engine: AudioEngine,
    should_quit: bool,
    search_mode: bool,
    key_handler: KeyHandler,
    fft_running: Arc<Mutex<bool>>,
    fft_data: Arc<Mutex<Vec<f32>>>,
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
        *self.fft_running.lock().unwrap() = false;
        disable_raw_mode().ok();
        execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();

        Ok(())
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Quit => {
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

                    // ── View switching ──
                    KeyCode::Char('4') => {
                        self.ui_state.active_view = match self.ui_state.active_view {
                            ViewMode::Visualizer => ViewMode::Player,
                            _ => ViewMode::Visualizer,
                        };
                    }
                    KeyCode::Char('3') => {
                        self.ui_state.active_view = match self.ui_state.active_view {
                            ViewMode::Lyrics => ViewMode::Player,
                            _ => ViewMode::Lyrics,
                        };
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
                        self.search_mode = false;
                        self.ui_state.search_query.clear();
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
        *self.fft_running.lock().unwrap() = true;
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
