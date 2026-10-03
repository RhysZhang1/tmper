use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::lyrics::types::LyricTrack;
use crate::ui::theme::Theme;

pub fn render_lyrics_view(
    f: &mut Frame,
    area: Rect,
    theme: &Theme,
    track: &LyricTrack,
    current_index: usize,
    offset_ms: i64,
) {
    let visible_lines = area.height.saturating_sub(2) as usize; // borders
    if visible_lines < 3 || track.lines.is_empty() {
        let text = if track.lines.is_empty() {
            "No lyrics found"
        } else {
            "Terminal too small"
        };
        let para = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(" Lyrics "))
            .alignment(Alignment::Center);
        f.render_widget(para, area);
        return;
    }

    let half = visible_lines / 2;
    let total = track.lines.len();

    let start = if current_index > half {
        current_index.saturating_sub(half)
    } else {
        0
    };
    let end = (start + visible_lines).min(total);

    let lines: Vec<Line> = (start..end)
        .map(|i| {
            let lyric = &track.lines[i];
            let is_current = i == current_index;
            let is_past = i < current_index;

            let style = if is_current {
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else if is_past {
                let dist = current_index.saturating_sub(i) as f32;
                let fade = (dist / half.max(1) as f32).min(1.0);
                let gray = (200.0 * (1.0 - fade * 0.6)) as u8;
                Style::default().fg(Color::Rgb(gray, gray, gray))
            } else {
                Style::default().fg(Color::Rgb(100, 100, 100))
            };

            Line::from(Span::styled(lyric.text.clone(), style))
        })
        .collect();

    let offset_label = if offset_ms != 0 {
        format!("offset: {:+.3}s", offset_ms as f64 / 1000.0)
    } else {
        String::new()
    };

    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Lyrics {}", offset_label)),
        )
        .alignment(Alignment::Center);

    f.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::types::{LyricLine, LyricMetadata, LyricTrack};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::time::Duration;

    fn make_track(lines: &[&str]) -> LyricTrack {
        LyricTrack {
            metadata: LyricMetadata::default(),
            lines: lines
                .iter()
                .enumerate()
                .map(|(i, t)| LyricLine {
                    timestamp: Duration::from_secs(i as u64),
                    text: t.to_string(),
                    word_timestamps: Vec::new(),
                })
                .collect(),
        }
    }

    /// Render into a 80×24 buffer and return all cell symbols.
    fn render(track: &LyricTrack, idx: usize, offset_ms: i64) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| render_lyrics_view(f, f.area(), &Theme::default(), track, idx, offset_ms))
            .unwrap();
        let buf = terminal.backend().buffer();
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn test_render_empty_shows_message() {
        let track = make_track(&[]);
        let out = render(&track, 0, 0);
        assert!(out.contains("No lyrics found"));
    }

    #[test]
    fn test_render_lyric_lines_and_current_highlight() {
        let track = make_track(&["First line", "Second line"]);
        let out = render(&track, 1, 0);
        assert!(out.contains("First line"), "first lyric rendered");
        assert!(out.contains("Second line"), "current lyric rendered");
    }

    #[test]
    fn test_render_offset_label() {
        let track = make_track(&["line one"]);
        let out = render(&track, 0, 1500);
        assert!(out.contains("offset: +1.500s"));
    }

    #[test]
    fn test_render_scrolls_to_keep_current_visible() {
        // A long list scrolled to the end: the last line must be rendered.
        let mut lines = vec![];
        for i in 0..40 {
            lines.push(format!("line {i:02}"));
        }
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let track = make_track(&refs);
        let out = render(&track, 39, 0);
        assert!(
            out.contains("line 39"),
            "current line rendered after scroll"
        );
    }
}
