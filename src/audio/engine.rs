use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
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
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// User-visible state of the current playback session.
///
/// This is also the wire spelling of the state ([`crate::ipc::proto`]): the
/// engine's state machine *is* the daemon's truth about playback, so the
/// snapshot carries it verbatim instead of a mirror that could drift.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PlaybackState {
    Stopped,
    Loading,
    Playing,
    Paused,
    Seeking,
    Finished,
    Failed(String),
}

impl PlaybackState {
    /// The audio clock is running: a track is loading, playing, or being
    /// seeked to. `Paused` is deliberately *not* active — it is a state the
    /// user chose, and it is what the transport toggle keys off.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Loading | Self::Playing | Self::Seeking)
    }
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
        // Recover from poisoning rather than dropping FFT data on the floor:
        // the ring buffer is a pure sample sink, so a poisoned guard still
        // leaves it in a usable state.
        let mut buffer = lock(&self.buffer);
        while buffer.len() >= self.capacity {
            buffer.pop_front();
        }
        buffer.push_back(sample);
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
        Self::with_output(AudioOutput::new()?)
    }

    #[cfg(test)]
    pub fn new_headless() -> Self {
        Self::with_output(AudioOutput::new_headless()).expect("headless output is infallible")
    }

    fn with_output(output: AudioOutput) -> AppResult<Self> {
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

    #[cfg(test)]
    fn begin_fake_session(&mut self, path: &Path) -> u64 {
        let (generation, ..) = self.begin_session();
        self.current_path = Some(path.to_path_buf());
        self.reset_position(0.0, false);
        self.state = PlaybackState::Loading;
        generation
    }

    #[cfg(test)]
    fn inject_decoder_event(&self, event: DecoderEvent) {
        self.event_tx.send(event).expect("test event receiver");
    }

    fn begin_session(&mut self) -> (u64, Arc<AtomicBool>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        self.cancel_decode();
        self.output.stop_and_replace();
        self.output.set_volume(self.current_volume);
        lock(&self.pcm_buffer).clear();
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
            let mut position = lock(&self.position);
            if position.pause_start.is_none() {
                position.pause_start = Some(Instant::now());
            }
            self.state = PlaybackState::Paused;
        }
    }

    pub fn resume(&mut self) {
        if self.state == PlaybackState::Paused {
            self.output.play();
            let mut position = lock(&self.position);
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
                    let base_offset = lock(&self.position).base_offset;
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
        *lock(&self.position) = PositionState {
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
        self.state.is_active()
    }

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
        let position = lock(&self.position);
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
    // Brings `channels` / `sample_rate` / … into scope for the
    // `InstrumentedSource` delegation assertions.
    use rodio::Source as _;

    #[test]
    #[ignore = "requires a real or virtual audio output device"]
    fn audio_output_engine_lifecycle() {
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
    #[ignore = "requires a real or virtual audio output device"]
    fn audio_output_stop_clears_position() {
        let mut engine = AudioEngine::new().expect("Failed to create engine");
        engine
            .play_file(Path::new("tests/fixtures/test.wav"))
            .expect("Failed to play file");
        engine.stop();
        assert_eq!(engine.position_secs(), 0.0);
        assert!(engine.duration_secs().is_none());
    }

    #[tokio::test]
    #[ignore = "requires a real or virtual audio output device"]
    async fn audio_output_stream_is_bounded_and_finishes() {
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
    #[ignore = "requires a real or virtual audio output device"]
    async fn audio_output_rapid_session_replacement_ignores_old_events() {
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
    #[ignore = "requires a real or virtual audio output device"]
    async fn audio_output_open_error_reaches_main_thread() {
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
    #[ignore = "requires a real or virtual audio output device"]
    async fn audio_output_seek_while_paused_stays_paused() {
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

    struct FakeDecoder {
        generation: u64,
    }

    impl FakeDecoder {
        fn ready(&self, engine: &AudioEngine, duration_secs: f64, sample_rate: u32) {
            engine.inject_decoder_event(DecoderEvent::Ready {
                generation: self.generation,
                duration_secs,
                sample_rate,
            });
        }

        fn finished(&self, engine: &AudioEngine) {
            engine.inject_decoder_event(DecoderEvent::Finished {
                generation: self.generation,
            });
        }

        fn failed(&self, engine: &AudioEngine, message: &str) {
            engine.inject_decoder_event(DecoderEvent::Failed {
                generation: self.generation,
                message: message.to_string(),
            });
        }
    }

    #[tokio::test]
    async fn playback_state_machine_switch_pause_seek_and_finish_without_device() {
        let mut engine = AudioEngine::new_headless();
        let first = FakeDecoder {
            generation: engine.begin_fake_session(Path::new("first.fake")),
        };
        first.ready(&engine, 120.0, 48_000);
        assert!(matches!(
            engine.drain_events().as_slice(),
            [PlaybackEvent::Ready { .. }]
        ));
        assert_eq!(engine.state(), &PlaybackState::Playing);

        engine.pause();
        assert_eq!(engine.state(), &PlaybackState::Paused);
        engine
            .seek_relative(10.0)
            .expect("fake seek starts a new session");
        assert_eq!(engine.state(), &PlaybackState::Paused);

        let second = FakeDecoder {
            generation: engine.begin_fake_session(Path::new("second.fake")),
        };
        first.finished(&engine);
        second.ready(&engine, 60.0, 44_100);
        let events = engine.drain_events();
        assert_eq!(events.len(), 1, "stale decoder event must be ignored");
        assert_eq!(engine.state(), &PlaybackState::Playing);
        second.finished(&engine);
        assert_eq!(engine.drain_events(), vec![PlaybackEvent::Finished]);
        assert_eq!(engine.state(), &PlaybackState::Finished);
    }

    // ── InstrumentedSource: the FFT ring buffer and its backpressure ledger ──

    /// Build a source the way the decoder does: count the packet in, then hand
    /// the source that same count to release as it plays.
    fn instrumented<I: Iterator<Item = f32>>(
        samples: I,
        count: usize,
        capacity: usize,
    ) -> (
        InstrumentedSource<I>,
        Arc<Mutex<VecDeque<f32>>>,
        Arc<AtomicUsize>,
    ) {
        let buffer = Arc::new(Mutex::new(VecDeque::new()));
        let queued = Arc::new(AtomicUsize::new(count));
        let source =
            InstrumentedSource::new(samples, buffer.clone(), capacity, queued.clone(), count);
        (source, buffer, queued)
    }

    #[test]
    fn instrumented_source_copies_every_sample_it_yields() {
        let (mut source, buffer, queued) = instrumented(vec![1.0f32, 2.0, 3.0].into_iter(), 3, 8);

        assert_eq!(source.next(), Some(1.0));
        assert_eq!(source.next(), Some(2.0));
        assert_eq!(source.next(), Some(3.0));
        assert_eq!(source.next(), None, "ends with the inner iterator");

        let copied: Vec<f32> = lock(&buffer).iter().copied().collect();
        assert_eq!(copied, vec![1.0, 2.0, 3.0]);
        assert_eq!(
            queued.load(Ordering::SeqCst),
            0,
            "every sample accounted for"
        );
    }

    #[test]
    fn instrumented_source_evicts_the_oldest_sample_at_capacity() {
        let (mut source, buffer, _) = instrumented((1..=6).map(|i| i as f32), 6, 4);
        while source.next().is_some() {}

        let copied: Vec<f32> = lock(&buffer).iter().copied().collect();
        assert_eq!(
            copied,
            vec![3.0, 4.0, 5.0, 6.0],
            "the buffer keeps the newest window"
        );
    }

    /// Backpressure waits on this counter. Every track change and seek drops a
    /// packet before it finishes playing, so a dropped source has to release
    /// the samples it never delivered — otherwise the count never returns to
    /// zero and decoding stalls for good.
    #[test]
    fn dropping_a_partly_played_packet_releases_its_unplayed_samples() {
        let (mut source, _, queued) = instrumented((0..10).map(|i| i as f32), 10, 32);

        assert_eq!(source.next(), Some(0.0));
        assert_eq!(source.next(), Some(1.0));
        assert_eq!(queued.load(Ordering::SeqCst), 8);

        drop(source);
        assert_eq!(
            queued.load(Ordering::SeqCst),
            0,
            "the 8 unplayed samples must be released"
        );
    }

    #[test]
    fn dropping_a_fully_played_packet_does_not_underflow_the_counter() {
        let (mut source, _, queued) = instrumented(vec![1.0f32, 2.0].into_iter(), 2, 32);
        while source.next().is_some() {}
        assert_eq!(queued.load(Ordering::SeqCst), 0);

        drop(source);
        assert_eq!(
            queued.load(Ordering::SeqCst),
            0,
            "released twice must saturate, not wrap"
        );
    }

    /// The wrapper must forward rodio's metadata untouched — comparing against
    /// an identical source rather than literals, because `SamplesBuffer`
    /// deliberately reports `None` for `current_frame_len`.
    #[test]
    fn instrumented_source_delegates_source_metadata() {
        let inner = rodio::buffer::SamplesBuffer::new(2, 44_100, vec![0.0f32; 16]);
        let reference = inner.clone();
        let buffer = Arc::new(Mutex::new(VecDeque::new()));
        let mut source = InstrumentedSource::new(
            inner,
            buffer.clone(),
            32,
            Arc::new(AtomicUsize::new(16)),
            16,
        );

        assert_eq!(source.channels(), reference.channels());
        assert_eq!(source.sample_rate(), reference.sample_rate());
        assert_eq!(source.current_frame_len(), reference.current_frame_len());
        assert_eq!(source.total_duration(), reference.total_duration());

        // And those forwarded values are the ones we constructed with.
        assert_eq!(source.channels(), 2);
        assert_eq!(source.sample_rate(), 44_100);

        // Pull a sample through the real `SamplesBuffer` path so this covers
        // the same `next` the decoder thread uses, not just the accessors.
        assert_eq!(source.next(), Some(0.0));
        assert_eq!(lock(&buffer).len(), 1);
    }

    /// The queue counters are diagnostics: `peak_queued_samples` is only read
    /// by the device-gated bound test, so without this it and the tracking in
    /// `play_file` could rot unnoticed.
    #[test]
    fn queue_diagnostics_start_empty() {
        let mut engine = AudioEngine::new_headless();
        assert_eq!(engine.queued_samples(), 0);
        assert_eq!(engine.peak_queued_samples(), 0);

        let _ = engine.begin_fake_session(Path::new("song.fake"));
        assert_eq!(
            engine.queued_samples(),
            0,
            "a new session must start with an empty queue"
        );
        assert_eq!(engine.peak_queued_samples(), 0);
    }

    // ── Session bookkeeping helpers ──

    #[test]
    fn atomic_saturating_sub_decrements_and_clamps_at_zero() {
        let value = AtomicUsize::new(10);
        atomic_saturating_sub(&value, 4);
        assert_eq!(value.load(Ordering::SeqCst), 6);

        atomic_saturating_sub(&value, 99);
        assert_eq!(value.load(Ordering::SeqCst), 0, "must not wrap around");
    }

    #[test]
    fn update_peak_only_moves_up() {
        let peak = AtomicUsize::new(5);
        update_peak(&peak, 3);
        assert_eq!(peak.load(Ordering::SeqCst), 5, "a lower value is ignored");
        update_peak(&peak, 9);
        assert_eq!(peak.load(Ordering::SeqCst), 9);
    }

    #[test]
    fn session_cancelled_reacts_to_both_signals() {
        let cancel = AtomicBool::new(false);
        let generation = AtomicU64::new(7);

        assert!(!session_cancelled(&cancel, &generation, 7), "live session");
        assert!(
            session_cancelled(&cancel, &generation, 6),
            "generation moved on — the session was replaced"
        );

        cancel.store(true, Ordering::SeqCst);
        assert!(
            session_cancelled(&cancel, &generation, 7),
            "explicit cancel"
        );
    }

    #[test]
    fn send_failure_carries_the_generation() {
        let (tx, rx) = mpsc::channel();
        send_failure(&tx, 42, "decode failed".into());

        match rx.try_recv() {
            Ok(DecoderEvent::Failed {
                generation,
                message,
            }) => {
                assert_eq!(generation, 42);
                assert_eq!(message, "decode failed");
            }
            _ => panic!("expected a Failed event"),
        }
    }

    #[test]
    fn send_failure_without_a_receiver_is_not_fatal() {
        let (tx, rx) = mpsc::channel();
        drop(rx);
        // The main loop may be gone; the decode thread must still wind down
        // cleanly rather than panicking on a closed channel.
        send_failure(&tx, 1, "nobody is listening".into());
    }

    // ── Failure and cancellation paths (headless) ──

    #[test]
    fn drain_events_turns_a_decoder_failure_into_a_failed_state() {
        let mut engine = AudioEngine::new_headless();
        let decoder = FakeDecoder {
            generation: engine.begin_fake_session(Path::new("broken.fake")),
        };

        decoder.failed(&engine, "unsupported codec");
        assert_eq!(
            engine.drain_events(),
            vec![PlaybackEvent::Failed("unsupported codec".into())]
        );
        assert_eq!(
            engine.state(),
            &PlaybackState::Failed("unsupported codec".into())
        );
        assert!(!engine.is_playing(), "a failed track is not playing");
    }

    #[test]
    fn drain_events_ignores_finished_and_failed_from_a_replaced_session() {
        let mut engine = AudioEngine::new_headless();
        let stale = FakeDecoder {
            generation: engine.begin_fake_session(Path::new("stale.fake")),
        };
        let current = FakeDecoder {
            generation: engine.begin_fake_session(Path::new("current.fake")),
        };

        stale.ready(&engine, 10.0, 44_100);
        stale.finished(&engine);
        stale.failed(&engine, "gone");
        assert!(
            engine.drain_events().is_empty(),
            "every event from the replaced session is dropped"
        );

        current.ready(&engine, 20.0, 48_000);
        assert_eq!(
            engine.drain_events().len(),
            1,
            "the live session still lands"
        );
    }

    // ── State machine guards ──

    #[test]
    fn pause_and_resume_do_nothing_from_a_stopped_engine() {
        let mut engine = AudioEngine::new_headless();

        engine.pause();
        assert_eq!(engine.state(), &PlaybackState::Stopped);
        engine.resume();
        assert_eq!(
            engine.state(),
            &PlaybackState::Stopped,
            "resuming a stopped engine must not claim to be playing"
        );
    }

    #[test]
    fn pause_is_idempotent_and_resume_restores_playing() {
        let mut engine = AudioEngine::new_headless();
        let decoder = FakeDecoder {
            generation: engine.begin_fake_session(Path::new("song.fake")),
        };
        decoder.ready(&engine, 180.0, 44_100);
        engine.drain_events();
        assert_eq!(engine.state(), &PlaybackState::Playing);

        engine.pause();
        assert_eq!(engine.state(), &PlaybackState::Paused);
        engine.pause(); // must not restart the paused window
        assert_eq!(engine.state(), &PlaybackState::Paused);

        engine.resume();
        assert_eq!(engine.state(), &PlaybackState::Playing);
        engine.resume(); // already playing — a no-op
        assert_eq!(engine.state(), &PlaybackState::Playing);
    }

    #[test]
    fn stop_clears_the_session_and_resets_the_queue_counter() {
        let mut engine = AudioEngine::new_headless();
        let decoder = FakeDecoder {
            generation: engine.begin_fake_session(Path::new("song.fake")),
        };
        decoder.ready(&engine, 180.0, 44_100);
        engine.drain_events();
        assert!(engine.duration_secs().is_some());

        engine.stop();

        assert_eq!(engine.state(), &PlaybackState::Stopped);
        assert_eq!(engine.position_secs(), 0.0);
        assert!(engine.duration_secs().is_none());
        assert_eq!(engine.queued_samples(), 0);
        assert!(!engine.is_playing());
    }
}
