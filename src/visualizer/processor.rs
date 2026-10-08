pub struct SpectrumProcessor {
    num_bars: usize,
    alpha: f32,
    /// Smoothed bar heights.
    smoothing: Vec<f32>,
    /// Slowly decaying reference level, used to avoid pumping every frame's
    /// loudest band back to full height.
    normalization_peak: f32,
    start_freq: f32,
    end_freq: f32,
}

impl SpectrumProcessor {
    pub fn new(num_bars: usize, alpha: f32) -> Self {
        Self {
            num_bars,
            alpha,
            smoothing: vec![0.0; num_bars],
            normalization_peak: 0.0,
            start_freq: 60.0,
            end_freq: 8000.0,
        }
    }

    pub fn process(&mut self, magnitudes: &[f32], sample_rate: u32) -> Vec<f32> {
        let nyquist = sample_rate as f32 / 2.0;
        let bin_width = nyquist / magnitudes.len() as f32;

        let start_freq = self.start_freq;
        let end_freq = self.end_freq;

        // Log-scale bucketing
        let mut bars = vec![0.0f32; self.num_bars];
        for (i, bar) in bars.iter_mut().enumerate() {
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
                *bar = max_mag;
            }
        }

        // Fast attack makes kicks and transients feel immediate; a slower
        // release keeps the columns fluid instead of jittery.
        let attack = self.alpha.sqrt();
        let release = self.alpha * 0.45;
        for (bar, smooth) in bars.iter_mut().zip(self.smoothing.iter_mut()) {
            // A square-root curve reveals quieter bands without changing
            // their ordering or requiring a fixed input gain.
            let raw = bar.sqrt();
            let response = if raw > *smooth { attack } else { release };
            *smooth = response * raw + (1.0 - response) * *smooth;
            *bar = *smooth;
        }

        // Dynamic normalization with a peak envelope. Normalizing against the
        // current maximum would pin one bar at 100% even while audio decays.
        let max_val = bars.iter().cloned().fold(0.0f32, f32::max);
        if max_val > 0.0 {
            self.normalization_peak =
                if self.normalization_peak == 0.0 || max_val >= self.normalization_peak {
                    max_val
                } else {
                    (self.normalization_peak * 0.96).max(max_val)
                };
            for bar in bars.iter_mut() {
                *bar = (*bar / self.normalization_peak).clamp(0.0, 1.0);
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

    #[test]
    fn silence_releases_instead_of_staying_normalized_to_full_height() {
        let mut proc = SpectrumProcessor::new(8, 0.35);
        let signal = vec![1.0f32; 512];
        let silence = vec![0.0f32; 512];

        let initial = proc.process(&signal, 44100);
        assert!(initial.iter().any(|&bar| bar > 0.9));

        let mut released = Vec::new();
        for _ in 0..20 {
            released = proc.process(&silence, 44100);
        }
        assert!(released.iter().all(|&bar| bar < 0.2));
    }
}
