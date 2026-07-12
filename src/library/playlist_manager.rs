use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::playlist::{Playlist, TrackEntry};

#[allow(dead_code)]
pub fn import_m3u(path: &Path) -> AppResult<Playlist> {
    let content = fs::read_to_string(path)
        .map_err(|e| AppError::Config(format!("Failed to read M3U file: {e}")))?;

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported")
        .to_string();

    let mut playlist = Playlist::new(&name);
    let base_dir = path.parent().unwrap_or(Path::new("."));

    let mut pending_title: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("#EXTM3U") {
            continue;
        }

        if line.starts_with("#EXTINF:") {
            // #EXTINF:seconds,display title
            let info = line.trim_start_matches("#EXTINF:");
            if let Some(comma_pos) = info.find(',') {
                let title_part = info[comma_pos + 1..].trim();
                if !title_part.is_empty() {
                    pending_title = Some(title_part.to_string());
                }
            }
        } else if !line.starts_with('#') {
            // File path line
            let file_path = PathBuf::from(line);
            let abs_path = if file_path.is_absolute() {
                file_path
            } else {
                base_dir.join(&file_path)
            };

            let title = pending_title.take().unwrap_or_else(|| {
                abs_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string()
            });

            playlist.push(TrackEntry::new(abs_path, title, String::new(), 0.0));
        }
    }

    Ok(playlist)
}

#[allow(dead_code)]
pub fn export_m3u(playlist: &Playlist, path: &Path) -> AppResult<()> {
    let mut content = String::from("#EXTM3U\n");

    for track in &playlist.tracks {
        let duration_secs = track.duration_secs as u64;
        let display = format!("{} - {}", track.artist, track.title);
        content.push_str(&format!("#EXTINF:{duration_secs},{display}\n"));
        content.push_str(&format!("{}\n", track.path.display()));
    }

    fs::write(path, content)
        .map_err(|e| AppError::Config(format!("Failed to write M3U file: {e}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_import_export_roundtrip() {
        let tmp = std::env::temp_dir().join("termusic_test_m3u");
        let _ = fs::create_dir_all(&tmp);

        let m3u_path = tmp.join("test.m3u");
        let m3u_content = "#EXTM3U\n#EXTINF:200,Song A - Artist\n/music/a.mp3\n#EXTINF:300,Song B\n/music/b.flac\n";
        fs::write(&m3u_path, m3u_content).unwrap();

        let playlist = import_m3u(&m3u_path).expect("import failed");
        assert_eq!(playlist.tracks.len(), 2);
        assert_eq!(playlist.tracks[0].title, "Song A - Artist");
        assert_eq!(playlist.tracks[1].title, "Song B");

        // Export
        let export_path = tmp.join("export.m3u");
        export_m3u(&playlist, &export_path).expect("export failed");

        // Re-import exported
        let playlist2 = import_m3u(&export_path).expect("re-import failed");
        assert_eq!(playlist2.tracks.len(), 2);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_import_relative_paths() {
        let tmp = std::env::temp_dir().join("termusic_test_m3u_rel");
        fs::create_dir_all(&tmp).unwrap();
        fs::create_dir_all(tmp.join("subdir")).unwrap();

        let m3u_path = tmp.join("playlist.m3u");
        let content = "#EXTM3U\n../song.mp3\nsubdir/other.flac\n";
        fs::write(&m3u_path, content).unwrap();

        let playlist = import_m3u(&m3u_path).expect("import failed");
        assert_eq!(playlist.tracks.len(), 2);
        // Relative paths should be resolved relative to M3U location
        assert!(playlist.tracks[0]
            .path
            .to_string_lossy()
            .contains("song.mp3"));

        fs::remove_dir_all(&tmp).unwrap();
    }
}
