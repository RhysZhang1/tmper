use std::path::PathBuf;

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
    use super::*;
    use std::fs;

    #[test]
    fn test_scan_filters_by_extension() {
        let tmp = std::env::temp_dir().join("termusic_test_scanner");
        fs::create_dir_all(&tmp).unwrap();

        // Create test files
        fs::write(tmp.join("song.mp3"), b"fake").unwrap();
        fs::write(tmp.join("song.flac"), b"fake").unwrap();
        fs::write(tmp.join("readme.txt"), b"fake").unwrap();
        fs::write(tmp.join("cover.jpg"), b"fake").unwrap();

        let exts: Vec<String> = vec!["mp3".into(), "flac".into()];
        let results = scan_directory(&tmp, &exts);

        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|p| p.extension().unwrap() == "mp3"));
        assert!(results.iter().any(|p| p.extension().unwrap() == "flac"));
        assert!(!results.iter().any(|p| p.extension().unwrap() == "txt"));

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_scan_empty_directory() {
        let tmp = std::env::temp_dir().join("termusic_test_empty");
        fs::create_dir_all(&tmp).unwrap();

        let exts: Vec<String> = vec!["mp3".into()];
        let results = scan_directory(&tmp, &exts);

        assert!(results.is_empty());

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn test_scan_nonexistent_directory() {
        let exts: Vec<String> = vec!["mp3".into()];
        let results = scan_directory(&std::path::PathBuf::from("/nonexistent/path/12345"), &exts);
        assert!(results.is_empty());
    }
}
