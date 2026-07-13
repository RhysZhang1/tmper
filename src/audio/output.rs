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
        match Sink::try_new(&self.stream_handle) {
            Ok(new_sink) => self.sink = new_sink,
            Err(e) => {
                tracing::error!("Failed to create new sink (audio may be unavailable): {e}");
                // Keep the old (stopped) sink — won't produce audio in this state
                // but avoids a dangling handle. User will see the error in logs.
            }
        }
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
}
