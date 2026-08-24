use std::sync::Arc;

use rodio::{OutputStream, OutputStreamHandle, Sink};

use crate::error::AppResult;

pub struct AudioOutput {
    sink: Arc<Sink>,
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
            sink: Arc::new(sink),
            stream_handle,
            _stream: stream,
        })
    }

    /// Append a source to the sink. Only used by the sync `play_file`
    /// (test-only) path — production decoding uses `sink_arc().append()`
    /// from background tasks.
    #[cfg(test)]
    pub fn append_source(&self, source: impl rodio::Source<Item = f32> + Send + 'static) {
        self.sink.append(source);
    }

    pub fn pause(&self) {
        self.sink.pause();
    }

    pub fn play(&self) {
        self.sink.play();
    }

    /// Stop current sink and create a brand-new one.
    /// The old `Arc<Sink>` is dropped if no background task holds a reference;
    /// otherwise the background task sees `stop()` and its `sleep_until_end()`
    /// wakes up, allowing the task to exit cleanly.
    pub fn stop_and_replace(&mut self) {
        self.sink.stop();
        match Sink::try_new(&self.stream_handle) {
            Ok(new_sink) => self.sink = Arc::new(new_sink),
            Err(e) => {
                tracing::error!("Failed to create new sink (audio may be unavailable): {e}");
            }
        }
    }

    pub fn set_volume(&self, vol: f32) {
        self.sink.set_volume(vol);
    }

    /// Clone of the shared `Arc<Sink>` for use in background decode tasks.
    /// Both the main thread and background task reference the same sink,
    /// so `set_volume` and `stop` work correctly regardless of which path
    /// is feeding audio.
    pub fn sink_arc(&self) -> Arc<Sink> {
        Arc::clone(&self.sink)
    }
}
