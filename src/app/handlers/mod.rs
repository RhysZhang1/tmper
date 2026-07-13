mod browser;
mod library;
mod playlist;
mod settings;

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;
use crate::event::AppEvent;
use crate::lyrics::engine::LyricEngine;
use crate::ui::views::playlist_view::{InsertMode, LineTarget, PlaylistFlatModel};
use crate::ui::{RepeatMode, ViewMode};
use crate::visualizer::fft::FftAnalyzer;
use crate::visualizer::processor::SpectrumProcessor;

impl App {
    pub(super) fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Quit => {
                self.save_state();
                self.should_quit = true;
            }
            AppEvent::JumpTop => {
                self.ui_state.selected_index = 0;
                self.ui_state.scroll_offset = 0;
            }
            AppEvent::RemoveSelected => self.handle_remove_selected(),
            AppEvent::Key(key) => self.handle_key_event(key),
            AppEvent::Tick => self.handle_tick(),
        }
    }

    // ── Key event dispatch ──

    fn handle_key_event(&mut self, key: KeyEvent) {
        // Library search mode: ALL keys (including numbers, q, etc.) go to search input
        if self.ui_state.library_state.search_mode {
            self.handle_library_key(&key);
            return;
        }

        // Insert mode: all keys pass through to playlist handler
        if matches!(
            self.ui_state.playlist_state.insert_mode,
            InsertMode::Typing(_)
        ) {
            self.handle_playlist_key(&key);
            return;
        }

        // View switching (works in all views)
        match key.code {
            KeyCode::Char('1') => self.ui_state.active_view = ViewMode::Player,
            KeyCode::Char('2') => {
                let switching_to_library = self.ui_state.active_view != ViewMode::Library;
                self.ui_state.active_view = if switching_to_library {
                    self.ensure_library_loaded();
                    ViewMode::Library
                } else {
                    ViewMode::Player
                };
            }
            KeyCode::Char('3') => {
                self.ui_state.active_view = if self.ui_state.active_view == ViewMode::Lyrics {
                    ViewMode::Player
                } else {
                    ViewMode::Lyrics
                };
            }
            KeyCode::Char('4') => {
                self.ui_state.active_view = if self.ui_state.active_view == ViewMode::Visualizer {
                    ViewMode::Player
                } else {
                    ViewMode::Visualizer
                };
            }
            KeyCode::Char('5') => {
                if self.ui_state.active_view != ViewMode::Playlists {
                    self.enter_playlist_view();
                }
                self.ui_state.active_view = if self.ui_state.active_view == ViewMode::Playlists {
                    ViewMode::Player
                } else {
                    ViewMode::Playlists
                };
            }
            KeyCode::Char('6') => {
                if self.ui_state.active_view != ViewMode::Browser {
                    self.enter_file_browser();
                }
                self.ui_state.active_view = if self.ui_state.active_view == ViewMode::Browser {
                    ViewMode::Player
                } else {
                    ViewMode::Browser
                };
            }
            KeyCode::Char('7') => {
                if self.ui_state.active_view != ViewMode::Settings {
                    crate::ui::views::settings_view::rebuild_settings(
                        &mut self.ui_state.settings_state,
                        &self.config,
                    );
                }
                self.ui_state.active_view = if self.ui_state.active_view == ViewMode::Settings {
                    ViewMode::Player
                } else {
                    ViewMode::Settings
                };
            }
            KeyCode::Char('0') => self.ui_state.show_help = !self.ui_state.show_help,
            KeyCode::Esc => {
                if self.ui_state.show_help {
                    self.ui_state.show_help = false;
                } else {
                    self.search_mode = false;
                    self.ui_state.search_query.clear();
                }
            }
            _ => {
                // View-specific interception
                match self.ui_state.active_view {
                    ViewMode::Playlists => {
                        self.handle_playlist_key(&key);
                        return;
                    }
                    ViewMode::Browser => {
                        self.handle_file_browser_key(&key);
                        return;
                    }
                    ViewMode::Library => {
                        self.handle_library_key(&key);
                        return;
                    }
                    ViewMode::Settings => {
                        self.handle_settings_key(&key);
                        return;
                    }
                    _ => {}
                }
            }
        }

        // Player View: sidebar playlist navigation
        if self.ui_state.active_view == ViewMode::Player
            && self.handle_player_view_sidebar_key(&key)
        {
            return;
        }

        // Global key bindings
        self.handle_global_key(key);
    }

    /// Returns true if the key was consumed by the sidebar.
    fn handle_player_view_sidebar_key(&mut self, key: &KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let ps = &mut self.ui_state.playlist_state;
                let model = PlaylistFlatModel::new(&ps.playlists, ps.expanded_playlist);
                let max_idx = model.total_lines().saturating_sub(2); // skip "..." row
                if ps.selected_playlist < max_idx {
                    ps.selected_playlist += 1;
                }
                Self::clamp_playlist_scroll(ps);
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let ps = &mut self.ui_state.playlist_state;
                if ps.selected_playlist > 0 {
                    ps.selected_playlist -= 1;
                }
                if ps.selected_playlist < ps.scroll_playlists {
                    ps.scroll_playlists = ps.selected_playlist;
                }
                true
            }
            KeyCode::Enter => {
                let ps = &self.ui_state.playlist_state;
                let model = PlaylistFlatModel::new(&ps.playlists, ps.expanded_playlist);
                match model.resolve(ps.selected_playlist) {
                    LineTarget::Song {
                        playlist,
                        song_index,
                    } => {
                        let pl_idx = ps.expanded_playlist.unwrap_or(playlist);
                        if pl_idx < ps.playlists.len()
                            && song_index < ps.playlists[pl_idx].songs.len()
                        {
                            let path = ps.playlists[pl_idx].songs[song_index].clone();
                            // Set this as the active playlist for scoped playback
                            self.ui_state.active_playlist = Some(pl_idx);
                            self.ui_state.active_playlist_song = Some(song_index);
                            // Update playlist_name for display
                            self.ui_state.playlist_name = ps.playlists[pl_idx].name.clone();
                            let _ = ps;
                            let _ = model;
                            self.load_and_play(&path);
                            return true;
                        }
                    }
                    LineTarget::PlaylistName(i) => {
                        let _ = model;
                        let ps = &mut self.ui_state.playlist_state;
                        // Toggle expand
                        if ps.expanded_playlist == Some(i) {
                            ps.expanded_playlist = None;
                        } else {
                            ps.expanded_playlist = Some(i);
                            // Set as active playlist on expand
                            self.ui_state.active_playlist = Some(i);
                            self.ui_state.playlist_name = ps.playlists[i].name.clone();
                        }
                        let new_model = PlaylistFlatModel::new(&ps.playlists, ps.expanded_playlist);
                        if let Some(new_line) = new_model.line_of_playlist(i) {
                            ps.selected_playlist = new_line;
                        }
                        Self::clamp_playlist_scroll(ps);
                    }
                    _ => {}
                }
                true
            }
            _ => false,
        }
    }

    fn handle_global_key(&mut self, key: KeyEvent) {
        let visible_h = 10u16;
        match key.code {
            // Playback
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
            // Seeking
            KeyCode::Left => {
                if let Err(e) = self
                    .engine
                    .seek_relative(-(self.config.playback.seek_step_small_secs as f64))
                {
                    tracing::error!("Seek error: {e}");
                }
            }
            KeyCode::Right => {
                if let Err(e) = self
                    .engine
                    .seek_relative(self.config.playback.seek_step_small_secs as f64)
                {
                    tracing::error!("Seek error: {e}");
                }
            }
            // Navigation
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1, visible_h),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1, visible_h),
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
            // Track change
            KeyCode::Char('n') => self.next_track(),
            KeyCode::Char('p') => self.prev_track(),
            // Modes
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.ui_state.lyrics_offset_ms = 0;
            }
            KeyCode::Char('r') => {
                self.ui_state.repeat_mode = match self.ui_state.repeat_mode {
                    RepeatMode::Sequential => RepeatMode::Shuffle,
                    RepeatMode::Shuffle => RepeatMode::SingleTrack,
                    RepeatMode::SingleTrack => RepeatMode::Sequential,
                };
                let label = self.ui_state.repeat_mode.label().to_string();
                self.ui_state.notification = Some((label, std::time::Instant::now()));
            }
            KeyCode::Enter => self.play_selected(),
            // Lyrics offset
            KeyCode::Char('[') => self.ui_state.lyrics_offset_ms -= 500,
            KeyCode::Char(']') => self.ui_state.lyrics_offset_ms += 500,
            KeyCode::Char('{') => self.ui_state.lyrics_offset_ms -= 2000,
            KeyCode::Char('}') => self.ui_state.lyrics_offset_ms += 2000,
            // Search
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.ui_state.search_query.clear();
            }
            _ => {
                if self.search_mode {
                    self.handle_search_input(key);
                }
            }
        }
    }

    fn handle_search_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c) if c != '/' => self.ui_state.search_query.push(c),
            KeyCode::Backspace => {
                self.ui_state.search_query.pop();
            }
            KeyCode::Esc => {
                self.search_mode = false;
                self.ui_state.search_query.clear();
            }
            _ => {}
        }
    }

    fn handle_remove_selected(&mut self) {
        if self.ui_state.active_view == ViewMode::Playlists {
            let state = &mut self.ui_state.playlist_state;
            if state.selected_playlist > 0 {
                let model = PlaylistFlatModel::new(&state.playlists, state.expanded_playlist);
                if let LineTarget::PlaylistName(i) = model.resolve(state.selected_playlist) {
                    state.playlists.remove(i);
                    state.selected_playlist = state.selected_playlist.min(state.playlists.len());
                    state.expanded_playlist = None;
                    self.save_playlists();
                }
            }
            return;
        }
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

    fn handle_tick(&mut self) {
        // Notification auto-dismiss
        if let Some((_, time)) = &self.ui_state.playlist_state.notification {
            if time.elapsed().as_secs_f64() >= 1.0 {
                self.ui_state.playlist_state.notification = None;
            }
        }
        // Mode change notification auto-dismiss (0.5s)
        if let Some((_, time)) = &self.ui_state.notification {
            if time.elapsed().as_secs_f64() >= 0.5 {
                self.ui_state.notification = None;
            }
        }

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

        if self.ui_state.is_playing && self.ui_state.duration > 0.0 && pos >= self.ui_state.duration
        {
            self.on_track_ended();
        }

        // Sync config flag to UI state (user may have toggled in settings)
        self.ui_state.show_cover_art = self.config.ui.show_cover_art;
    }

    // ── Helpers ──

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

    fn next_track(&mut self) {
        if let Some(pl_idx) = self.ui_state.active_playlist {
            let pls = &self.ui_state.playlist_state.playlists;
            if pl_idx < pls.len() {
                let songs = &pls[pl_idx].songs;
                if songs.is_empty() {
                    return;
                }
                let cur_song = self.ui_state.active_playlist_song.unwrap_or(0);
                let next_song = (cur_song + 1) % songs.len();
                self.ui_state.active_playlist_song = Some(next_song);
                let path = songs[next_song].clone();
                self.load_and_play(&path);
                return;
            }
        }
        // Fallback: global track list
        if let Some(idx) = self.ui_state.playing_index {
            if idx + 1 < self.ui_state.tracks.len() {
                self.ui_state.selected_index = idx + 1;
                let path = self.ui_state.tracks[idx + 1].path.clone();
                self.load_and_play(&path);
            }
        }
    }

    fn prev_track(&mut self) {
        if let Some(pl_idx) = self.ui_state.active_playlist {
            let pls = &self.ui_state.playlist_state.playlists;
            if pl_idx < pls.len() {
                let songs = &pls[pl_idx].songs;
                if songs.is_empty() {
                    return;
                }
                let cur_song = self.ui_state.active_playlist_song.unwrap_or(0);
                let prev_song = if cur_song == 0 {
                    songs.len().saturating_sub(1)
                } else {
                    cur_song - 1
                };
                self.ui_state.active_playlist_song = Some(prev_song);
                let path = songs[prev_song].clone();
                self.load_and_play(&path);
                return;
            }
        }
        // Fallback: global track list
        if let Some(idx) = self.ui_state.playing_index {
            if idx > 0 {
                self.ui_state.selected_index = idx - 1;
                let path = self.ui_state.tracks[idx - 1].path.clone();
                self.load_and_play(&path);
            }
        }
    }

    fn on_track_ended(&mut self) {
        // Try playlist-scoped first
        if let Some(pl_idx) = self.ui_state.active_playlist {
            let songs = {
                let pls = &self.ui_state.playlist_state.playlists;
                if pl_idx < pls.len() {
                    pls[pl_idx].songs.clone()
                } else {
                    Vec::new()
                }
            };
            if !songs.is_empty() {
                match self.ui_state.repeat_mode {
                    RepeatMode::SingleTrack => {
                        let cur = self
                            .ui_state
                            .active_playlist_song
                            .unwrap_or(0)
                            .min(songs.len() - 1);
                        let path = songs[cur].clone();
                        self.load_and_play(&path);
                        return;
                    }
                    RepeatMode::Sequential => {
                        let cur = self.ui_state.active_playlist_song.unwrap_or(0);
                        let next = (cur + 1) % songs.len();
                        self.ui_state.active_playlist_song = Some(next);
                        let path = songs[next].clone();
                        self.load_and_play(&path);
                        return;
                    }
                    RepeatMode::Shuffle => {
                        use rand::Rng;
                        let mut rng = rand::thread_rng();
                        let next = rng.gen_range(0..songs.len());
                        self.ui_state.active_playlist_song = Some(next);
                        let path = songs[next].clone();
                        self.load_and_play(&path);
                        return;
                    }
                }
            }
        }

        // Fallback: global track list
        if let Some(idx) = self.ui_state.playing_index {
            match self.ui_state.repeat_mode {
                RepeatMode::SingleTrack => {
                    if idx < self.ui_state.tracks.len() {
                        let path = self.ui_state.tracks[idx].path.clone();
                        self.load_and_play(&path);
                        return;
                    }
                }
                RepeatMode::Sequential => {
                    let next = (idx + 1) % self.ui_state.tracks.len();
                    self.ui_state.selected_index = next;
                    let path = self.ui_state.tracks[next].path.clone();
                    self.load_and_play(&path);
                    return;
                }
                RepeatMode::Shuffle => {
                    use rand::Rng;
                    let mut rng = rand::thread_rng();
                    let next = rng.gen_range(0..self.ui_state.tracks.len());
                    self.ui_state.selected_index = next;
                    let path = self.ui_state.tracks[next].path.clone();
                    self.load_and_play(&path);
                    return;
                }
            }
        }
        self.ui_state.is_playing = false;
        self.engine.stop();
    }

    pub(crate) fn start_fft(&mut self) {
        // Cancel any existing FFT thread by dropping the old sender.
        self.fft_cancel_tx = None;

        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(());
        self.fft_cancel_tx = Some(cancel_tx);

        let pcm_buf = self.engine.pcm_buffer.clone();
        let fft_data = self.fft_data.clone();
        let num_bars = self.config.visualizer.num_bars as usize;
        let smoothing = self.config.visualizer.smoothing;

        tokio::task::spawn_blocking(move || {
            let fft_size = 2048;
            let mut analyzer = FftAnalyzer::new(fft_size);
            let mut processor = SpectrumProcessor::new(num_bars, smoothing);

            loop {
                // Check cancellation (non-blocking)
                if cancel_rx.has_changed().is_err() {
                    break; // sender dropped
                }

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

    pub(crate) fn load_lyrics_for_current(&mut self) {
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
}
