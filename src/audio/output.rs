use rodio::{OutputStream, Sink};

use crate::error::AppResult;

#[allow(dead_code)]
pub struct AudioOutput {
    sink: Sink,
    _stream: OutputStream,
}

#[allow(dead_code)]
impl AudioOutput {
    pub fn new() -> AppResult<Self> {
        let (stream, stream_handle) = OutputStream::try_default().map_err(|e| {
            crate::error::AppError::Audio(format!("Failed to create audio output stream: {e}"))
        })?;

        let sink = Sink::try_new(&stream_handle).map_err(|e| {
            crate::error::AppError::Audio(format!("Failed to create audio sink: {e}"))
        })?;

        Ok(Self {
            sink,
            _stream: stream,
        })
    }

    pub fn play_raw(&self, samples: Vec<f32>, sample_rate: u32, channels: u8) {
        let source = rodio::buffer::SamplesBuffer::new(channels as u16, sample_rate, samples);
        self.sink.append(source);
    }

    pub fn append_source(&self, source: rodio::buffer::SamplesBuffer<f32>) {
        self.sink.append(source);
    }

    pub fn pause(&self) {
        self.sink.pause();
    }

    pub fn play(&self) {
        self.sink.play();
    }

    pub fn stop(&self) {
        self.sink.stop();
        self.sink.clear();
    }

    pub fn set_volume(&self, vol: f32) {
        self.sink.set_volume(vol);
    }

    pub fn is_paused(&self) -> bool {
        self.sink.is_paused()
    }

    pub fn empty(&self) -> bool {
        self.sink.empty()
    }

    pub fn len(&self) -> usize {
        self.sink.len()
    }
}
