use std::path::{Path, PathBuf};

use crate::error::AppResult;
use crate::lyrics::parser::load_lrc_file;
use crate::lyrics::types::LyricTrack;

#[allow(dead_code)]
pub struct LyricEngine;

#[allow(dead_code)]
impl LyricEngine {
    pub fn find_lyrics(audio_path: &Path) -> Option<PathBuf> {
        // 1. Same-name .lrc file
        let lrc_path = audio_path.with_extension("lrc");
        if lrc_path.exists() {
            return Some(lrc_path);
        }

        // 2. Check for embedded lyrics (handled later via lofty in load())
        // 3. Check unified lyrics directory
        if let Some(lyrics_dir) = Some(crate::paths::data_dir()) {
            let name = audio_path.file_stem()?;
            let mut lrc_in_dir = lyrics_dir.join("Lyrics");
            lrc_in_dir.push(name);
            lrc_in_dir.set_extension("lrc");
            if lrc_in_dir.exists() {
                return Some(lrc_in_dir);
            }
        }

        None
    }

    pub fn load(audio_path: &Path) -> AppResult<Option<LyricTrack>> {
        let fallbacks = &["utf-8", "gbk", "shift-jis"];
        let lrc_path = match Self::find_lyrics(audio_path) {
            Some(p) => p,
            None => return Ok(None),
        };
        let track = load_lrc_file(&lrc_path, fallbacks)?;
        Ok(Some(track))
    }

    pub fn sync(track: &LyricTrack, position_secs: f64, _hint_index: usize) -> usize {
        use std::time::Duration;

        let target = Duration::from_secs_f64(position_secs);

        // Binary search for the last line with timestamp <= target
        let mut lo = 0i32;
        let mut hi = track.lines.len() as i32 - 1;
        let mut result = 0usize;

        while lo <= hi {
            let mid = (lo + hi) / 2;
            if track.lines[mid as usize].timestamp <= target {
                result = mid as usize;
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }

        result
    }
}
