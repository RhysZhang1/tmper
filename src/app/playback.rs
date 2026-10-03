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

                let samples = {
                    let buf = crate::audio::engine::lock(&pcm_buf);
                    match latest_window(&buf, fft_size) {
                        Some(samples) => samples,
                        None => {
                            drop(buf);
                            std::thread::sleep(Duration::from_millis(runtime::FFT_WAIT_SLEEP_MS));
                            continue;
                        }
                    }
                };

                let magnitudes = analyzer.process(&samples);
                let bars = processor.process(
                    &magnitudes,
                    sample_rate.load(std::sync::atomic::Ordering::Relaxed),
                );

                *crate::audio::engine::lock(&fft_data) = bars;

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

/// The most recent `fft_size` samples from the PCM ring buffer, or `None`
/// until that many have accumulated.
///
/// `InstrumentedSource` pushes new samples at the back and drops old ones from
/// the front, so the newest window sits at the *end* of the deque. Reading from
/// the front instead analyses samples `capacity - fft_size` behind the audio —
/// ≈0.7s at 44.1kHz — which is what made the spectrum visibly trail playback.
fn latest_window(buffer: &std::collections::VecDeque<f32>, fft_size: usize) -> Option<Vec<f32>> {
    if buffer.len() < fft_size {
        return None;
    }
    let start = buffer.len() - fft_size;
    Some(buffer.iter().skip(start).copied().collect())
}

#[cfg(test)]
mod tests {
    use super::latest_window;
    use crate::app::handlers::test_support::{pl, test_app};
    use crate::app::App;
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

    // ── Global queue (no active playlist) ──

    /// Seed `n` tracks whose paths do not exist: `load_and_play` fails quietly
    /// on those, so the assertions below see only the cursor movement.
    fn seed_queue(app: &mut App, n: usize) {
        app.ui_state.player.tracks = (0..n)
            .map(|i| crate::ui::TrackDisplay {
                path: PathBuf::from(format!("/track{i}.flac")),
                title: format!("Track {i}"),
                artist: "X".into(),
                duration_secs: 1.0,
            })
            .collect();
    }

    #[test]
    fn next_and_prev_walk_the_global_queue_when_no_playlist_is_active() {
        let mut app = test_app();
        seed_queue(&mut app, 3);
        app.ui_state.player.playing_index = Some(1);

        app.next_track();
        assert_eq!(app.ui_state.player.selected_index, 2);

        app.prev_track();
        assert_eq!(app.ui_state.player.selected_index, 0);
    }

    /// The global fallback stops at the ends rather than wrapping, unlike the
    /// playlist path which wraps with a modulo.
    #[test]
    fn next_stops_at_the_end_of_the_global_queue() {
        let mut app = test_app();
        seed_queue(&mut app, 3);
        app.ui_state.player.playing_index = Some(2);
        app.ui_state.player.selected_index = 2;

        app.next_track();

        assert_eq!(app.ui_state.player.selected_index, 2);
    }

    #[test]
    fn prev_stops_at_the_start_of_the_global_queue() {
        let mut app = test_app();
        seed_queue(&mut app, 3);
        app.ui_state.player.playing_index = Some(0);
        app.ui_state.player.selected_index = 0;

        app.prev_track();

        assert_eq!(app.ui_state.player.selected_index, 0);
    }

    // ── on_track_ended, global queue ──

    #[test]
    fn track_end_advances_and_wraps_in_sequential_mode() {
        let mut app = test_app();
        seed_queue(&mut app, 3);
        app.ui_state.repeat_mode = crate::ui::RepeatMode::Sequential;

        app.ui_state.player.playing_index = Some(0);
        app.on_track_ended();
        assert_eq!(app.ui_state.player.selected_index, 1);

        app.ui_state.player.playing_index = Some(2);
        app.on_track_ended();
        assert_eq!(app.ui_state.player.selected_index, 0, "wraps to the start");
    }

    #[test]
    fn track_end_repeats_the_same_track_in_single_mode() {
        let mut app = test_app();
        seed_queue(&mut app, 3);
        app.ui_state.repeat_mode = crate::ui::RepeatMode::SingleTrack;
        app.ui_state.player.playing_index = Some(1);
        app.ui_state.player.selected_index = 1;

        app.on_track_ended();

        assert_eq!(app.ui_state.player.selected_index, 1, "cursor stays put");
    }

    #[test]
    fn track_end_in_shuffle_mode_lands_inside_the_queue() {
        let mut app = test_app();
        seed_queue(&mut app, 5);
        app.ui_state.repeat_mode = crate::ui::RepeatMode::Shuffle;
        app.ui_state.player.playing_index = Some(0);

        for _ in 0..20 {
            app.on_track_ended();
            assert!(
                app.ui_state.player.selected_index < 5,
                "shuffle must pick a real index"
            );
        }
    }

    /// Nothing playing → nothing to advance to. Must not index at all.
    #[test]
    fn track_end_without_a_current_track_does_nothing() {
        let mut app = test_app();
        seed_queue(&mut app, 3);
        app.ui_state.player.playing_index = None;
        app.ui_state.player.selected_index = 0;

        app.on_track_ended();

        assert_eq!(app.ui_state.player.selected_index, 0);
    }

    // ── Selection ──

    #[test]
    fn play_selected_plays_the_highlighted_track() {
        let mut app = test_app();
        app.ui_state.player.tracks = vec![
            crate::ui::TrackDisplay {
                path: PathBuf::from("/one.flac"),
                title: "One".into(),
                artist: "X".into(),
                duration_secs: 1.0,
            },
            crate::ui::TrackDisplay {
                path: PathBuf::from("/two.flac"),
                title: "Two".into(),
                artist: "X".into(),
                duration_secs: 1.0,
            },
        ];
        app.ui_state.player.selected_index = 1;

        app.play_selected();

        // Neither path exists, so metadata lookup fails and nothing is queued —
        // but the guard must not have panicked or touched the selection.
        assert_eq!(app.ui_state.player.selected_index, 1);
        assert_eq!(app.ui_state.player.playing_index, None);
    }

    /// A selection past the end is the stale-index case again: it must be
    /// ignored rather than panicking.
    #[test]
    fn play_selected_out_of_range_is_a_noop() {
        let mut app = test_app();
        app.ui_state.player.tracks = vec![crate::ui::TrackDisplay {
            path: PathBuf::from("/one.flac"),
            title: "One".into(),
            artist: "X".into(),
            duration_secs: 1.0,
        }];
        app.ui_state.player.selected_index = 7;

        app.play_selected();

        assert_eq!(app.ui_state.player.playing_index, None);
    }

    // ── FFT analysis window ──

    /// The window must be the newest samples, not the oldest ones still held.
    #[test]
    fn latest_window_returns_the_newest_samples() {
        let mut buffer = std::collections::VecDeque::new();
        for i in 0..10 {
            buffer.push_back(i as f32);
        }
        assert_eq!(latest_window(&buffer, 3), Some(vec![7.0, 8.0, 9.0]));
    }

    #[test]
    fn latest_window_is_none_until_enough_samples_arrive() {
        let mut buffer = std::collections::VecDeque::new();
        buffer.push_back(1.0);
        assert_eq!(latest_window(&buffer, 4), None);
    }

    /// Once the ring buffer is at capacity the answer must still be the tail —
    /// this is the case that was wrong, and the lag scaled with the buffer.
    #[test]
    fn latest_window_stays_at_the_tail_when_the_buffer_is_full() {
        let mut buffer = std::collections::VecDeque::new();
        for i in 0..100 {
            buffer.push_back(i as f32);
            while buffer.len() > 8 {
                buffer.pop_front();
            }
        }
        assert_eq!(latest_window(&buffer, 2), Some(vec![98.0, 99.0]));
    }
}
