use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::lyrics::types::LyricTrack;

pub fn render_lyrics(f: &mut Frame, area: Rect, track: &LyricTrack, current_line_index: usize) {
    let visible_lines = area.height as usize;
    if visible_lines < 2 {
        return;
    }

    let half = visible_lines / 2;
    let total = track.lines.len();

    // Calculate visible range centered on current line
    let start = if current_line_index > half {
        current_line_index.saturating_sub(half)
    } else {
        0
    };
    let end = (start + visible_lines).min(total);

    let lines: Vec<Line> = (start..end)
        .map(|i| {
            let lyric = &track.lines[i];
            let is_current = i == current_line_index;
            let is_past = i < current_line_index;

            let style = if is_current {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if is_past {
                let fade = (current_line_index - i) as f32 / half.max(1) as f32;
                let gray = (128.0 * (1.0 - fade.min(1.0))) as u8;
                Style::default().fg(Color::Rgb(gray, gray, gray))
            } else {
                Style::default().fg(Color::Gray)
            };

            Line::from(Span::styled(lyric.text.clone(), style))
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, area);
}

#[allow(dead_code)]
pub fn render_no_lyrics(f: &mut Frame, area: Rect) {
    let text = Line::from(Span::styled(
        "No lyrics found",
        Style::default().fg(Color::DarkGray),
    ));
    let para = Paragraph::new(text);
    f.render_widget(para, area);
}
