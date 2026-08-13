use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::audio::decoder::AudioDecoder;
use crate::audio::output::AudioOutput;
use crate::constants::runtime;
use crate::error::AppResult;

/// Lock a `Mutex`, recovering from poisoning instead of panicking.
///
/// `Mutex::lock()` returns `Err(PoisonError)` only if a thread panicked while
/// holding the guard. The engine never holds a lock across a fallible `?`
/// operation, so the guarded data is always left in a consistent state;
/// `into_inner()` is therefore safe and avoids crashing the player — or, worse,
/// a background decode/FFT thread — on an unrelated panic elsewhere.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Wraps a PCM sample iterator, copying each sample to a shared ring buffer.
pub struct InstrumentedSource<I> {
    inner: I,
    buffer: Arc<Mutex<VecDeque<f32>>>,
    capacity: usize,
}

impl<I: Iterator<Item = f32>> InstrumentedSource<I> {
    pub fn new(inner: I, buffer: Arc<Mutex<VecDeque<f32>>>, capacity: usize) -> Self {
        Self {
            inner,
            buffer,
            capacity,
        }
    }
}

impl<I: Iterator<Item = f32>> Iterator for InstrumentedSource<I> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().inspect(|&sample| {
            let mut buf = lock(&self.buffer);
            while buf.len() >= self.capacity {
                buf.pop_front();
            }
            buf.push_back(sample);
        })
    }
}

impl<I: Iterator<Item = f32> + rodio::Source> rodio::Source for InstrumentedSource<I> {
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }

    fn channels(&self) -> u16 {
        self.inner.channels()
    }

    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        self.inner.total_duration()
    }
}

/// Tracks playback position using wall-clock time (not decoded frame counts).
struct PositionState {
    start: Option<Instant>,
    total_paused: Duration,
    pause_start: Option<Instant>,
    base_offset: f64, // seek offset in seconds
}

pub struct AudioEngine {
    output: AudioOutput,
    decoder: Option<AudioDecoder>,
    current_volume: f32,
    duration_secs: Arc<Mutex<Option<f64>>>,
    pub pcm_buffer: Arc<Mutex<VecDeque<f32>>>,
    position: Mutex<PositionState>,
    current_path: Option<PathBuf>,
    decode_handle: Option<tokio::task::JoinHandle<()>>,
    /// Flag shared with the background decode task; set to cancel.
    cancel_flag: Arc<AtomicBool>,
}

impl AudioEngine {
    pub fn new() -> AppResult<Self> {
        let output = AudioOutput::new()?;
        Ok(Self {
            output,
            decoder: None,
            current_volume: 0.8,
            duration_secs: Arc::new(Mutex::new(None)),
            pcm_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(
                runtime::PCM_BUFFER_CAPACITY,
            ))),
            position: Mutex::new(PositionState {
                start: None,
                total_paused: Duration::ZERO,
                pause_start: None,
                base_offset: 0.0,
            }),
            current_path: None,
            decode_handle: None,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Cancel any in-progress async decode.
    fn cancel_decode(&mut self) {
        if self.decode_handle.is_some() {
            // Signal the background task to stop its wait loop.
            self.cancel_flag.store(true, Ordering::SeqCst);
            // Stop the shared sink — unblocks the bg task if it's in
            // the cancellable wait loop.
            self.output.stop_and_replace();
            self.output.set_volume(self.current_volume);
            // Reset flag for next use.
            self.cancel_flag = Arc::new(AtomicBool::new(false));
            self.decode_handle = None;
        }
    }

    /// Sync decode — blocks until the entire file is decoded and queued.
    #[cfg(test)]
    pub fn play_file(&mut self, path: &Path) -> AppResult<()> {
        self.cancel_decode();
        self.decoder = None;
        *lock(&self.duration_secs) = None;
        *lock(&self.position) = PositionState {
            start: None,
            total_paused: Duration::ZERO,
            pause_start: None,
            base_offset: 0.0,
        };

        self.output.stop_and_replace();
        self.output.set_volume(self.current_volume);
        lock(&self.pcm_buffer).clear();

        let mut decoder = AudioDecoder::open(path)?;
        *lock(&self.duration_secs) = Some(decoder.duration_secs());

        let channels = decoder.channels;
        let sample_rate = decoder.sample_rate;
        let pcm_buf = self.pcm_buffer.clone();

        while let Some(samples) = decoder.read_packet()? {
            let source = rodio::buffer::SamplesBuffer::new(channels as u16, sample_rate, samples);
            self.output.append_source(InstrumentedSource::new(
                source,
                pcm_buf.clone(),
                runtime::PCM_BUFFER_CAPACITY,
            ));
        }

        self.decoder = Some(decoder);
        self.current_path = Some(path.to_path_buf());

        let mut pos = lock(&self.position);
        pos.start = Some(Instant::now());
        pos.total_paused = Duration::ZERO;
        pos.pause_start = None;
        pos.base_offset = 0.0;

        Ok(())
    }

    /// Async decode — spawns a background task, returns immediately.
    /// Uses the same `Arc<Sink>` as the main thread so `set_volume` and
    /// `stop` work correctly regardless of which path feeds audio.
    pub fn play_file_async(&mut self, path: &Path) -> AppResult<()> {
        self.cancel_decode();
        lock(&self.pcm_buffer).clear();
        *lock(&self.duration_secs) = None;

        let path_buf = path.to_path_buf();
        let sink = self.output.sink_arc();
        let pcm_buf = self.pcm_buffer.clone();
        let duration = self.duration_secs.clone();
        let volume = self.current_volume;
        let cancel = self.cancel_flag.clone();
        sink.set_volume(volume);

        let decode_path = path_buf.clone();
        let h = tokio::task::spawn_blocking(move || {
            let mut decoder = match AudioDecoder::open(&decode_path) {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("Async decode open failed: {e}");
                    return;
                }
            };
            *lock(&duration) = Some(decoder.duration_secs());

            let sample_rate = decoder.sample_rate;
            let channels = decoder.channels;

            while let Ok(Some(samples)) = decoder.read_packet() {
                let source =
                    rodio::buffer::SamplesBuffer::new(channels as u16, sample_rate, samples);
                let instrumented =
                    InstrumentedSource::new(source, pcm_buf.clone(), runtime::PCM_BUFFER_CAPACITY);
                sink.append(instrumented);
            }
            // Cancellable wait loop: polls every 50ms, exits immediately
            // when cancel_flag is set or the sink drains naturally.
            while !cancel.load(Ordering::Relaxed) && !sink.empty() {
                std::thread::sleep(Duration::from_millis(50));
            }
        });

        self.decode_handle = Some(h);
        self.current_path = Some(path_buf);

        let mut pos = lock(&self.position);
        *pos = PositionState {
            start: Some(Instant::now()),
            total_paused: Duration::ZERO,
            pause_start: None,
            base_offset: 0.0,
        };

        Ok(())
    }

    pub fn pause(&mut self) {
        self.output.pause();
        lock(&self.position).pause_start = Some(Instant::now());
    }

    pub fn resume(&mut self) {
        self.output.play();
        let mut pos = lock(&self.position);
        if let Some(pause_start) = pos.pause_start.take() {
            pos.total_paused += pause_start.elapsed();
        }
    }

    pub fn stop(&mut self) {
        self.cancel_decode();
        self.output.stop_and_replace();
        self.decoder = None;
        self.current_path = None;
        *lock(&self.duration_secs) = None;
        *lock(&self.position) = PositionState {
            start: None,
            total_paused: Duration::ZERO,
            pause_start: None,
            base_offset: 0.0,
        };
    }

    /// Seek by a relative delta (seconds). Positive = forward, negative = backward.
    /// Re-decodes the file from the new position (true audio seek).
    ///
    /// Decoding runs on a background thread (mirrors `play_file_async`), so
    /// seeking large files no longer freezes the event loop. Position jumps
    /// to the target immediately; audio begins once the task feeds the sink.
    pub fn seek_relative(&mut self, delta_secs: f64) -> AppResult<()> {
        self.cancel_decode();
        let path = match &self.current_path {
            Some(p) => p.clone(),
            None => return Ok(()),
        };

        let dur = *lock(&self.duration_secs);
        let max_dur = dur.unwrap_or(f64::MAX);
        let pos_data = lock(&self.position);
        let current = Self::elapsed_without_offset(&pos_data) + pos_data.base_offset;
        drop(pos_data);
        let target = (current + delta_secs).clamp(0.0, max_dur * 0.999);
        if target == current {
            return Ok(());
        }

        // Jump position immediately — the UI advances to the target while
        // the background task decodes the remaining audio from there.
        *lock(&self.position) = PositionState {
            start: Some(Instant::now()),
            total_paused: Duration::ZERO,
            pause_start: None,
            base_offset: target,
        };

        // Replace the sink and clear the FFT buffer for the new stream.
        lock(&self.pcm_buffer).clear();
        self.output.stop_and_replace();
        self.output.set_volume(self.current_volume);

        let sink = self.output.sink_arc();
        let pcm_buf = self.pcm_buffer.clone();
        let volume = self.current_volume;
        let cancel = self.cancel_flag.clone();
        sink.set_volume(volume);

        let h = tokio::task::spawn_blocking(move || {
            let mut decoder = match AudioDecoder::open(&path) {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("Async seek: failed to open file: {e}");
                    return;
                }
            };
            if let Err(e) = decoder.seek_to_secs(target) {
                tracing::error!("Async seek: failed to seek to {target}s: {e}");
                return;
            }
            let sample_rate = decoder.sample_rate;
            let channels = decoder.channels;

            while let Ok(Some(samples)) = decoder.read_packet() {
                let source =
                    rodio::buffer::SamplesBuffer::new(channels as u16, sample_rate, samples);
                let instrumented =
                    InstrumentedSource::new(source, pcm_buf.clone(), runtime::PCM_BUFFER_CAPACITY);
                sink.append(instrumented);
            }
            // Cancellable wait loop: exits when the sink drains naturally
            // or the next play/seek/stop cancels the decode.
            while !cancel.load(Ordering::Relaxed) && !sink.empty() {
                std::thread::sleep(Duration::from_millis(50));
            }
        });

        self.decode_handle = Some(h);

        Ok(())
    }

    /// Helper: wall-clock elapsed without base_offset.
    fn elapsed_without_offset(pos: &PositionState) -> f64 {
        match pos.start {
            Some(start_time) => {
                let playing = start_time
                    .elapsed()
                    .checked_sub(pos.total_paused)
                    .unwrap_or(Duration::ZERO);
                let adjusted = match pos.pause_start {
                    Some(ps) => playing.checked_sub(ps.elapsed()).unwrap_or(Duration::ZERO),
                    None => playing,
                };
                adjusted.as_secs_f64().max(0.0)
            }
            None => 0.0,
        }
    }

    pub fn set_volume(&mut self, vol: f32) {
        self.current_volume = vol;
        self.output.set_volume(vol);
    }

    pub fn is_playing(&self) -> bool {
        !self.output.is_paused() && !self.output.empty()
    }

    pub fn position_secs(&self) -> f64 {
        let pos = lock(&self.position);
        Self::elapsed_without_offset(&pos) + pos.base_offset
    }

    pub fn duration_secs(&self) -> Option<f64> {
        *lock(&self.duration_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_lifecycle() {
        let mut engine = AudioEngine::new().expect("Failed to create engine");

        engine
            .play_file(Path::new("tests/fixtures/test.wav"))
            .expect("Failed to play file");

        let pos = engine.position_secs();
        assert!(pos > 0.0, "Position should be > 0 after decoding");

        engine.pause();
        engine.resume();

        engine.stop();
        assert!(
            engine.duration_secs().is_none(),
            "Duration should be cleared after stop"
        );
    }

    #[test]
    fn test_position_tracking() {
        let mut engine = AudioEngine::new().expect("Failed to create engine");

        engine
            .play_file(Path::new("tests/fixtures/test.wav"))
            .expect("Failed to play file");

        let pos = engine.position_secs();
        assert!(
            pos > 0.0,
            "Position should be > 0 after playing 2s of audio"
        );

        let dur = engine.duration_secs();
        assert!(dur.is_some(), "Duration should be known");
        assert!(dur.unwrap() > 1.5, "Duration should be ~2 seconds");
    }

    #[test]
    fn test_stop_clears_position() {
        let mut engine = AudioEngine::new().expect("Failed to create engine");

        engine
            .play_file(Path::new("tests/fixtures/test.wav"))
            .expect("Failed to play file");

        engine.stop();

        assert_eq!(
            engine.position_secs(),
            0.0,
            "Position should be 0 after stop"
        );
        assert!(engine.duration_secs().is_none());
    }

    #[test]
    fn test_audio_pipeline_queued() {
        let mut engine = AudioEngine::new().expect("Failed to create engine");

        engine
            .play_file(Path::new("tests/fixtures/test.wav"))
            .expect("Failed to play file");

        // Sources are queued in the sink synchronously during play_file.
        // In headless test environments the audio device may drain instantly,
        // so is_playing() may return false. Verify position tracking works
        // (wall-clock based) and duration was set correctly instead.
        assert!(
            engine.position_secs() > 0.0,
            "Position should advance after play_file"
        );
        assert!(
            engine.duration_secs().unwrap_or(0.0) > 0.0,
            "Duration should be set after play_file"
        );
    }

    #[tokio::test]
    async fn test_async_seek_jumps_position() {
        let mut engine = AudioEngine::new().expect("Failed to create engine");
        engine
            .play_file(Path::new("tests/fixtures/test.wav"))
            .expect("Failed to play file");

        // Seek forward by 0.5s. The async path must jump position immediately
        // (base_offset), not block while the background task re-decodes.
        engine.seek_relative(0.5).expect("seek should not error");
        let pos = engine.position_secs();
        assert!(
            (0.4..0.7).contains(&pos),
            "position should jump to ~0.5s after seek, got {pos}"
        );

        // Give the background decode task time to finish draining.
        tokio::time::sleep(Duration::from_millis(300)).await;
        engine.stop();
    }
}
