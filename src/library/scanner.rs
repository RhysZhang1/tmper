use std::path::PathBuf;

/// Scan a directory for audio files matching the given extensions.
/// Used by library view to populate file browser.
#[allow(dead_code)]
pub fn scan_directory(dir: &std::path::Path, extensions: &[String]) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| {
                    extensions
                        .iter()
                        .any(|allowed| allowed == &ext.to_lowercase())
                })
                .unwrap_or(false)
        })
        .map(|e| e.path().to_path_buf())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::scan_directory;
    use std::fs;

    #[test]
    fn test_scan_filters_by_extension() {
        let tmp = std::env::temp_dir().join("tmper_test_scan2");
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("song.mp3"), b"fake").unwrap();
        fs::write(tmp.join("readme.txt"), b"fake").unwrap();

        let exts: Vec<String> = vec!["mp3".into(), "flac".into()];
        let results = scan_directory(&tmp, &exts);
        assert_eq!(results.len(), 1);

        fs::remove_dir_all(&tmp).unwrap();
    }
}
