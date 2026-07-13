#[allow(dead_code)]
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    pub timestamp: Duration,
    pub text: String,
    pub word_timestamps: Vec<(Duration, String)>,
}

#[derive(Debug, Clone)]
pub struct LyricTrack {
    #[allow(dead_code)]
    pub metadata: LyricMetadata,
    pub lines: Vec<LyricLine>,
}

#[derive(Debug, Clone, Default)]
pub struct LyricMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub global_offset_ms: i64,
}
