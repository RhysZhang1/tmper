use rodio::{OutputStream, OutputStreamHandle, Sink};

use crate::error::AppResult;

pub struct AudioOutput {
    sink: Sink,
    stream_handle: OutputStreamHandle,
    _stream: OutputStream,
}

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
            stream_handle,
            _stream: stream,
        })
    }

    pub fn append_source(&self, source: impl rodio::Source<Item = f32> + Send + 'static) {
        self.sink.append(source);
    }

    pub fn pause(&self) {
        self.sink.pause();
    }

    pub fn play(&self) {
        self.sink.play();
    }

    /// Stop current playback and create a brand-new sink.
    /// This avoids rodio's permanent-detach-on-stop() issue.
    pub fn stop_and_replace(&mut self) {
        self.sink.stop();
        self.sink = Sink::try_new(&self.stream_handle).unwrap_or_else(|_| {
            // If creation fails, return a detached sink (best-effort)
            let (_, handle) = rodio::OutputStream::try_default().expect("audio device");
            Sink::try_new(&handle).expect("new sink")
        });
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

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.sink.len()
    }
}
