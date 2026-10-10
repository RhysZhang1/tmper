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

fn timestamp(cap: &regex::Captures<'_>) -> Duration {
    let min: u64 = cap[1].parse().unwrap_or(0);
    let sec: u64 = cap[2].parse().unwrap_or(0);
    let millis = cap
        .get(3)
        .map(|frac| {
            frac.as_str().parse::<u64>().unwrap_or(0) * 10u64.pow(3 - frac.as_str().len() as u32)
        })
        .unwrap_or(0);
    Duration::from_millis(min * 60000 + sec * 1000 + millis)
}

pub fn parse_lrc(content: &str) -> AppResult<LyricTrack> {
    use std::sync::OnceLock;
    static TAG: OnceLock<Regex> = OnceLock::new();
    static TIME: OnceLock<Regex> = OnceLock::new();
    static WORD: OnceLock<Regex> = OnceLock::new();
    let tag_re = TAG.get_or_init(|| Regex::new(r"\[(ti|ar|al|by|offset|length):(.+?)\]").unwrap());
    let time_re =
        TIME.get_or_init(|| Regex::new(r"\[(\d{1,3}):(\d{2})(?:\.(\d{1,3}))?\]").unwrap());
    let word_re = WORD.get_or_init(|| Regex::new(r"<(\d{1,3}):(\d{2})(?:\.(\d{1,3}))?>").unwrap());
    let mut metadata = LyricMetadata::default();
    let mut lines = Vec::new();
    for line in content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        for cap in tag_re.captures_iter(line) {
            match &cap[1] {
                "ti" => metadata.title = Some(cap[2].to_string()),
                "ar" => metadata.artist = Some(cap[2].to_string()),
                "offset" => metadata.global_offset_ms = cap[2].parse().unwrap_or(0),
                _ => {}
            }
        }
        let captures: Vec<_> = time_re.captures_iter(line).collect();
        let Some(last) = captures.last() else {
            continue;
        };
        let raw = line[last.get(0).unwrap().end()..].trim();
        let words: Vec<_> = word_re.captures_iter(raw).collect();
        let text = word_re.replace_all(raw, "").into_owned();
        let base = timestamp(&captures[0]);
        for cap in &captures {
            let line_time = timestamp(cap);
            let word_timestamps = words
                .iter()
                .enumerate()
                .map(|(i, word)| {
                    let end = words
                        .get(i + 1)
                        .map(|next| next.get(0).unwrap().start())
                        .unwrap_or(raw.len());
                    let text = raw[word.get(0).unwrap().end()..end].to_string();
                    let time = timestamp(word)
                        .saturating_add(line_time)
                        .saturating_sub(base);
                    (time, text)
                })
                .collect();
            lines.push(LyricLine {
                timestamp: line_time,
                text: text.clone(),
                word_timestamps,
            });
        }
    }
    lines.sort_by_key(|line| line.timestamp);
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
    fn enhanced_lyrics_preserve_spacing_remove_tags_and_shift_repeated_words() {
        let track =
            parse_lrc("[offset:500]\n[0:01][0:11]Intro <0:01.5>你好 <0:02.05>world<0:03.125>")
                .unwrap();
        assert_eq!(track.lines[0].text, "Intro 你好 world");
        assert_eq!(
            track.lines[0].word_timestamps[0],
            (Duration::from_millis(1500), "你好 ".into())
        );
        assert_eq!(
            track.lines[1].word_timestamps[0].0,
            Duration::from_millis(11500)
        );
        assert_eq!(track.adjusted_position(1.0, -200), 1.3);
        assert_eq!(track.adjusted_position(0.0, -1000), 0.0);
    }

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
