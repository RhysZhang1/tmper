use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use std::f32::consts::PI;
use std::sync::Arc;

pub struct FftAnalyzer {
    fft: Arc<dyn rustfft::Fft<f32>>,
    scratch: Vec<Complex<f32>>,
    window: Vec<f32>,
    pub size: usize,
}

impl FftAnalyzer {
    pub fn new(fft_size: usize) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);

        // Hann window: w[i] = 0.5 * (1 - cos(2π * i / (N-1)))
        let window: Vec<f32> = (0..fft_size)
            .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / (fft_size - 1) as f32).cos()))
            .collect();

        let scratch = vec![Complex::new(0.0, 0.0); fft_size];

        Self {
            fft,
            scratch,
            window,
            size: fft_size,
        }
    }

    pub fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        let n = self.size.min(samples.len());

        // Apply window and copy to scratch buffer (as complex)
        for (scratch, (&sample, &w)) in self
            .scratch
            .iter_mut()
            .zip(samples.iter().zip(self.window.iter()).take(n))
        {
            *scratch = Complex::new(sample * w, 0.0);
        }
        for scratch in self.scratch.iter_mut().skip(n) {
            *scratch = Complex::new(0.0, 0.0);
        }

        // FFT
        self.fft.process(&mut self.scratch);

        // Magnitude of first half (up to Nyquist)
        let half = self.size / 2;
        let magnitudes: Vec<f32> = (0..half)
            .map(|i| {
                let c = self.scratch[i];
                (c.re * c.re + c.im * c.im).sqrt()
            })
            .collect();

        magnitudes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fft_440hz() {
        let fft_size = 2048;
        let sample_rate = 44100;
        let freq = 440.0;

        // Generate a 440Hz sine wave
        let samples: Vec<f32> = (0..fft_size)
            .map(|i| (2.0 * PI * freq * i as f32 / sample_rate as f32).sin())
            .collect();

        let mut analyzer = FftAnalyzer::new(fft_size);
        let magnitudes = analyzer.process(&samples);

        // Find the peak bin
        let (peak_idx, _) = magnitudes
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .unwrap();

        // Expected bin: freq * fft_size / sample_rate = 440 * 2048 / 44100 ≈ 20.4
        let expected_bin = (freq * fft_size as f32 / sample_rate as f32).round() as usize;
        let diff = (peak_idx as i32 - expected_bin as i32).abs();
        assert!(
            diff <= 2,
            "Peak bin {peak_idx} should be within ±2 of expected {expected_bin}"
        );
    }
}
