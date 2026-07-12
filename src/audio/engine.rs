use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::audio::decoder::AudioDecoder;
use crate::audio::output::AudioOutput;
use crate::error::AppResult;

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
            let mut buf = self.buffer.lock().unwrap();
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

#[allow(dead_code)]
pub struct AudioEngine {
    output: AudioOutput,
    decoder: Option<AudioDecoder>,
    total_frames: Arc<Mutex<u64>>,
    sample_rate: u32,
    channels: u8,
    duration_secs: Arc<Mutex<Option<f64>>>,
    pub pcm_buffer: Arc<Mutex<VecDeque<f32>>>,
}

#[allow(dead_code)]
impl AudioEngine {
    pub fn new() -> AppResult<Self> {
        let output = AudioOutput::new()?;
        Ok(Self {
            output,
            decoder: None,
            total_frames: Arc::new(Mutex::new(0)),
            sample_rate: 44100,
            channels: 2,
            duration_secs: Arc::new(Mutex::new(None)),
            pcm_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(8192))),
        })
    }

    pub fn play_file(&mut self, path: &Path) -> AppResult<()> {
        self.stop();

        let mut decoder = AudioDecoder::open(path)?;
        self.sample_rate = decoder.sample_rate;
        self.channels = decoder.channels;
        *self.duration_secs.lock().unwrap() = Some(decoder.duration_secs());

        let channels = self.channels;
        let sample_rate = self.sample_rate;
        let total_frames = self.total_frames.clone();
        let pcm_buf = self.pcm_buffer.clone();

        {
            let mut tf = total_frames.lock().unwrap();
            *tf = 0;
        }
        // Clear ring buffer on new track
        pcm_buf.lock().unwrap().clear();

        while let Some(samples) = decoder.read_packet()? {
            let frame_count = samples.len() as u64 / channels as u64;

            // Wrap in InstrumentedSource so samples are copied to ring buffer
            let instrumented = InstrumentedSource::new(samples.into_iter(), pcm_buf.clone(), 8192);

            let source = rodio::buffer::SamplesBuffer::new(
                channels as u16,
                sample_rate,
                instrumented.collect::<Vec<f32>>(),
            );
            self.output.append_source(source);

            let mut tf = total_frames.lock().unwrap();
            *tf += frame_count;
        }

        self.decoder = Some(decoder);
        Ok(())
    }

    pub fn pause(&self) {
        self.output.pause();
    }

    pub fn resume(&self) {
        self.output.play();
    }

    pub fn stop(&mut self) {
        self.output.stop();
        self.decoder = None;
        let mut tf = self.total_frames.lock().unwrap();
        *tf = 0;
        *self.duration_secs.lock().unwrap() = None;
    }

    pub fn set_volume(&self, vol: f32) {
        self.output.set_volume(vol);
    }

    pub fn is_playing(&self) -> bool {
        !self.output.is_paused() && !self.output.empty()
    }

    pub fn position_secs(&self) -> f64 {
        let frames = *self.total_frames.lock().unwrap();
        frames as f64 / self.sample_rate as f64
    }

    pub fn duration_secs(&self) -> Option<f64> {
        *self.duration_secs.lock().unwrap()
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
    fn test_pcm_buffer_filled() {
        let mut engine = AudioEngine::new().expect("Failed to create engine");

        engine
            .play_file(Path::new("tests/fixtures/test.wav"))
            .expect("Failed to play file");

        let buf = engine.pcm_buffer.lock().unwrap();
        assert!(
            !buf.is_empty(),
            "PCM buffer should contain samples after play_file"
        );
    }
}
