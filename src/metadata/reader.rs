use std::path::Path;
use std::time::Duration;

use lofty::prelude::*;
use lofty::probe::Probe;

use crate::error::AppResult;

pub struct TrackInfo {
    pub path: std::path::PathBuf,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    /// Reserved for future track-list display columns.
    #[allow(dead_code)]
    pub track_total: Option<u32>,
    pub disc_number: Option<u32>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub duration: Duration,
    pub bitrate: u32,
    pub sample_rate: u32,
    pub channels: u8,
    pub codec: String,
    pub cover_art: Option<Vec<u8>>,
}

pub fn read_metadata(path: &Path) -> AppResult<TrackInfo> {
    let tagged_file = Probe::open(path)
        .map_err(|e| crate::error::AppError::Metadata(format!("Failed to open file: {e}")))?
        .read()
        .map_err(|e| crate::error::AppError::Metadata(format!("Failed to read tags: {e}")))?;

    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());

    let properties = tagged_file.properties();

    let title = tag
        .and_then(|t| t.title().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    let artist = tag
        .and_then(|t| t.artist().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    let album = tag
        .and_then(|t| t.album().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    let album_artist = tag
        .and_then(|t| {
            t.get_string(&lofty::tag::ItemKey::AlbumArtist)
                .map(|s| s.to_string())
        })
        .filter(|s| !s.is_empty());

    let genre = tag
        .and_then(|t| t.genre().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    let year = tag.and_then(|t| t.year());
    let track_number = tag.and_then(|t| t.track());
    let track_total = tag.and_then(|t| t.track_total());
    let disc_number = tag.and_then(|t| t.disk());

    let duration = properties.duration();
    let sample_rate = properties.sample_rate().unwrap_or(44100);
    let channels = properties.channels().unwrap_or(2);
    let bitrate = properties.audio_bitrate().unwrap_or(0);

    let codec = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_uppercase();

    let cover_art = tag.and_then(|t| t.pictures().first().map(|p| p.data().to_vec()));

    let title = title.unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string()
    });

    Ok(TrackInfo {
        path: path.to_path_buf(),
        title,
        artist,
        album,
        album_artist,
        track_number,
        track_total,
        disc_number,
        genre,
        year,
        duration,
        bitrate,
        sample_rate,
        channels,
        codec,
        cover_art,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_flac_with_tags() {
        let path = Path::new("tests/fixtures/test.flac");
        let info = read_metadata(path).expect("Failed to read metadata");

        assert_eq!(info.title, "Test Song");
        assert_eq!(info.artist.as_deref(), Some("Test Artist"));
        assert_eq!(info.album.as_deref(), Some("Test Album"));
        assert_eq!(info.genre.as_deref(), Some("Rock"));
        assert_eq!(info.year, Some(2024));
        assert_eq!(info.track_number, Some(3));
        assert_eq!(info.track_total, Some(12));
        assert_eq!(info.sample_rate, 44100);
        assert_eq!(info.channels, 2);
        assert!(info.duration.as_secs_f64() > 0.5);
    }

    #[test]
    fn test_read_wav_no_tags() {
        let path = Path::new("tests/fixtures/test_notags.wav");
        let info = read_metadata(path).expect("Failed to read metadata");

        assert_eq!(info.title, "test_notags");
        assert_eq!(info.artist, None);
        assert_eq!(info.album, None);
        assert_eq!(info.genre, None);
        assert_eq!(info.year, None);
        assert_eq!(info.sample_rate, 44100);
        assert_eq!(info.channels, 1);
    }

    #[test]
    fn test_nonexistent_file() {
        let result = read_metadata(Path::new("tests/fixtures/nonexistent.flac"));
        assert!(result.is_err());
    }
}
