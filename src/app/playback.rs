//! What the *client* still owns about playback.
//!
//! The policy — what plays next, when a track ends, how the repeat mode is
//! applied — moved to [`crate::player`], because it has to keep working when no
//! TUI is attached. What is left here is everything that is genuinely
//! per-client: which row is highlighted, how far the list is scrolled, and the
//! lyric line for the position the player reported. Two attached TUIs each have
//! their own copy of all of it.

use crate::app::App;
use crate::ipc::proto::Request;
use crate::lyrics::engine::LyricEngine;

impl App {
    /// Move selection by `delta` rows, clamping to track list bounds.
    /// Updates scroll offset to keep selection visible.
    pub(super) fn move_selection(&mut self, delta: i32, visible_h: u16) {
        if self.ui_state.player.tracks.is_empty() {
            return;
        }
        let len = self.ui_state.player.tracks.len() as i32;
        let new_idx = (self.ui_state.player.selected_index as i32 + delta).clamp(0, len - 1);
        self.ui_state.player.selected_index = new_idx as usize;

        let scroll = self.ui_state.player.scroll_offset as i32;
        let vis = visible_h as i32;
        if new_idx < scroll {
            self.ui_state.player.scroll_offset = new_idx.max(0) as usize;
        } else if new_idx >= scroll + vis {
            self.ui_state.player.scroll_offset = (new_idx - vis + 1).max(0) as usize;
        }
    }

    pub(super) fn move_scroll(&mut self, delta: i32) {
        let new_scroll = (self.ui_state.player.scroll_offset as i32 + delta).max(0);
        self.ui_state.player.scroll_offset = new_scroll as usize;
    }

    pub(super) fn play_selected(&mut self) {
        if self.ui_state.player.selected_index < self.ui_state.player.tracks.len() {
            let path = self.ui_state.player.tracks[self.ui_state.player.selected_index]
                .path
                .clone();
            self.dispatch(Request::Play { path });
        }
    }

    /// Read the lyric file that sits next to `path`, if it has one.
    ///
    /// The client does this rather than the player: lyrics are a *view* of the
    /// position, the file is on the same disk either way, and shipping the
    /// parsed lines over the socket would buy nothing.
    pub(crate) fn load_lyrics_for_path(&mut self, path: &std::path::Path) {
        match LyricEngine::load(path) {
            Ok(Some(track)) => {
                tracing::info!("Lyrics loaded: {} lines", track.lines.len());
                self.ui_state.lyrics.lyric_track = Some(track);
                self.ui_state.lyrics.current_lyric_index = 0;
            }
            Ok(None) => {
                self.ui_state.lyrics.lyric_track = None;
            }
            Err(e) => {
                tracing::warn!("Failed to load lyrics: {e}");
                self.ui_state.lyrics.lyric_track = None;
            }
        }
    }

    pub(super) fn sync_lyrics(&mut self, position_secs: f64) {
        if let Some(ref track) = self.ui_state.lyrics.lyric_track {
            if track.lines.is_empty() {
                return;
            }
            let adjusted_pos =
                track.adjusted_position(position_secs, self.ui_state.lyrics.lyrics_offset_ms);
            let idx = LyricEngine::sync(
                track,
                adjusted_pos.max(0.0),
                self.ui_state.lyrics.current_lyric_index,
            );
            self.ui_state.lyrics.current_lyric_index = idx;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::handlers::test_support::test_app;
    use crate::ui::TrackDisplay;
    use std::path::PathBuf;

    fn track(path: &str) -> TrackDisplay {
        TrackDisplay {
            path: PathBuf::from(path),
            title: path.into(),
            artist: "X".into(),
            duration_secs: 1.0,
        }
    }

    // ── Selection ──

    #[test]
    fn play_selected_plays_the_highlighted_track() {
        let mut app = test_app();
        app.ui_state.player.tracks = vec![track("/one.flac"), track("/two.flac")];
        app.ui_state.player.selected_index = 1;

        app.play_selected();

        // Neither path exists, so metadata lookup fails and nothing is queued —
        // but the guard must not have panicked or touched the selection.
        assert_eq!(app.ui_state.player.selected_index, 1);
        assert_eq!(app.ui_state.player.playing_index, None);
    }

    /// A selection past the end is the stale-index case again: it must be
    /// ignored rather than panicking.
    #[test]
    fn play_selected_out_of_range_is_a_noop() {
        let mut app = test_app();
        app.ui_state.player.tracks = vec![track("/one.flac")];
        app.ui_state.player.selected_index = 7;

        app.play_selected();

        assert_eq!(app.ui_state.player.playing_index, None);
    }

    // ── Selection movement ──

    #[test]
    fn move_selection_clamps_at_both_ends() {
        let mut app = test_app();
        app.ui_state.player.tracks = vec![track("/a.flac"), track("/b.flac")];

        app.ui_state.player.selected_index = 0;
        app.move_selection(-1, 10);
        assert_eq!(app.ui_state.player.selected_index, 0);

        app.ui_state.player.selected_index = 1;
        app.move_selection(1, 10);
        assert_eq!(app.ui_state.player.selected_index, 1);
    }

    #[test]
    fn move_selection_scrolls_to_keep_the_row_visible() {
        let mut app = test_app();
        app.ui_state.player.tracks = (0..20).map(|i| track(&format!("/t{i}.flac"))).collect();
        app.ui_state.player.selected_index = 0;
        app.ui_state.player.scroll_offset = 0;

        // Walk past the bottom of a 5-row window.
        for _ in 0..6 {
            app.move_selection(1, 5);
        }
        assert!(
            app.ui_state.player.scroll_offset > 0,
            "the window must follow the cursor down"
        );

        // And back above the top.
        for _ in 0..20 {
            app.move_selection(-1, 5);
        }
        assert_eq!(app.ui_state.player.scroll_offset, 0);
    }

    #[test]
    fn move_selection_on_an_empty_queue_does_nothing() {
        let mut app = test_app();
        app.ui_state.player.tracks.clear();
        app.move_selection(1, 10);
        assert_eq!(app.ui_state.player.selected_index, 0);
    }

    #[test]
    fn move_scroll_never_goes_negative() {
        let mut app = test_app();
        app.ui_state.player.scroll_offset = 2;
        app.move_scroll(-5);
        assert_eq!(app.ui_state.player.scroll_offset, 0);
    }

    // ── Lyrics ──

    #[test]
    fn a_track_without_lyrics_clears_the_previous_ones() {
        let mut app = test_app();
        app.ui_state.lyrics.lyric_track = None;
        // No .lrc next to this golden path, so the engine reports "none".
        app.load_lyrics_for_path(std::path::Path::new("/definitely/not/here.flac"));
        assert!(app.ui_state.lyrics.lyric_track.is_none());
    }

    fn two_line_track() -> crate::lyrics::types::LyricTrack {
        use crate::lyrics::types::{LyricLine, LyricTrack};
        use std::time::Duration;
        let line = |secs: u64, text: &str| LyricLine {
            timestamp: Duration::from_secs(secs),
            text: text.into(),
            word_timestamps: Vec::new(),
        };
        LyricTrack {
            metadata: Default::default(),
            lines: vec![line(0, "one"), line(10, "two")],
        }
    }

    /// The offset shifts which line is current, and it is applied on the
    /// client: the player never sees it applied to a position.
    #[test]
    fn sync_lyrics_applies_the_offset() {
        let mut app = test_app();
        app.ui_state.lyrics.lyric_track = Some(two_line_track());
        app.ui_state.lyrics.lyrics_offset_ms = 0;
        app.ui_state.lyrics.current_lyric_index = 0;

        app.sync_lyrics(11.0);
        assert_eq!(app.ui_state.lyrics.current_lyric_index, 1);

        // A +5s offset makes 6s land on the line at 10s.
        app.ui_state.lyrics.current_lyric_index = 0;
        app.ui_state.lyrics.lyrics_offset_ms = 5_000;
        app.sync_lyrics(6.0);
        assert_eq!(app.ui_state.lyrics.current_lyric_index, 1);
    }
}
