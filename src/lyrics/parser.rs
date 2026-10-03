use std::time::Duration;

use regex::Regex;

use crate::error::{AppError, AppResult};
use crate::lyrics::types::{LyricLine, LyricMetadata, LyricTrack};

fn decode_with_fallback(bytes: &[u8], fallbacks: &[&str]) -> AppResult<String> {
    if let Some((encoding, _)) = encoding_rs::Encoding::for_bom(bytes) {
        let (decoded, _, had_errors) = encoding.decode(bytes);
        if !had_errors {
            return Ok(decoded.into_owned());
        }
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Ok(s.to_string());
    }
    for label in fallbacks {
        if let Some(encoding) = encoding_rs::Encoding::for_label(label.as_bytes()) {
            let (decoded, _, had_errors) = encoding.decode(bytes);
            if !had_errors {
                return Ok(decoded.into_owned());
            }
        }
    }
    // Last resort: try first fallback even with errors
    if let Some(label) = fallbacks.first() {
        if let Some(encoding) = encoding_rs::Encoding::for_label(label.as_bytes()) {
            let (decoded, _, _) = encoding.decode(bytes);
            return Ok(decoded.into_owned());
        }
    }
    Err(AppError::Lyrics("All encodings failed".into()).into())
}

pub fn parse_lrc(content: &str) -> AppResult<LyricTrack> {
    let tag_re = Regex::new(r"\[(ti|ar|al|by|offset|length):(.+?)\]").unwrap();
    let time_re = Regex::new(r"\[(\d{2}):(\d{2})\.(\d{2,3})\]").unwrap();
    let word_re = Regex::new(r"<(\d{2}):(\d{2})\.(\d{2,3})>").unwrap();

    let mut metadata = LyricMetadata::default();
    let mut lines: Vec<LyricLine> = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let mut has_time_tag = false;

        // Extract metadata tags
        for cap in tag_re.captures_iter(line) {
            let key = cap.get(1).unwrap().as_str();
            let value = cap.get(2).unwrap().as_str();
            match key {
                "ti" => metadata.title = Some(value.to_string()),
                "ar" => metadata.artist = Some(value.to_string()),
                "offset" => {
                    metadata.global_offset_ms = value.parse::<i64>().unwrap_or(0);
                }
                _ => {}
            }
        }

        // Extract time tags
        let timestamps: Vec<Duration> = time_re
            .captures_iter(line)
            .map(|cap| {
                let min: u64 = cap.get(1).unwrap().as_str().parse().unwrap_or(0);
                let sec: u64 = cap.get(2).unwrap().as_str().parse().unwrap_or(0);
                let frac_str = cap.get(3).unwrap().as_str();
                let frac: u64 = if frac_str.len() == 2 {
                    frac_str.parse().unwrap_or(0) * 10 // centiseconds to ms
                } else {
                    frac_str.parse().unwrap_or(0) // milliseconds
                };
                Duration::from_millis(min * 60000 + sec * 1000 + frac)
            })
            .collect();

        if !timestamps.is_empty() {
            has_time_tag = true;

            // Find the last time tag position for text extraction
            let mut last_end = 0usize;
            for cap in time_re.captures_iter(line) {
                if let Some(m) = cap.get(0) {
                    last_end = last_end.max(m.end());
                }
            }

            let text_after_tags = line[last_end..].trim().to_string();

            // Parse word-level timestamps
            let word_timestamps: Vec<(Duration, String)> = word_re
                .captures_iter(line)
                .map(|cap| {
                    let min: u64 = cap.get(1).unwrap().as_str().parse().unwrap_or(0);
                    let sec: u64 = cap.get(2).unwrap().as_str().parse().unwrap_or(0);
                    let frac_str = cap.get(3).unwrap().as_str();
                    let frac: u64 = if frac_str.len() == 2 {
                        frac_str.parse().unwrap_or(0) * 10
                    } else {
                        frac_str.parse().unwrap_or(0)
                    };
                    let ts = Duration::from_millis(min * 60000 + sec * 1000 + frac);
                    (ts, String::new())
                })
                .collect();

            // If we have word timestamps, extract the word text between/before tags
            let mut word_texts: Vec<String> = Vec::new();
            if !word_timestamps.is_empty() {
                let remaining = &line[last_end..];
                let parts: Vec<&str> = word_re.split(remaining).collect();
                for p in parts {
                    let trimmed = p.trim().to_string();
                    if !trimmed.is_empty() {
                        word_texts.push(trimmed);
                    }
                }
            }

            let word_timestamps: Vec<(Duration, String)> = word_timestamps
                .into_iter()
                .enumerate()
                .map(|(i, (ts, _))| {
                    let text = word_texts.get(i).cloned().unwrap_or_default();
                    (ts, text)
                })
                .collect();

            // Create a line for each timestamp (supports multi-timestamp lines)
            for ts in &timestamps {
                lines.push(LyricLine {
                    timestamp: *ts,
                    text: text_after_tags.clone(),
                    word_timestamps: word_timestamps.clone(),
                });
            }
        }

        // Lines without time tags but have content are ignored (comments/empty)
        if !has_time_tag && !line.starts_with('[') {
            // Plain text line without time tag — skip
        }
    }

    // Sort by timestamp
    lines.sort_by_key(|l| l.timestamp);

    Ok(LyricTrack { metadata, lines })
}

pub fn load_lrc_file(path: &std::path::Path, fallbacks: &[&str]) -> AppResult<LyricTrack> {
    let bytes = std::fs::read(path)
        .map_err(|e| AppError::Lyrics(format!("Failed to read LRC file: {e}")))?;
    let content = decode_with_fallback(&bytes, fallbacks)?;
    parse_lrc(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_standard_lrc() {
        let lrc = "[00:12.34]Hello\n[00:15.67]World";
        let track = parse_lrc(lrc).expect("Failed to parse");

        assert_eq!(track.lines.len(), 2);
        assert_eq!(track.lines[0].text, "Hello");
        assert_eq!(track.lines[0].timestamp, Duration::from_millis(12340));
        assert_eq!(track.lines[1].text, "World");
        assert_eq!(track.lines[1].timestamp, Duration::from_millis(15670));
    }

    #[test]
    fn test_parse_metadata() {
        let lrc = "[ti:Song Title]\n[ar:Artist Name]\n[offset:+500]\n[00:10.00]Lyric";
        let track = parse_lrc(lrc).expect("Failed to parse");

        assert_eq!(track.metadata.title.as_deref(), Some("Song Title"));
        assert_eq!(track.metadata.artist.as_deref(), Some("Artist Name"));
        assert_eq!(track.metadata.global_offset_ms, 500);
        assert_eq!(track.lines.len(), 1);
    }

    #[test]
    fn test_multi_timestamp() {
        let lrc = "[00:10.00][01:00.00]Repeated chorus";
        let track = parse_lrc(lrc).expect("Failed to parse");

        assert_eq!(track.lines.len(), 2);
        assert_eq!(track.lines[0].timestamp, Duration::from_millis(10000));
        assert_eq!(track.lines[1].timestamp, Duration::from_millis(60000));
        assert_eq!(track.lines[0].text, "Repeated chorus");
        assert_eq!(track.lines[1].text, "Repeated chorus");
    }

    #[test]
    fn test_word_timestamps() {
        let lrc = "[00:18.90]<00:18.90>A<00:19.20>B";
        let track = parse_lrc(lrc).expect("Failed to parse");

        assert_eq!(track.lines.len(), 1);
        let line = &track.lines[0];
        assert!(!line.word_timestamps.is_empty());
        assert_eq!(line.word_timestamps.len(), 2);
    }

    #[test]
    fn test_empty_lrc() {
        let track = parse_lrc("").expect("Failed to parse");
        assert!(track.lines.is_empty());
    }

    #[test]
    fn test_corrupt_lines() {
        let lrc = "[00:12.34]Valid\n[xx:yy.zz]Bad\n[00:15.00]Also valid";
        let track = parse_lrc(lrc).expect("Should not panic");

        // Bad line is skipped
        assert_eq!(track.lines.len(), 2);
    }

    #[test]
    fn test_lines_sorted() {
        let lrc = "[00:30.00]Third\n[00:10.00]First\n[00:20.00]Second";
        let track = parse_lrc(lrc).expect("Failed to parse");

        assert_eq!(track.lines[0].timestamp, Duration::from_millis(10000));
        assert_eq!(track.lines[1].timestamp, Duration::from_millis(20000));
        assert_eq!(track.lines[2].timestamp, Duration::from_millis(30000));
    }

    // ── Encoding detection ──

    #[test]
    fn plain_utf8_passes_through() {
        let decoded = decode_with_fallback("歌词".as_bytes(), &[]).expect("utf-8 decodes");
        assert_eq!(decoded, "歌词");
    }

    #[test]
    fn a_byte_order_mark_selects_the_encoding() {
        // UTF-16LE BOM followed by "hi".
        let decoded = decode_with_fallback(b"\xFF\xFE\x68\x00\x69\x00", &[]).expect("BOM decodes");
        assert!(decoded.contains("hi"), "got {decoded:?}");
    }

    #[test]
    fn gbk_bytes_decode_with_a_gbk_fallback() {
        let gbk = b"\xC4\xE3\xBA\xC3"; // 你好
        assert_eq!(
            decode_with_fallback(gbk, &["gbk"]).expect("decodes"),
            "你好"
        );
    }

    #[test]
    fn shift_jis_bytes_decode_when_offered() {
        let sjis = b"\x82\xB1\x82\xF1\x82\xC9\x82\xBF\x82\xCD"; // こんにちは
        assert_eq!(
            decode_with_fallback(sjis, &["shift-jis"]).expect("decodes"),
            "こんにちは"
        );
    }

    /// The caller passes `["utf-8", "gbk", "shift-jis"]`, and GBK accepts
    /// almost any byte sequence — so a Shift-JIS lyric sheet is decoded as GBK
    /// mojibake and the shift-jis attempt is never reached. Pinned here so the
    /// limitation is visible: reordering the list or sniffing would fix it,
    /// and either change should be a deliberate one.
    #[test]
    fn the_caller_s_fallback_order_decodes_shift_jis_as_gbk() {
        let sjis = b"\x82\xB1\x82\xF1\x82\xC9\x82\xBF\x82\xCD"; // こんにちは

        let with_caller_order =
            decode_with_fallback(sjis, &["utf-8", "gbk", "shift-jis"]).expect("decodes");
        assert_ne!(
            with_caller_order, "こんにちは",
            "GBK claims the bytes before shift-jis is tried"
        );
        assert_eq!(
            decode_with_fallback(sjis, &["shift-jis"]).expect("decodes"),
            "こんにちは",
            "shift-jis decodes them correctly when reached first"
        );
    }

    /// The last resort decodes with the first fallback *even if it reported
    /// errors*, so undecodable input produces replacement characters rather
    /// than failing — a partly garbled lyric beats no lyric.
    #[test]
    fn undecodable_bytes_yield_replacement_characters_not_an_error() {
        let junk = b"\xFF\xFF\xFF\xFF";
        let decoded = decode_with_fallback(junk, &["gbk"]).expect("last resort never fails");
        assert!(!decoded.is_empty());
    }

    /// Which leaves exactly one way to reach the error: no fallbacks at all.
    #[test]
    fn without_any_fallback_undecodable_input_errors() {
        assert!(decode_with_fallback(b"\xFF\xFF\xFF\xFF", &[]).is_err());
    }

    // ── File loading ──

    #[test]
    fn load_lrc_file_reads_and_parses() {
        let dir = std::env::temp_dir().join(format!("tmper-lrc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("song.lrc");
        std::fs::write(&path, "[00:01.00]Hello\n[00:02.50]World\n").unwrap();

        let track = load_lrc_file(&path, &["utf-8"]).expect("loads");

        assert_eq!(track.lines.len(), 2);
        assert_eq!(track.lines[1].text, "World");
        assert_eq!(track.lines[1].timestamp, Duration::from_millis(2500));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_lrc_file_reports_a_missing_file() {
        let missing = std::path::Path::new("/definitely/not/here.lrc");
        assert!(load_lrc_file(missing, &["utf-8"]).is_err());
    }
}
