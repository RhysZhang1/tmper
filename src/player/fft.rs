//! The spectrum thread.
//!
//! It reads the raw PCM the audio source is pushing (the same ring buffer the
//! decoder fills, shared by `Arc`) and writes one frame of bars at a time into
//! another shared slot. Both ends are plain `Arc<Mutex<_>>` **within the
//! daemon process**: the client gets its copy over the socket, not through
//! shared memory.
//!
//! The thread runs only while a client is subscribed to the visualizer —
//! nobody looking at bars should not cost a core.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::audio::engine::lock;
use crate::constants::runtime;
use crate::visualizer::fft::FftAnalyzer;
use crate::visualizer::processor::SpectrumProcessor;

/// Start the analyser, replacing any thread already running.
pub fn spawn(
    pcm_buf: Arc<Mutex<VecDeque<f32>>>,
    sample_rate: Arc<AtomicU32>,
    bars_out: Arc<Mutex<Vec<f32>>>,
    num_bars: usize,
    smoothing: f32,
    cancel_rx: tokio::sync::watch::Receiver<()>,
) {
    tokio::task::spawn_blocking(move || {
        let fft_size = runtime::FFT_SIZE;
        let mut analyzer = FftAnalyzer::new(fft_size);
        let mut processor = SpectrumProcessor::new(num_bars, smoothing);

        loop {
            // Check cancellation (non-blocking)
            if cancel_rx.has_changed().is_err() {
                break; // sender dropped
            }

            let samples = {
                let buf = lock(&pcm_buf);
                match latest_window(&buf, fft_size) {
                    Some(samples) => samples,
                    None => {
                        drop(buf);
                        std::thread::sleep(Duration::from_millis(runtime::FFT_WAIT_SLEEP_MS));
                        continue;
                    }
                }
            };

            let magnitudes = analyzer.process(&samples);
            let bars = processor.process(&magnitudes, sample_rate.load(Ordering::Relaxed));

            // A setting change may have replaced this worker during analysis.
            // Prevent the cancelled worker from publishing its old bar count.
            if cancel_rx.has_changed().is_err() {
                break;
            }
            *lock(&bars_out) = bars;

            std::thread::sleep(Duration::from_millis(runtime::FFT_LOOP_SLEEP_MS));
        }
    });
}

/// The most recent `fft_size` samples from the PCM ring buffer, or `None`
/// until that many have accumulated.
///
/// `InstrumentedSource` pushes new samples at the back and drops old ones from
/// the front, so the newest window sits at the *end* of the deque. Reading from
/// the front instead analyses samples `capacity - fft_size` behind the audio —
/// ≈0.7s at 44.1kHz — which is what made the spectrum visibly trail playback.
fn latest_window(buffer: &VecDeque<f32>, fft_size: usize) -> Option<Vec<f32>> {
    if buffer.len() < fft_size {
        return None;
    }
    let start = buffer.len() - fft_size;
    Some(buffer.iter().skip(start).copied().collect())
}

#[cfg(test)]
mod tests {
    use super::latest_window;

    /// The window must be the newest samples, not the oldest ones still held.
    #[test]
    fn latest_window_returns_the_newest_samples() {
        let mut buffer = std::collections::VecDeque::new();
        for i in 0..10 {
            buffer.push_back(i as f32);
        }
        assert_eq!(latest_window(&buffer, 3), Some(vec![7.0, 8.0, 9.0]));
    }

    #[test]
    fn latest_window_is_none_until_enough_samples_arrive() {
        let mut buffer = std::collections::VecDeque::new();
        buffer.push_back(1.0);
        assert_eq!(latest_window(&buffer, 4), None);
    }

    /// Once the ring buffer is at capacity the answer must still be the tail —
    /// this is the case that was wrong, and the lag scaled with the buffer.
    #[test]
    fn latest_window_stays_at_the_tail_when_the_buffer_is_full() {
        let mut buffer = std::collections::VecDeque::new();
        for i in 0..100 {
            buffer.push_back(i as f32);
            while buffer.len() > 8 {
                buffer.pop_front();
            }
        }
        assert_eq!(latest_window(&buffer, 2), Some(vec![98.0, 99.0]));
    }
}
