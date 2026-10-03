use std::sync::Arc;

use rodio::{OutputStream, OutputStreamHandle, Sink};

use crate::error::AppResult;

pub struct AudioOutput {
    sink: Arc<Sink>,
    stream_handle: Option<OutputStreamHandle>,
    _stream: Option<OutputStream>,
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
            stream_handle: Some(stream_handle),
            _stream: Some(stream),
        })
    }

    /// Device-free output used by unit tests. The queue receiver is dropped,
    /// so app and state-machine tests never open ALSA/PulseAudio.
    #[cfg(test)]
    pub fn new_headless() -> Self {
        let (sink, _) = Sink::new_idle();
        Self {
            sink: Arc::new(sink),
            stream_handle: None,
            _stream: None,
        }
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

    /// Retire the current sink and install a fresh one for the next session.
    ///
    /// The retired sink is deliberately *not* `stop()`ed. rodio's `stop` only
    /// sets the `controls.stopped` flag (sink.rs:312), and the *next* `append`
    /// on a flagged sink whose `sound_count > 0` calls `sleep_until_end`
    /// (sink.rs:111-114) — a blocking `recv` on a "sound ended" signal that
    /// nothing ends on a sink nothing polls, which is what the headless sink is
    /// and what a stalled device becomes. A decode thread that is between two
    /// appends when the flag lands parks in rodio for good, and the runtime that
    /// owns it can no longer shut down either (`Runtime::drop` waits for running
    /// blocking tasks) — that is how a rare interleaving used to hang the whole
    /// test suite.
    ///
    /// Dropping the sink is rodio's documented way to stop sound (`detach` is
    /// the documented way *not* to), and it silences the retired sink by the
    /// same mechanism: `Drop` sets `stopped` and clears `keep_alive_if_empty`
    /// (sink.rs:356-365), so the current source is stopped by its 5 ms
    /// `periodic_access` and an emptied queue ends instead of playing silence
    /// (queue.rs:235-242). What differs is only *when* the flag lands — not on
    /// this call, but when the decode thread holding the sink's `Arc` next
    /// checks for cancellation, within one `AUDIO_BACKPRESSURE_SLEEP_MS` poll
    /// (10 ms, 20 ms while completing). Dropping the last `Arc` is what runs
    /// `Drop`, so it cannot land under a live append.
    pub fn stop_and_replace(&mut self) {
        let new_sink = match &self.stream_handle {
            Some(handle) => Sink::try_new(handle),
            None => Ok(Sink::new_idle().0),
        };
        match new_sink {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn silence() -> rodio::buffer::SamplesBuffer<f32> {
        rodio::buffer::SamplesBuffer::new(2, 44_100, vec![0.0f32; 64])
    }

    /// A retired sink must still take appends — the next one belongs to a decode
    /// thread that has not noticed the cancellation yet.
    ///
    /// `Sink::stop` only sets a flag; the next `append` on that sink then waits
    /// inside `sleep_until_end` for the queued sound to end, which on a sink
    /// nothing polls never happens. A decode thread that reaches that append
    /// parks in rodio for good, and the runtime that owns it cannot shut down
    /// while a blocking task runs, so the whole suite hangs with it. Retiring
    /// the sink by dropping it keeps this append non-blocking.
    #[test]
    fn a_retired_sink_still_accepts_appends() {
        let mut output = AudioOutput::new_headless();
        let session_sink = output.sink_arc();
        // rodio arms its end-of-sound signal on every append, and the hazard
        // needs one already queued.
        session_sink.append(silence());

        output.stop_and_replace();

        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            session_sink.append(silence());
            let _ = done_tx.send(());
        });
        assert!(
            done_rx.recv_timeout(Duration::from_secs(2)).is_ok(),
            "appending to the retired sink blocked inside rodio's sleep_until_end"
        );
    }
}
