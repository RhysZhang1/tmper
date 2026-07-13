use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::error::{AppError, AppResult};

pub struct AudioDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    pub sample_rate: u32,
    pub channels: u8,
    pub total_frames: u64,
}

impl AudioDecoder {
    pub fn open(path: &Path) -> AppResult<Self> {
        let file =
            File::open(path).map_err(|e| AppError::Audio(format!("Failed to open file: {e}")))?;

        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions::default();
        let metadata_opts = MetadataOptions::default();
        let decoder_opts = DecoderOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .map_err(|e| AppError::Audio(format!("Failed to probe format: {e}")))?;

        let format = probed.format;

        let track = format
            .default_track()
            .ok_or_else(|| AppError::Audio("No default track found".into()))?;

        let track_id = track.id;

        let codec_params = track.codec_params.clone();

        let decoder = symphonia::default::get_codecs()
            .make(&codec_params, &decoder_opts)
            .map_err(|e| AppError::Audio(format!("Failed to create decoder: {e}")))?;

        let sample_rate = codec_params.sample_rate.unwrap_or(44100);

        let channels = codec_params.channels.map(|c| c.count() as u8).unwrap_or(2);

        let total_frames = codec_params.n_frames.unwrap_or(0);

        Ok(Self {
            format,
            decoder,
            track_id,
            sample_rate,
            channels,
            total_frames,
        })
    }

    pub fn read_packet(&mut self) -> AppResult<Option<Vec<f32>>> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(ref e)) if e.to_string().contains("end of stream") => {
                    return Ok(None);
                }
                Err(e) => {
                    return Err(AppError::Audio(format!("Failed to read packet: {e}")).into());
                }
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let num_frames = decoded.frames();
                    let spec = *decoded.spec();

                    let mut sample_buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
                    sample_buf.copy_interleaved_ref(decoded);

                    let samples = sample_buf.samples().to_vec();
                    return Ok(Some(samples));
                }
                Err(SymphoniaError::DecodeError("no more data")) => {
                    continue;
                }
                Err(e) => {
                    return Err(AppError::Audio(format!("Failed to decode packet: {e}")).into());
                }
            }
        }
    }

    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate > 0 && self.total_frames > 0 {
            self.total_frames as f64 / self.sample_rate as f64
        } else {
            0.0
        }
    }

    /// Fast-forward: decode and discard packets until `target_secs` is reached.
    /// Returns the total number of frames skipped, or an error.
    pub fn skip_to_secs(&mut self, target_secs: f64) -> AppResult<u64> {
        let target_frames = (target_secs * self.sample_rate as f64) as u64;
        let mut skipped = 0u64;
        while skipped < target_frames {
            match self.read_packet()? {
                Some(samples) => {
                    let frame_count = samples.len() as u64 / self.channels as u64;
                    skipped += frame_count;
                }
                None => break,
            }
        }
        Ok(skipped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_wav() {
        let path = Path::new("tests/fixtures/test.wav");
        let mut decoder = AudioDecoder::open(path).expect("Failed to open test.wav");

        assert_eq!(decoder.sample_rate, 44100);
        assert_eq!(decoder.channels, 2);
        assert!(decoder.duration_secs() > 1.5);

        let mut total_samples = 0usize;
        while let Ok(Some(packet)) = decoder.read_packet() {
            total_samples += packet.len();
        }

        assert!(total_samples > 0, "Should have decoded some samples");
        let expected_min = (decoder.sample_rate as usize * decoder.channels as usize * 2) * 9 / 10;
        assert!(
            total_samples >= expected_min,
            "Should have decoded roughly 2 seconds of audio"
        );
    }

    #[test]
    fn test_open_nonexistent_file() {
        let result = AudioDecoder::open(Path::new("tests/fixtures/nonexistent.wav"));
        assert!(result.is_err());
    }
}
