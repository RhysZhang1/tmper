use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    pub timestamp: Duration,
    pub text: String,
    pub word_timestamps: Vec<(Duration, String)>,
}

#[derive(Debug, Clone)]
pub struct LyricTrack {
    pub metadata: LyricMetadata,
    pub lines: Vec<LyricLine>,
}

#[derive(Debug, Clone, Default)]
pub struct LyricMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub global_offset_ms: i64,
}

impl LyricTrack {
    /// Positive file/user offsets advance the lyric clock; negative values delay it.
    pub fn adjusted_position(&self, position: f64, user_offset_ms: i64) -> f64 {
        (position + self.metadata.global_offset_ms as f64 / 1000.0 + user_offset_ms as f64 / 1000.0)
            .max(0.0)
    }
}
