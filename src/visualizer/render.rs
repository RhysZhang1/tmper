use ratatui::style::Color;

const BLOCKS: &[char] = &[' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum CharSet {
    Blocks,
    Braille,
    Ascii,
}

pub fn render_bars(bars: &[f32], width: u16, height: u16, char_set: &CharSet) -> Vec<String> {
    let chars = match char_set {
        CharSet::Blocks => BLOCKS,
        CharSet::Braille => &[' ', '⣀', '⣤', '⣶', '⣿'],
        CharSet::Ascii => &[' ', '.', '-', '~', '*', '#', '@'],
    };
    let levels = chars.len() - 1;

    let num_bars = bars.len().min(width as usize);
    let h = height as usize;
    if h == 0 {
        return vec![];
    }

    let mut rows = Vec::with_capacity(h);
    #[allow(clippy::needless_range_loop)]
    for row in 0..h {
        let row_from_bottom = h - 1 - row;
        let mut row_str = String::with_capacity(num_bars);
        #[allow(clippy::needless_range_loop)]
        for bar_idx in 0..num_bars {
            let val = bars[bar_idx].clamp(0.0, 1.0);
            let filled_rows = (val * h as f32) as usize;
            if row_from_bottom < filled_rows {
                row_str.push(chars[levels]);
            } else if row_from_bottom == filled_rows && filled_rows < h {
                let remainder = val * h as f32 - filled_rows as f32;
                let char_idx = (remainder * levels as f32).round() as usize;
                row_str.push(chars[char_idx.min(levels)]);
            } else {
                row_str.push(' ');
            }
        }
        rows.push(row_str);
    }

    rows
}

pub fn bar_color(index: usize, total: usize) -> Color {
    let ratio = index as f32 / total.max(1) as f32;

    if ratio < 0.5 {
        let t = ratio / 0.5;
        let r = (t * 255.0) as u8;
        let g = 255u8;
        Color::Rgb(r, g, 0)
    } else {
        let t = (ratio - 0.5) / 0.5;
        let r = 255u8;
        let g = (255.0 * (1.0 - t)) as u8;
        Color::Rgb(r, g, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_bars_output() {
        let bars = vec![0.0, 0.5, 1.0];
        let lines = render_bars(&bars, 3, 4, &CharSet::Blocks);
        assert!(!lines.is_empty(), "Should have some lines");
    }

    #[test]
    fn test_bar_color_gradient() {
        let low = bar_color(0, 10);
        let high = bar_color(9, 10);
        assert_ne!(low, high);
    }
}
