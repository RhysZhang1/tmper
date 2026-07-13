use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::visualizer::render::{self, CharSet};

pub fn render_visualizer(f: &mut Frame, area: Rect, bars: &[f32]) {
    if bars.is_empty() {
        return;
    }

    let char_set = CharSet::Blocks;
    let lines = render::render_bars(bars, area.width, area.height, &char_set);

    let bw = render::bar_width(bars, area.width);
    let num_bars = bars.len();
    let rat_lines: Vec<Line> = lines
        .iter()
        .map(|text| {
            let spans: Vec<Span> = text
                .chars()
                .enumerate()
                .map(|(j, c)| {
                    let bar_idx = j / bw;
                    let color = render::bar_color(bar_idx, num_bars);
                    Span::styled(c.to_string(), Style::default().fg(color))
                })
                .collect();
            Line::from(spans)
        })
        .collect();

    let para = Paragraph::new(rat_lines);
    f.render_widget(para, area);
}
