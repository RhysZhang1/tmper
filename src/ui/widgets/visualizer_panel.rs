use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::ui::theme::Theme;
use crate::visualizer::render::{self, CharSet};

pub fn render_visualizer(f: &mut Frame, area: Rect, theme: &Theme, data: &[f32]) {
    if data.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }

    let char_set = CharSet::Blocks;
    let bars = render::fit_bars(data, area.width);
    let lines = render::render_bars(&bars, area.width, area.height, &char_set);
    let columns = render::column_map(bars.len(), area.width);
    let palette = render::SpectrumPalette {
        low: theme.primary,
        mid: theme.control,
        high: theme.accent,
        highlight: theme.text,
    };

    let rat_lines: Vec<Line> = lines
        .iter()
        .enumerate()
        .map(|(row, text)| {
            let row_from_bottom = area.height as usize - 1 - row;
            let vertical_position = (row_from_bottom + 1) as f32 / area.height as f32;
            let spans: Vec<Span> = text
                .chars()
                .enumerate()
                .map(|(j, c)| {
                    let Some(bar_idx) = columns.get(j).copied().flatten() else {
                        return Span::raw(" ");
                    };
                    let scaled = bars[bar_idx].clamp(0.0, 1.0) * area.height as f32;
                    let top_row = scaled.ceil().max(1.0) as usize - 1;
                    let is_tip = c != ' ' && row_from_bottom == top_row;
                    let color =
                        render::bar_color(bar_idx, bars.len(), vertical_position, is_tip, palette);
                    let style = if is_tip {
                        Style::default().fg(color).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(color)
                    };
                    Span::styled(c.to_string(), style)
                })
                .collect();
            Line::from(spans)
        })
        .collect();

    let para = Paragraph::new(rat_lines);
    f.render_widget(para, area);
}
