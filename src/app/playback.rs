use std::time::Duration;

use crate::app::App;
use crate::constants::runtime;
use crate::lyrics::engine::LyricEngine;
use crate::ui::RepeatMode;
use crate::visualizer::fft::FftAnalyzer;
use crate::visualizer::processor::SpectrumProcessor;

impl App {
    /// Move selection by `delta` rows, clamping to track list bounds.
    /// Updates scroll offset to keep selection visible.
    pub(super) fn move_selection(&mut self, delta: i32, visible_h: u16) {
        if self.ui_state.player.tracks.is_empty() {
            return;
        }
        let len = self.ui_state.player.tracks.len() as i32;
        let new_idx = (self.ui_state.player.selected_index as i32 + delta).clamp(0, len - 1);
        self.ui_state.player.selected_index = new_idx as usize;

        let scroll = self.ui_state.player.scroll_offset as i32;
        let vis = visible_h as i32;
        if new_idx < scroll {
            self.ui_state.player.scroll_offset = new_idx.max(0) as usize;
        } else if new_idx >= scroll + vis {
            self.ui_state.player.scroll_offset = (new_idx - vis + 1).max(0) as usize;
        }
    }

    pub(super) fn move_scroll(&mut self, delta: i32) {
        let new_scroll = (self.ui_state.player.scroll_offset as i32 + delta).max(0);
        self.ui_state.player.scroll_offset = new_scroll as usize;
    }

    pub(super) fn play_selected(&mut self) {
        if self.ui_state.player.selected_index < self.ui_state.player.tracks.len() {
            let path = self.ui_state.player.tracks[self.ui_state.player.selected_index]
                .path
                .clone();
            self.load_and_play(&path);
        }
    }

    pub(super) fn next_track(&mut self) {
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
        if let Some(idx) = self.ui_state.player.playing_index {
            if idx + 1 < self.ui_state.player.tracks.len() {
                self.ui_state.player.selected_index = idx + 1;
                let path = self.ui_state.player.tracks[idx + 1].path.clone();
                self.load_and_play(&path);
            }
        }
    }

    pub(super) fn prev_track(&mut self) {
        if let Some(pl_idx) = self.ui_state.active_playlist {
            let pls = &self.ui_state.playlist_state.playlists;
            if pl_idx < pls.len() {
                let songs = &pls[pl_idx].songs;
                if songs.is_empty() {
                    return;
                }
                // `active_playlist_song` is chosen in one view and used in
                // another, so it can outlive the playlist edit that shrank
                // `songs` (removing a track or a whole playlist does not
                // re-point it). Clamp before deriving `prev_song` — `next_track`
                // and `on_track_ended` both guard, and this one did not, so a
                // stale index indexed past the end and panicked.
                let cur_song = self
                    .ui_state
                    .active_playlist_song
                    .unwrap_or(0)
                    .min(songs.len() - 1);
                let prev_song = if cur_song == 0 {
                    songs.len() - 1
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
        if let Some(idx) = self.ui_state.player.playing_index {
            if idx > 0 {
                self.ui_state.player.selected_index = idx - 1;
                let path = self.ui_state.player.tracks[idx - 1].path.clone();
                self.load_and_play(&path);
            }
        }
    }

    pub(super) fn on_track_ended(&mut self) {
        // Reset transient playback and view state before loading next track.
        self.ui_state.player.reset_on_track_change();
        self.ui_state.view.reset_on_track_change();

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
        if let Some(idx) = self.ui_state.player.playing_index {
            match self.ui_state.repeat_mode {
                RepeatMode::SingleTrack => {
                    if idx < self.ui_state.player.tracks.len() {
                        let path = self.ui_state.player.tracks[idx].path.clone();
                        self.load_and_play(&path);
                        return;
                    }
                }
                RepeatMode::Sequential => {
                    let next = (idx + 1) % self.ui_state.player.tracks.len();
                    self.ui_state.player.selected_index = next;
                    let path = self.ui_state.player.tracks[next].path.clone();
                    self.load_and_play(&path);
                    return;
                }
                RepeatMode::Shuffle => {
                    use rand::Rng;
                    let mut rng = rand::thread_rng();
                    let next = rng.gen_range(0..self.ui_state.player.tracks.len());
                    self.ui_state.player.selected_index = next;
                    let path = self.ui_state.player.tracks[next].path.clone();
                    self.load_and_play(&path);
                    return;
                }
            }
        }
        self.ui_state.player.is_playing = false;
        self.engine.stop();
    }

    pub(crate) fn start_fft(&mut self) {
        // Cancel any existing FFT thread by dropping the old sender.
        self.fft_cancel_tx = None;

        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(());
        self.fft_cancel_tx = Some(cancel_tx);

        let pcm_buf = self.engine.pcm_buffer.clone();
        let sample_rate = self.engine.sample_rate_handle();
        let fft_data = self.fft_data.clone();
        let num_bars = self.config.visualizer.num_bars as usize;
        let smoothing = self.config.visualizer.smoothing;

        tokio::task::spawn_blocking(move || {
            let fft_size = runtime::FFT_SIZE;
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
                        std::thread::sleep(Duration::from_millis(runtime::FFT_WAIT_SLEEP_MS));
                        continue;
                    }
                    buf.iter().take(fft_size).copied().collect()
                };

                if samples.len() < fft_size {
                    std::thread::sleep(Duration::from_millis(runtime::FFT_WAIT_SLEEP_MS));
                    continue;
                }

                let magnitudes = analyzer.process(&samples);
                let bars = processor.process(
                    &magnitudes,
                    sample_rate.load(std::sync::atomic::Ordering::Relaxed),
                );

                if let Ok(mut data) = fft_data.lock() {
                    *data = bars;
                }

                std::thread::sleep(Duration::from_millis(runtime::FFT_LOOP_SLEEP_MS));
            }
        });
    }

    pub(crate) fn load_lyrics_for_current(&mut self) {
        if let Some(idx) = self.ui_state.player.playing_index {
            if idx < self.ui_state.player.tracks.len() {
                let path = &self.ui_state.player.tracks[idx].path.clone();
                match LyricEngine::load(path) {
                    Ok(Some(track)) => {
                        tracing::info!("Lyrics loaded: {} lines", track.lines.len());
                        self.ui_state.lyrics.lyric_track = Some(track);
                        self.ui_state.lyrics.current_lyric_index = 0;
                    }
                    Ok(None) => {
                        self.ui_state.lyrics.lyric_track = None;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load lyrics: {e}");
                        self.ui_state.lyrics.lyric_track = None;
                    }
                }
            }
        }
    }

    pub(super) fn sync_lyrics(&mut self, position_secs: f64) {
        if let Some(ref track) = self.ui_state.lyrics.lyric_track {
            if track.lines.is_empty() {
                return;
            }
            let adjusted_pos =
                position_secs + self.ui_state.lyrics.lyrics_offset_ms as f64 / 1000.0;
            let idx = LyricEngine::sync(
                track,
                adjusted_pos.max(0.0),
                self.ui_state.lyrics.current_lyric_index,
            );
            self.ui_state.lyrics.current_lyric_index = idx;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::handlers::test_support::{pl, test_app};
    use std::path::PathBuf;

    /// `prev_track` derives its index from `active_playlist_song`, which is set
    /// in one view and consumed in another — deleting a song or a whole
    /// playlist never re-points it. A stale index used to be used verbatim and
    /// indexed past the end of `songs`.
    #[test]
    fn prev_track_ignores_a_stale_playlist_index() {
        let mut app = test_app();
        app.ui_state.playlist_state.playlists = vec![pl("Mix", &["/a.flac", "/b.flac"])];
        app.ui_state.active_playlist = Some(0);
        // Stale: the playlist has since shrunk to two songs.
        app.ui_state.active_playlist_song = Some(5);

        app.prev_track();

        // Clamped to the last song, then stepped back to the first.
        assert_eq!(app.ui_state.active_playlist_song, Some(0));
    }

    /// The documented behaviour still holds for an in-range index.
    #[test]
    fn prev_track_wraps_from_the_first_song_to_the_last() {
        let mut app = test_app();
        app.ui_state.playlist_state.playlists = vec![pl("Mix", &["/a.flac", "/b.flac"])];
        app.ui_state.active_playlist = Some(0);
        app.ui_state.active_playlist_song = Some(0);

        app.prev_track();

        assert_eq!(app.ui_state.active_playlist_song, Some(1));
    }

    #[test]
    fn prev_track_on_an_empty_playlist_is_a_noop() {
        let mut app = test_app();
        app.ui_state.playlist_state.playlists = vec![pl("Empty", &[])];
        app.ui_state.active_playlist = Some(0);
        app.ui_state.active_playlist_song = Some(3);

        app.prev_track();

        assert_eq!(app.ui_state.active_playlist_song, Some(3));
    }

    /// `next_track` wraps with a modulo, so a stale cursor cannot index out of
    /// bounds — unlike `prev_track`, which is why only that one panicked. It
    /// still has to land inside the playlist.
    #[test]
    fn next_track_with_a_stale_cursor_stays_inside_the_playlist() {
        let mut app = test_app();
        app.ui_state.playlist_state.playlists = vec![pl("Mix", &["/a.flac", "/b.flac"])];
        app.ui_state.active_playlist = Some(0);
        app.ui_state.active_playlist_song = Some(9);

        app.next_track();

        let landed = app.ui_state.active_playlist_song.unwrap();
        assert!(landed < 2, "must stay in range, landed on {landed}");
    }

    /// `on_track_ended` in sequential mode advances within the playlist; with a
    /// stale cursor it must not run past the end either.
    #[test]
    fn on_track_ended_with_a_stale_cursor_stays_inside_the_playlist() {
        let mut app = test_app();
        app.ui_state.playlist_state.playlists = vec![pl("Mix", &["/a.flac", "/b.flac"])];
        app.ui_state.active_playlist = Some(0);
        app.ui_state.active_playlist_song = Some(7);
        app.ui_state.repeat_mode = crate::ui::RepeatMode::Sequential;
        app.ui_state.player.tracks = vec![crate::ui::TrackDisplay {
            path: PathBuf::from("/a.flac"),
            title: "A".into(),
            artist: "X".into(),
            duration_secs: 1.0,
        }];
        app.ui_state.player.playing_index = Some(0);

        app.on_track_ended();

        assert!(!app.should_quit);
    }
}
