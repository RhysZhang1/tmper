use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::ui::theme::Theme;
use crate::visualizer::render::{self, CharSet};

pub fn render_visualizer(f: &mut Frame, area: Rect, theme: &Theme, data: &[f32]) {
    // The visualizer palette comes from `visualizer::render::bar_color`; the
    // theme is threaded through for API consistency across all panels.
    let _ = theme;
    if data.is_empty() {
        return;
    }

    let char_set = CharSet::Blocks;
    let lines = render::render_bars(data, area.width, area.height, &char_set);

    let num_bars = data.len();
    let rat_lines: Vec<Line> = lines
        .iter()
        .map(|text| {
            let spans: Vec<Span> = text
                .chars()
                .enumerate()
                .map(|(column, c)| {
                    // `column` counts screen columns from the left edge, which
                    // includes the centring padding — asking render for the bar
                    // at that column keeps the two in step. Dividing the column
                    // by the bar width here instead was a second, divergent
                    // implementation of the layout.
                    let color = render::bar_at_column(column, num_bars, area.width)
                        .map_or(Color::Reset, |bar| render::bar_color(bar, num_bars));
                    Span::styled(c.to_string(), Style::default().fg(color))
                })
                .collect();
            Line::from(spans)
        })
        .collect();

    let para = Paragraph::new(rat_lines);
    f.render_widget(para, area);
}
