#!/usr/bin/env python3
"""Regenerate tmper's synthetic, copyright-free decoder regression corpus."""
from pathlib import Path
import subprocess

FORMATS = {
    "pcm.wav": "pcm_s16le", "lossless.flac": "flac", "mpeg.mp3": "libmp3lame",
    "vorbis.ogg": "libvorbis", "aac.m4a": "aac", "alac.m4a": "alac",
    "pcm.aiff": "pcm_s16be", "adts.aac": "aac",
}

if __name__ == "__main__":
    destination = Path(__file__).resolve().parents[1] / "tests/fixtures/formats"
    destination.mkdir(parents=True, exist_ok=True)
    for name, codec in FORMATS.items():
        subprocess.run([
            "ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi",
            "-i", "sine=frequency=440:duration=3", "-ar", "44100", "-ac", "2",
            "-c:a", codec, str(destination / name),
        ], check=True)
    print(f"Generated {len(FORMATS)} fixtures in {destination}")
