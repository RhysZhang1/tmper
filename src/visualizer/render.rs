use ratatui::style::Color;

const BLOCKS: &[char] = &[' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

#[derive(Debug, Clone)]
pub enum CharSet {
    Blocks,
}

/// Horizontal placement of the bars within a row.
///
/// `render_bars` and the panel that colours its output must agree on this
/// exactly; when each derived it separately the colours were offset by
/// `left_pad` columns and drifted a little further with every bar.
pub struct BarLayout {
    pub cols_per_bar: usize,
    pub left_pad: usize,
}

pub fn bar_layout(num_bars: usize, width: u16) -> BarLayout {
    let w = width as usize;
    if num_bars == 0 {
        return BarLayout {
            cols_per_bar: 1,
            left_pad: 0,
        };
    }
    let cols_per_bar = (w / num_bars).max(1);
    let used = cols_per_bar * num_bars;
    BarLayout {
        cols_per_bar,
        left_pad: w.saturating_sub(used) / 2,
    }
}

/// Which bar occupies screen column `column`, or `None` for the padding that
/// centres the bars (and for anything past the last bar).
pub fn bar_at_column(column: usize, num_bars: usize, width: u16) -> Option<usize> {
    let layout = bar_layout(num_bars, width);
    let bar = column.checked_sub(layout.left_pad)? / layout.cols_per_bar;
    (bar < num_bars).then_some(bar)
}

pub fn render_bars(bars: &[f32], width: u16, height: u16, char_set: &CharSet) -> Vec<String> {
    let chars = match char_set {
        CharSet::Blocks => BLOCKS,
    };
    let levels = chars.len() - 1;

    let num_bars = bars.len();
    let h = height as usize;
    let w = width as usize;
    if h == 0 || num_bars == 0 {
        return vec![];
    }

    let layout = bar_layout(num_bars, width);
    let cols_per_bar = layout.cols_per_bar;

    let mut rows = Vec::with_capacity(h);
    for row in 0..h {
        let row_from_bottom = h - 1 - row;
        // Columns are counted explicitly rather than derived from the string
        // length: block characters are three bytes wide in UTF-8, so a
        // byte-length comparison both over-counted and let rows run past
        // `width` when `num_bars > width`.
        let mut columns = 0usize;
        let mut row_str = String::with_capacity(w * 3);

        for _ in 0..layout.left_pad.min(w) {
            row_str.push(' ');
            columns += 1;
        }

        'bars: for &val in bars.iter().take(num_bars) {
            let val = val.clamp(0.0, 1.0);
            let filled_rows = (val * h as f32) as usize;
            let ch = if row_from_bottom < filled_rows {
                chars[levels]
            } else if row_from_bottom == filled_rows && filled_rows < h {
                let remainder = val * h as f32 - filled_rows as f32;
                let char_idx = (remainder * levels as f32).round() as usize;
                chars[char_idx.min(levels)]
            } else {
                ' '
            };
            for _ in 0..cols_per_bar {
                if columns >= w {
                    break 'bars;
                }
                row_str.push(ch);
                columns += 1;
            }
        }

        while columns < w {
            row_str.push(' ');
            columns += 1;
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

    /// Every row must be exactly `width` screen columns. The padding loop used
    /// to compare `String::len()` — bytes — against a column count, which block
    /// characters (three bytes each) satisfied at once, so rows were short by
    /// the trailing space and could overflow when `num_bars > width`.
    #[test]
    fn rows_are_exactly_the_requested_width_in_columns() {
        for (num_bars, width) in [(3usize, 10u16), (32, 100), (64, 100), (10, 3), (1, 1)] {
            let bars = vec![0.5f32; num_bars];
            for row in render_bars(&bars, width, 4, &CharSet::Blocks) {
                assert_eq!(
                    row.chars().count(),
                    width as usize,
                    "num_bars={num_bars} width={width}"
                );
            }
        }
    }

    /// More bars than columns: the row is clipped rather than running long.
    #[test]
    fn more_bars_than_columns_is_clipped() {
        let bars = vec![0.5f32; 20];
        let rows = render_bars(&bars, 5, 3, &CharSet::Blocks);
        assert_eq!(rows.len(), 3);
        for row in rows {
            assert_eq!(row.chars().count(), 5);
        }
    }

    /// The colour lookup must skip the centring padding and then advance one
    /// bar per `cols_per_bar` columns.
    #[test]
    fn bar_at_column_accounts_for_the_centring_padding() {
        // 100 columns, 32 bars → 3 columns per bar, 2 columns of padding.
        let layout = bar_layout(32, 100);
        assert_eq!(layout.cols_per_bar, 3);
        assert_eq!(layout.left_pad, 2);

        assert_eq!(bar_at_column(0, 32, 100), None, "padding column");
        assert_eq!(bar_at_column(1, 32, 100), None, "padding column");
        assert_eq!(bar_at_column(2, 32, 100), Some(0), "first bar starts here");
        assert_eq!(bar_at_column(4, 32, 100), Some(0), "still the first bar");
        assert_eq!(bar_at_column(5, 32, 100), Some(1), "second bar");
        // Past the last bar there is no colour to apply.
        assert_eq!(bar_at_column(97, 32, 100), Some(31));
        assert_eq!(bar_at_column(100, 32, 100), None);
    }

    /// The layout the renderer uses and the layout the painter asks about must
    /// be the same one — this is what was wrong.
    #[test]
    fn painted_columns_line_up_with_drawn_bars() {
        let num_bars = 8;
        let width = 20u16; // 2 columns per bar, 2 of padding
        let bars = vec![1.0f32; num_bars];
        let rows = render_bars(&bars, width, 2, &CharSet::Blocks);
        let top_row: Vec<char> = rows[0].chars().collect();

        for column in 0..width as usize {
            match bar_at_column(column, num_bars, width) {
                // A full-height bar draws the solid block.
                Some(_) => assert_eq!(top_row[column], '█', "column {column}"),
                None => assert_eq!(top_row[column], ' ', "column {column}"),
            }
        }
    }
}
