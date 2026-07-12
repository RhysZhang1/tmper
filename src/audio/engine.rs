use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::audio::decoder::AudioDecoder;
use crate::audio::output::AudioOutput;
use crate::error::AppResult;

#[allow(dead_code)]
pub struct AudioEngine {
    output: AudioOutput,
    decoder: Option<AudioDecoder>,
    total_frames: Arc<Mutex<u64>>,
    sample_rate: u32,
    channels: u8,
    duration_secs: Arc<Mutex<Option<f64>>>,
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

        {
            let mut tf = total_frames.lock().unwrap();
            *tf = 0;
        }

        while let Some(samples) = decoder.read_packet()? {
            let frame_count = samples.len() as u64 / channels as u64;
            self.output.play_raw(samples, sample_rate, channels);
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

        // Verify position tracking works after feeding audio data
        let pos = engine.position_secs();
        assert!(pos > 0.0, "Position should be > 0 after decoding");

        // Pause and resume should not panic
        engine.pause();
        engine.resume();

        // Stop should clean up state
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
}
