use std::fs::File;
use std::path::{Path, PathBuf};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{Time, TimeBase};

use crate::error::{AppError, AppResult};

pub struct AudioDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    path: PathBuf,
    time_base: Option<TimeBase>,
    skip_frames: u64,
    pending_samples: Option<Vec<f32>>,
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
            path: path.to_path_buf(),
            time_base: codec_params.time_base,
            skip_frames: 0,
            pending_samples: None,
            sample_rate,
            channels,
            total_frames,
        })
    }

    pub fn read_packet(&mut self) -> AppResult<Option<Vec<f32>>> {
        if let Some(samples) = self.pending_samples.take() {
            return Ok(Some(samples));
        }
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(ref e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
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
                    // Vorbis setup/seek packets can decode to zero frames.
                    // SampleBuffer::copy_interleaved_ref assumes a nonempty plane.
                    if num_frames == 0 {
                        continue;
                    }
                    let spec = *decoded.spec();

                    let mut sample_buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
                    sample_buf.copy_interleaved_ref(decoded);

                    let channels = self.channels.max(1) as usize;
                    let discard = self.skip_frames.min(num_frames as u64) as usize;
                    self.skip_frames -= discard as u64;
                    let samples = sample_buf.samples()[discard * channels..].to_vec();
                    if samples.is_empty() {
                        continue;
                    }
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
                    let needed = target_frames - skipped;
                    if frame_count > needed {
                        self.pending_samples =
                            Some(samples[needed as usize * self.channels as usize..].to_vec());
                        skipped = target_frames;
                    } else {
                        skipped += frame_count;
                    }
                }
                None => break,
            }
        }
        Ok(skipped)
    }

    /// Container-level native seek to `target_secs`.
    ///
    /// Uses the format reader's seek table (MP3/FLAC/M4A etc.) instead of
    /// decoding-and-discarding. After seeking, the decoder is `reset()` to
    /// flush its internal buffers (required by symphonia after a format seek).
    ///
    /// Falls back to `skip_to_secs` if the container does not support
    /// seeking — preserving the previous behavior for those formats.
    pub fn seek_to_secs(&mut self, target_secs: f64) -> AppResult<()> {
        let target = target_secs.max(0.0);
        let seek_to = SeekTo::Time {
            time: Time::new(target.floor() as u64, target.fract()),
            track_id: Some(self.track_id),
        };
        self.pending_samples = None;
        self.skip_frames = 0;
        match self.format.seek(SeekMode::Accurate, seek_to) {
            Ok(seeked) => {
                self.decoder.reset();
                if let Some(base) = self.time_base {
                    let actual = base.calc_time(seeked.actual_ts);
                    let actual_secs = actual.seconds as f64 + actual.frac;
                    self.skip_frames =
                        ((target - actual_secs).max(0.0) * self.sample_rate as f64).round() as u64;
                }
                Ok(())
            }
            Err(e) => {
                tracing::warn!(
                    "native seek failed ({e}); falling back to decode-skip to {target_secs}s"
                );
                // A failed seek may have moved the container cursor. Decode
                // the fallback from the beginning, retaining the packet tail.
                *self = Self::open(&self.path.clone())?;
                self.skip_to_secs(target)?;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_formats_decode_to_finite_pcm_and_seek_to_the_requested_position() {
        for name in [
            "pcm.wav",
            "lossless.flac",
            "mpeg.mp3",
            "vorbis.ogg",
            "aac.m4a",
            "alac.m4a",
            "pcm.aiff",
            "adts.aac",
        ] {
            let path = Path::new("tests/fixtures/formats").join(name);
            crate::metadata::reader::read_metadata(&path)
                .unwrap_or_else(|e| panic!("{name}: metadata: {e}"));
            let mut decoder = AudioDecoder::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
            let mut count = 0;
            let mut energy = 0.0f64;
            while let Some(samples) = decoder
                .read_packet()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                assert!(
                    samples.iter().all(|v| v.is_finite()),
                    "{name}: non-finite PCM"
                );
                count += samples.len();
                energy += samples.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
            }
            let seconds = count as f64 / decoder.sample_rate as f64 / decoder.channels as f64;
            assert!((2.9..3.2).contains(&seconds), "{name}: decoded {seconds}s");
            assert!(energy / count as f64 > 0.0001, "{name}: unexpected silence");
            // A non-packet-aligned target detects ignored actual_ts values.
            decoder
                .seek_to_secs(1.234)
                .unwrap_or_else(|e| panic!("{name}: seek: {e}"));
            let mut remaining = 0;
            while let Some(samples) = decoder
                .read_packet()
                .unwrap_or_else(|e| panic!("{name}: after seek: {e}"))
            {
                remaining += samples.len();
            }
            let seconds = remaining as f64 / decoder.sample_rate as f64 / decoder.channels as f64;
            assert!(
                (1.73..1.91).contains(&seconds),
                "{name}: seek left {seconds}s"
            );
        }
    }

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

    #[test]
    fn test_seek_to_secs_yields_samples() {
        let path = Path::new("tests/fixtures/test.wav");
        let mut decoder = AudioDecoder::open(path).expect("Failed to open test.wav");

        // Native seek to the middle of the 2s fixture.
        decoder
            .seek_to_secs(1.0)
            .expect("seek_to_secs should succeed on wav");

        // After seeking, the next read must still produce samples.
        let packet = decoder
            .read_packet()
            .expect("read_packet after seek should not error")
            .expect("should have samples after seek");
        assert!(!packet.is_empty(), "post-seek packet must be non-empty");
    }
}
