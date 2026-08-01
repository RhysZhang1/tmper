use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::playlist::PlaylistData;

/// Parse an M3U/M3U8 file into `PlaylistData`. `#EXTINF` titles are parsed
/// and then dropped — only file paths are kept, matching the app's single
/// path-based playlist model.
pub fn import_m3u(path: &Path) -> AppResult<PlaylistData> {
    let content = fs::read_to_string(path)
        .map_err(|e| AppError::Config(format!("Failed to read M3U file: {e}")))?;

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported")
        .to_string();

    let mut songs = Vec::new();
    let base_dir = path.parent().unwrap_or(Path::new("."));

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // File path line (relative paths resolve against the M3U location)
        let file_path = PathBuf::from(line);
        let abs_path = if file_path.is_absolute() {
            file_path
        } else {
            base_dir.join(&file_path)
        };
        songs.push(abs_path);
    }

    Ok(PlaylistData { name, songs })
}

pub fn export_m3u(playlist: &PlaylistData, path: &Path) -> AppResult<()> {
    let mut content = String::from("#EXTM3U\n");

    for song in &playlist.songs {
        let title = song
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown");
        content.push_str(&format!("#EXTINF:0,{title}\n"));
        content.push_str(&format!("{}\n", song.display()));
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
        let tmp = std::env::temp_dir().join("tmper_test_m3u");
        let _ = fs::create_dir_all(&tmp);

        let m3u_path = tmp.join("test.m3u");
        let m3u_content =
            "#EXTM3U\n#EXTINF:200,Song A - Artist\n/music/a.mp3\n#EXTINF:300,Song B\n/music/b.flac\n";
        fs::write(&m3u_path, m3u_content).unwrap();

        let playlist = import_m3u(&m3u_path).expect("import failed");
        assert_eq!(playlist.songs.len(), 2);
        assert!(playlist.songs[0].to_string_lossy().ends_with("a.mp3"));

        // Export
        let export_path = tmp.join("export.m3u");
        export_m3u(&playlist, &export_path).expect("export failed");

        // Re-import exported
        let playlist2 = import_m3u(&export_path).expect("re-import failed");
        assert_eq!(playlist2.songs.len(), 2);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_import_relative_paths() {
        let tmp = std::env::temp_dir().join("tmper_test_m3u_rel");
        fs::create_dir_all(&tmp).unwrap();
        fs::create_dir_all(tmp.join("subdir")).unwrap();

        let m3u_path = tmp.join("playlist.m3u");
        let content = "#EXTM3U\n../song.mp3\nsubdir/other.flac\n";
        fs::write(&m3u_path, content).unwrap();

        let playlist = import_m3u(&m3u_path).expect("import failed");
        assert_eq!(playlist.songs.len(), 2);
        // Relative paths should be resolved relative to M3U location
        assert!(playlist.songs[0].to_string_lossy().contains("song.mp3"));

        fs::remove_dir_all(&tmp).unwrap();
    }
}
