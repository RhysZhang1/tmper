use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use crate::audio::decoder::AudioDecoder;
use crate::audio::output::AudioOutput;
use crate::constants::runtime;
use crate::error::AppResult;

/// User-visible state of the current playback session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Loading,
    Playing,
    Paused,
    Seeking,
    Finished,
    Failed(String),
}

/// Events produced by the active decoder and consumed by the app tick.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackEvent {
    Ready {
        duration_secs: f64,
        sample_rate: u32,
    },
    Finished,
    Failed(String),
}

enum DecoderEvent {
    Ready {
        generation: u64,
        duration_secs: f64,
        sample_rate: u32,
    },
    Finished {
        generation: u64,
    },
    Failed {
        generation: u64,
        message: String,
    },
}

/// Copies played samples to the FFT ring buffer and decrements the exact
/// number of samples still queued for this packet. Dropping a stopped source
/// also releases its unplayed count, so backpressure cannot remain stuck.
pub struct InstrumentedSource<I> {
    inner: I,
    buffer: Arc<Mutex<VecDeque<f32>>>,
    capacity: usize,
    queued_samples: Arc<AtomicUsize>,
    remaining: usize,
}

impl<I: Iterator<Item = f32>> InstrumentedSource<I> {
    pub fn new(
        inner: I,
        buffer: Arc<Mutex<VecDeque<f32>>>,
        capacity: usize,
        queued_samples: Arc<AtomicUsize>,
        sample_count: usize,
    ) -> Self {
        Self {
            inner,
            buffer,
            capacity,
            queued_samples,
            remaining: sample_count,
        }
    }
}

impl<I> Drop for InstrumentedSource<I> {
    fn drop(&mut self) {
        if self.remaining > 0 {
            atomic_saturating_sub(&self.queued_samples, self.remaining);
            self.remaining = 0;
        }
    }
}

impl<I: Iterator<Item = f32>> Iterator for InstrumentedSource<I> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.inner.next()?;
        if self.remaining > 0 {
            self.remaining -= 1;
            atomic_saturating_sub(&self.queued_samples, 1);
        }
        if let Ok(mut buffer) = self.buffer.lock() {
            while buffer.len() >= self.capacity {
                buffer.pop_front();
            }
            buffer.push_back(sample);
        }
        Some(sample)
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

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

struct PositionState {
    start: Option<Instant>,
    total_paused: Duration,
    pause_start: Option<Instant>,
    base_offset: f64,
}

pub struct AudioEngine {
    output: AudioOutput,
    #[cfg(test)]
    decoder: Option<AudioDecoder>,
    current_volume: f32,
    duration_secs: Option<f64>,
    pub pcm_buffer: Arc<Mutex<VecDeque<f32>>>,
    position: Mutex<PositionState>,
    current_path: Option<PathBuf>,
    decode_handle: Option<tokio::task::JoinHandle<()>>,
    cancel_flag: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    state: PlaybackState,
    event_tx: mpsc::Sender<DecoderEvent>,
    event_rx: mpsc::Receiver<DecoderEvent>,
    sample_rate: Arc<AtomicU32>,
    queued_samples: Arc<AtomicUsize>,
    peak_queued_samples: Arc<AtomicUsize>,
}

impl AudioEngine {
    pub fn new() -> AppResult<Self> {
        let output = AudioOutput::new()?;
        let (event_tx, event_rx) = mpsc::channel();
        Ok(Self {
            output,
            #[cfg(test)]
            decoder: None,
            current_volume: 0.8,
            duration_secs: None,
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
            generation: Arc::new(AtomicU64::new(0)),
            state: PlaybackState::Stopped,
            event_tx,
            event_rx,
            sample_rate: Arc::new(AtomicU32::new(runtime::DEFAULT_SAMPLE_RATE)),
            queued_samples: Arc::new(AtomicUsize::new(0)),
            peak_queued_samples: Arc::new(AtomicUsize::new(0)),
        })
    }

    fn begin_session(&mut self) -> (u64, Arc<AtomicBool>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        self.cancel_decode();
        self.output.stop_and_replace();
        self.output.set_volume(self.current_volume);
        self.pcm_buffer.lock().unwrap().clear();
        self.duration_secs = None;

        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.cancel_flag = Arc::new(AtomicBool::new(false));
        self.queued_samples = Arc::new(AtomicUsize::new(0));
        self.peak_queued_samples = Arc::new(AtomicUsize::new(0));
        (
            generation,
            self.cancel_flag.clone(),
            self.queued_samples.clone(),
            self.peak_queued_samples.clone(),
        )
    }

    fn cancel_decode(&mut self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
        if let Some(handle) = self.decode_handle.take() {
            handle.abort();
        }
    }

    #[cfg(test)]
    pub fn play_file(&mut self, path: &Path) -> AppResult<()> {
        self.begin_session();
        self.decoder = None;

        let mut decoder = AudioDecoder::open(path)?;
        self.duration_secs = Some(decoder.duration_secs());
        self.sample_rate
            .store(decoder.sample_rate, Ordering::Relaxed);
        let channels = decoder.channels;
        let sample_rate = decoder.sample_rate;
        let pcm_buffer = self.pcm_buffer.clone();

        while let Some(samples) = decoder.read_packet()? {
            let sample_count = samples.len();
            self.queued_samples
                .fetch_add(sample_count, Ordering::Relaxed);
            update_peak(&self.peak_queued_samples, self.queued_samples());
            let source = rodio::buffer::SamplesBuffer::new(channels as u16, sample_rate, samples);
            self.output.append_source(InstrumentedSource::new(
                source,
                pcm_buffer.clone(),
                runtime::PCM_BUFFER_CAPACITY,
                self.queued_samples.clone(),
                sample_count,
            ));
        }

        self.decoder = Some(decoder);
        self.current_path = Some(path.to_path_buf());
        self.reset_position(0.0, false);
        self.state = PlaybackState::Playing;
        Ok(())
    }

    pub fn play_file_async(&mut self, path: &Path) -> AppResult<()> {
        let path_buf = path.to_path_buf();
        let (generation, cancel, queued, peak) = self.begin_session();
        let sink = self.output.sink_arc();
        self.current_path = Some(path_buf.clone());
        self.reset_position(0.0, false);
        self.state = PlaybackState::Loading;
        self.spawn_decoder(path_buf, 0.0, generation, cancel, queued, peak, sink);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_decoder(
        &mut self,
        path: PathBuf,
        offset_secs: f64,
        generation: u64,
        cancel: Arc<AtomicBool>,
        queued: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        sink: Arc<rodio::Sink>,
    ) {
        let pcm_buffer = self.pcm_buffer.clone();
        let event_tx = self.event_tx.clone();
        let generation_counter = self.generation.clone();

        self.decode_handle = Some(tokio::task::spawn_blocking(move || {
            let mut decoder = match AudioDecoder::open(&path) {
                Ok(decoder) => decoder,
                Err(error) => {
                    send_failure(
                        &event_tx,
                        generation,
                        format!("Failed to open audio: {error}"),
                    );
                    return;
                }
            };
            if offset_secs > 0.0 {
                if let Err(error) = decoder.seek_to_secs(offset_secs) {
                    send_failure(&event_tx, generation, format!("Failed to seek: {error}"));
                    return;
                }
            }

            let sample_rate = decoder.sample_rate;
            let channels = decoder.channels.max(1);
            let max_queued =
                sample_rate as usize * channels as usize * runtime::AUDIO_PREBUFFER_SECS as usize;
            if event_tx
                .send(DecoderEvent::Ready {
                    generation,
                    duration_secs: decoder.duration_secs(),
                    sample_rate,
                })
                .is_err()
            {
                return;
            }

            loop {
                while queued.load(Ordering::Relaxed) >= max_queued {
                    if session_cancelled(&cancel, &generation_counter, generation) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(runtime::AUDIO_BACKPRESSURE_SLEEP_MS));
                }
                if session_cancelled(&cancel, &generation_counter, generation) {
                    return;
                }

                let samples = match decoder.read_packet() {
                    Ok(Some(samples)) => samples,
                    Ok(None) => break,
                    Err(error) => {
                        send_failure(&event_tx, generation, format!("Decode failed: {error}"));
                        return;
                    }
                };
                let sample_count = samples.len();
                queued.fetch_add(sample_count, Ordering::Relaxed);
                update_peak(&peak, queued.load(Ordering::Relaxed));
                let source =
                    rodio::buffer::SamplesBuffer::new(channels as u16, sample_rate, samples);
                sink.append(InstrumentedSource::new(
                    source,
                    pcm_buffer.clone(),
                    runtime::PCM_BUFFER_CAPACITY,
                    queued.clone(),
                    sample_count,
                ));
            }

            while !sink.empty() {
                if session_cancelled(&cancel, &generation_counter, generation) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(runtime::AUDIO_COMPLETION_POLL_MS));
            }
            if !session_cancelled(&cancel, &generation_counter, generation) {
                let _ = event_tx.send(DecoderEvent::Finished { generation });
            }
        }));
    }

    pub fn pause(&mut self) {
        if matches!(
            self.state,
            PlaybackState::Loading | PlaybackState::Playing | PlaybackState::Seeking
        ) {
            self.output.pause();
            let mut position = self.position.lock().unwrap();
            if position.pause_start.is_none() {
                position.pause_start = Some(Instant::now());
            }
            self.state = PlaybackState::Paused;
        }
    }

    pub fn resume(&mut self) {
        if self.state == PlaybackState::Paused {
            self.output.play();
            let mut position = self.position.lock().unwrap();
            if let Some(pause_start) = position.pause_start.take() {
                position.total_paused += pause_start.elapsed();
            }
            self.state = PlaybackState::Playing;
        }
    }

    pub fn stop(&mut self) {
        self.cancel_decode();
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.output.stop_and_replace();
        #[cfg(test)]
        {
            self.decoder = None;
        }
        self.current_path = None;
        self.duration_secs = None;
        self.queued_samples.store(0, Ordering::Relaxed);
        self.reset_position(0.0, false);
        self.state = PlaybackState::Stopped;
    }

    pub fn seek_relative(&mut self, delta_secs: f64) -> AppResult<()> {
        let path = match &self.current_path {
            Some(path) => path.clone(),
            None => return Ok(()),
        };
        let current = self.position_secs();
        let max_duration = self.duration_secs.unwrap_or(f64::MAX);
        let target = (current + delta_secs).clamp(0.0, max_duration * 0.999);
        if (target - current).abs() < f64::EPSILON {
            return Ok(());
        }

        let was_paused = self.state == PlaybackState::Paused;
        let (generation, cancel, queued, peak) = self.begin_session();
        if was_paused {
            self.output.pause();
        }
        let sink = self.output.sink_arc();
        self.current_path = Some(path.clone());
        self.reset_position(target, was_paused);
        self.state = if was_paused {
            PlaybackState::Paused
        } else {
            PlaybackState::Seeking
        };
        self.spawn_decoder(path, target, generation, cancel, queued, peak, sink);
        Ok(())
    }

    pub fn drain_events(&mut self) -> Vec<PlaybackEvent> {
        let current_generation = self.generation.load(Ordering::SeqCst);
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                DecoderEvent::Ready {
                    generation,
                    duration_secs,
                    sample_rate,
                } if generation == current_generation => {
                    self.duration_secs = Some(duration_secs);
                    self.sample_rate.store(sample_rate, Ordering::Relaxed);
                    let paused = self.state == PlaybackState::Paused;
                    let base_offset = self.position.lock().unwrap().base_offset;
                    self.reset_position(base_offset, paused);
                    if !paused {
                        self.state = PlaybackState::Playing;
                    }
                    events.push(PlaybackEvent::Ready {
                        duration_secs,
                        sample_rate,
                    });
                }
                DecoderEvent::Finished { generation } if generation == current_generation => {
                    self.state = PlaybackState::Finished;
                    events.push(PlaybackEvent::Finished);
                }
                DecoderEvent::Failed {
                    generation,
                    message,
                } if generation == current_generation => {
                    self.state = PlaybackState::Failed(message.clone());
                    events.push(PlaybackEvent::Failed(message));
                }
                _ => {}
            }
        }
        events
    }

    fn reset_position(&self, base_offset: f64, paused: bool) {
        *self.position.lock().unwrap() = PositionState {
            start: Some(Instant::now()),
            total_paused: Duration::ZERO,
            pause_start: paused.then(Instant::now),
            base_offset,
        };
    }

    fn elapsed_without_offset(position: &PositionState) -> f64 {
        match position.start {
            Some(start) => {
                let elapsed = start
                    .elapsed()
                    .checked_sub(position.total_paused)
                    .unwrap_or(Duration::ZERO);
                let adjusted = match position.pause_start {
                    Some(pause_start) => elapsed
                        .checked_sub(pause_start.elapsed())
                        .unwrap_or(Duration::ZERO),
                    None => elapsed,
                };
                adjusted.as_secs_f64()
            }
            None => 0.0,
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.current_volume = volume;
        self.output.set_volume(volume);
    }

    pub fn is_playing(&self) -> bool {
        matches!(
            self.state,
            PlaybackState::Loading | PlaybackState::Playing | PlaybackState::Seeking
        )
    }

    #[cfg(test)]
    pub fn state(&self) -> &PlaybackState {
        &self.state
    }

    pub fn position_secs(&self) -> f64 {
        if matches!(
            self.state,
            PlaybackState::Stopped | PlaybackState::Failed(_)
        ) {
            return 0.0;
        }
        let position = self.position.lock().unwrap();
        let value = Self::elapsed_without_offset(&position) + position.base_offset;
        self.duration_secs
            .map_or(value, |duration| value.min(duration))
    }

    pub fn duration_secs(&self) -> Option<f64> {
        self.duration_secs
    }

    pub fn sample_rate_handle(&self) -> Arc<AtomicU32> {
        self.sample_rate.clone()
    }

    #[cfg(test)]
    pub fn queued_samples(&self) -> usize {
        self.queued_samples.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    fn peak_queued_samples(&self) -> usize {
        self.peak_queued_samples.load(Ordering::Relaxed)
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(handle) = self.decode_handle.take() {
            handle.abort();
        }
    }
}

fn session_cancelled(cancel: &AtomicBool, generation: &AtomicU64, expected: u64) -> bool {
    cancel.load(Ordering::Relaxed) || generation.load(Ordering::SeqCst) != expected
}

fn send_failure(tx: &mpsc::Sender<DecoderEvent>, generation: u64, message: String) {
    tracing::error!("{message}");
    let _ = tx.send(DecoderEvent::Failed {
        generation,
        message,
    });
}

fn update_peak(peak: &AtomicUsize, value: usize) {
    let mut current = peak.load(Ordering::Relaxed);
    while value > current {
        match peak.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

fn atomic_saturating_sub(value: &AtomicUsize, amount: usize) {
    let mut current = value.load(Ordering::Relaxed);
    loop {
        let next = current.saturating_sub(amount);
        match value.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
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
        assert!(engine.position_secs() >= 0.0);
        engine.pause();
        assert_eq!(engine.state(), &PlaybackState::Paused);
        engine.resume();
        assert_eq!(engine.state(), &PlaybackState::Playing);
        engine.stop();
        assert_eq!(engine.state(), &PlaybackState::Stopped);
        assert!(engine.duration_secs().is_none());
    }

    #[test]
    fn test_stop_clears_position() {
        let mut engine = AudioEngine::new().expect("Failed to create engine");
        engine
            .play_file(Path::new("tests/fixtures/test.wav"))
            .expect("Failed to play file");
        engine.stop();
        assert_eq!(engine.position_secs(), 0.0);
        assert!(engine.duration_secs().is_none());
    }

    #[tokio::test]
    async fn async_stream_is_bounded_and_finishes() {
        let mut engine = AudioEngine::new().expect("Failed to create engine");
        engine
            .play_file_async(Path::new("tests/fixtures/test.wav"))
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(4);
        let mut finished = false;
        while Instant::now() < deadline {
            for event in engine.drain_events() {
                if event == PlaybackEvent::Finished {
                    finished = true;
                }
            }
            if finished {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(finished, "stream should emit an explicit completion event");
        let bound =
            runtime::DEFAULT_SAMPLE_RATE as usize * 2 * runtime::AUDIO_PREBUFFER_SECS as usize
                + 8192;
        assert!(engine.peak_queued_samples() <= bound);
    }

    #[tokio::test]
    async fn rapid_session_replacement_ignores_old_events() {
        let mut engine = AudioEngine::new().expect("Failed to create engine");
        engine
            .play_file_async(Path::new("tests/fixtures/test.wav"))
            .unwrap();
        engine.seek_relative(0.5).unwrap();
        engine
            .play_file_async(Path::new("tests/fixtures/test.flac"))
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut ready_rate = None;
        while Instant::now() < deadline && ready_rate.is_none() {
            for event in engine.drain_events() {
                if let PlaybackEvent::Ready { sample_rate, .. } = event {
                    ready_rate = Some(sample_rate);
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(ready_rate, Some(44100));
        assert!(!matches!(engine.state(), PlaybackState::Failed(_)));
        engine.stop();
    }

    #[tokio::test]
    async fn async_open_error_reaches_main_thread() {
        let mut engine = AudioEngine::new().expect("Failed to create engine");
        engine
            .play_file_async(Path::new("tests/fixtures/missing.wav"))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if engine
                .drain_events()
                .iter()
                .any(|event| matches!(event, PlaybackEvent::Failed(_)))
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("decoder failure should reach the main thread");
    }

    #[tokio::test]
    async fn seek_while_paused_stays_paused() {
        let mut engine = AudioEngine::new().expect("Failed to create engine");
        engine
            .play_file_async(Path::new("tests/fixtures/test.wav"))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if engine
                .drain_events()
                .iter()
                .any(|event| matches!(event, PlaybackEvent::Ready { .. }))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        engine.pause();
        engine.seek_relative(0.5).unwrap();
        assert_eq!(engine.state(), &PlaybackState::Paused);
        tokio::time::sleep(Duration::from_millis(20)).await;
        engine.drain_events();
        assert_eq!(engine.state(), &PlaybackState::Paused);
        let first = engine.position_secs();
        tokio::time::sleep(Duration::from_millis(30)).await;
        let second = engine.position_secs();
        assert!(
            (second - first).abs() < 0.01,
            "paused position must not advance"
        );
    }
}
