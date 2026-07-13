use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::lyrics::types::LyricTrack;

pub fn render_lyrics_view(
    f: &mut Frame,
    area: Rect,
    track: &LyricTrack,
    current_line_index: usize,
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
                let dist = current_line_index.saturating_sub(i) as f32;
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
