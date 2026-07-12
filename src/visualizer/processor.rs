use std::collections::VecDeque;

pub struct SpectrumProcessor {
    num_bars: usize,
    smoothing: Vec<f32>,
    peak_window: VecDeque<f32>,
    alpha: f32,
    peak_alpha: f32,
}

impl SpectrumProcessor {
    pub fn new(num_bars: usize, alpha: f32) -> Self {
        Self {
            num_bars,
            smoothing: vec![0.0; num_bars],
            peak_window: VecDeque::with_capacity(64),
            alpha,
            peak_alpha: 0.1,
        }
    }

    pub fn process(&mut self, magnitudes: &[f32], sample_rate: u32) -> Vec<f32> {
        let nyquist = sample_rate as f32 / 2.0;
        let bin_width = nyquist / magnitudes.len() as f32;

        let start_freq = 20.0f32;
        let end_freq = 16000.0f32;

        // Log-scale bucketing
        let mut bars = vec![0.0f32; self.num_bars];
        #[allow(clippy::needless_range_loop)]
        for i in 0..self.num_bars {
            let ratio = i as f32 / self.num_bars as f32;
            let freq_low = start_freq * (end_freq / start_freq).powf(ratio);
            let freq_high =
                start_freq * (end_freq / start_freq).powf(ratio + 1.0 / self.num_bars as f32);

            let bin_low = (freq_low / bin_width).round() as usize;
            let bin_high = ((freq_high / bin_width).round() as usize).min(magnitudes.len());

            if bin_low < bin_high {
                let max_mag = magnitudes[bin_low..bin_high]
                    .iter()
                    .fold(0.0f32, |a, &b| a.max(b));
                bars[i] = max_mag;
            }
        }

        // EMA smoothing
        #[allow(clippy::needless_range_loop)]
        #[allow(clippy::needless_range_loop)]
        for i in 0..self.num_bars {
            self.smoothing[i] = self.alpha * bars[i] + (1.0 - self.alpha) * self.smoothing[i];
            bars[i] = self.smoothing[i];
        }

        // Dynamic range normalization
        let max_val = bars.iter().cloned().fold(0.0f32, f32::max);
        if max_val > 0.0 {
            // Update peak EMA
            let old_peak = self.peak_window.back().copied().unwrap_or(max_val);
            let new_peak = self.peak_alpha * max_val + (1.0 - self.peak_alpha) * old_peak;
            self.peak_window.push_back(new_peak);
            if self.peak_window.len() > 64 {
                self.peak_window.pop_front();
            }

            let peak = self.peak_window.back().copied().unwrap_or(max_val);
            let divisor = max_val.max(peak * 0.3);
            for bar in bars.iter_mut() {
                *bar = (*bar / divisor).clamp(0.0, 1.0);
            }
        }

        bars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_buckets_count() {
        let mut proc = SpectrumProcessor::new(32, 0.35);
        // Create fake magnitudes (all 1.0)
        let mags = vec![1.0f32; 1024];
        let bars = proc.process(&mags, 44100);
        assert_eq!(bars.len(), 32);
    }

    #[test]
    fn test_smoothing_converges() {
        let mut proc = SpectrumProcessor::new(8, 0.5);
        let mags = vec![1.0f32; 512];

        // Feed the same data multiple times
        for _ in 0..10 {
            proc.process(&mags, 44100);
        }
        let bars = proc.process(&mags, 44100);

        // All bars should be non-zero after feeding non-zero data
        for bar in &bars {
            assert!(
                *bar > 0.0,
                "Bar should be > 0 after repeated non-zero input"
            );
        }
    }
}
