# Synthetic format regression corpus

These files contain a three-second 440 Hz sine wave at 44.1 kHz, stereo. They
were generated locally with FFmpeg; no third-party recording was downloaded.
They are distributed under the project's MIT license. FFmpeg is a fixture
regeneration tool, not a tmper build or runtime dependency.

Regenerate from the repository root:

```sh
python3 scripts/generate_audio_fixtures.py
```

| File | Codec/container |
| --- | --- |
| `pcm.wav` | signed 16-bit little-endian PCM / WAV |
| `lossless.flac` | FLAC |
| `mpeg.mp3` | MPEG Layer III |
| `vorbis.ogg` | Vorbis / Ogg |
| `aac.m4a` | AAC-LC / MP4 |
| `alac.m4a` | ALAC / MP4 |
| `pcm.aiff` | signed 16-bit big-endian PCM / AIFF |
| `adts.aac` | AAC-LC / ADTS |

The regression test verifies metadata fallback, complete decoding through EOF,
finite non-silent PCM, duration, and a non-packet-aligned seek to 1.234 seconds.
The Vorbis case also guards zero-frame packets that previously panicked during
sample copying. `SHA256SUMS` records the checked-in bytes; regenerated lossy
encodings may differ with the installed FFmpeg encoder version.
