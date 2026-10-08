use ratatui::style::Color;

const BLOCKS: &[char] = &[' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

#[derive(Debug, Clone)]
pub enum CharSet {
    Blocks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarLayout {
    pub bar_width: usize,
    pub gap: usize,
    pub left_pad: usize,
    pub right_pad: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct SpectrumPalette {
    pub low: Color,
    pub mid: Color,
    pub high: Color,
    pub highlight: Color,
}

/// Keep the configured number of bars whenever the terminal has at least one
/// column per bar. Max-pooling is only used when the terminal is physically
/// too narrow to represent every configured frequency band.
pub fn fit_bars(bars: &[f32], width: u16) -> Vec<f32> {
    let target = bars.len().min(width as usize);
    if target == 0 {
        return Vec::new();
    }
    if target == bars.len() {
        return bars.to_vec();
    }

    (0..target)
        .map(|i| {
            let start = i * bars.len() / target;
            let end = ((i + 1) * bars.len() / target).max(start + 1);
            bars[start..end].iter().copied().fold(0.0_f32, f32::max)
        })
        .collect()
}

/// Pick a uniform layout without consuming whole columns for gutters. The
/// renderer creates a much narrower fractional-cell separation instead.
pub fn bar_layout(num_bars: usize, width: u16) -> BarLayout {
    let w = width as usize;
    if num_bars == 0 || w == 0 {
        return BarLayout {
            bar_width: 1,
            gap: 0,
            left_pad: 0,
            right_pad: w,
        };
    }

    let gap = 0;
    let gutters = gap * num_bars.saturating_sub(1);
    let bar_width = ((w.saturating_sub(gutters)) / num_bars).max(1);
    let used = bar_width * num_bars + gutters;
    let remaining = w.saturating_sub(used);

    BarLayout {
        bar_width,
        gap,
        left_pad: remaining / 2,
        right_pad: remaining - remaining / 2,
    }
}

/// Map each terminal column back to a spectrum bar. Gutter and padding columns
/// are `None`; the result always has exactly `width` entries.
pub fn column_map(num_bars: usize, width: u16) -> Vec<Option<usize>> {
    let layout = bar_layout(num_bars, width);
    let mut columns = Vec::with_capacity(width as usize);
    columns.extend(std::iter::repeat_n(None, layout.left_pad));
    for bar in 0..num_bars {
        columns.extend(std::iter::repeat_n(Some(bar), layout.bar_width));
        if bar + 1 < num_bars {
            columns.extend(std::iter::repeat_n(None, layout.gap));
        }
    }
    columns.extend(std::iter::repeat_n(None, layout.right_pad));
    columns.resize(width as usize, None);
    columns
}

pub fn render_bars(bars: &[f32], width: u16, height: u16, char_set: &CharSet) -> Vec<String> {
    let chars = match char_set {
        CharSet::Blocks => BLOCKS,
    };
    let levels = chars.len() - 1;
    let bars = fit_bars(bars, width);
    let h = height as usize;
    if h == 0 || bars.is_empty() {
        return vec![];
    }

    let columns = column_map(bars.len(), width);
    (0..h)
        .map(|row| {
            let row_from_bottom = h - 1 - row;
            columns
                .iter()
                .enumerate()
                .map(|(column, bar_idx)| {
                    let Some(&val) = bar_idx.and_then(|i| bars.get(i)) else {
                        return ' ';
                    };
                    let scaled = val.clamp(0.0, 1.0) * h as f32;
                    let filled_rows = scaled.floor() as usize;
                    if row_from_bottom < filled_rows {
                        let next_bar = columns.get(column + 1).copied().flatten();
                        if next_bar != *bar_idx {
                            // Leave only one eighth of a terminal cell at the
                            // right edge: visibly separated, without a wide
                            // one-column gutter or fewer spectrum bands.
                            '▉'
                        } else {
                            chars[levels]
                        }
                    } else if row_from_bottom == filled_rows && filled_rows < h {
                        // Ceil ensures that every non-zero signal gets at least
                        // a one-eighth block instead of flickering to blank.
                        let char_idx = ((scaled - filled_rows as f32) * levels as f32).ceil();
                        chars[(char_idx as usize).min(levels)]
                    } else {
                        ' '
                    }
                })
                .collect()
        })
        .collect()
}

fn rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::White => (255, 255, 255),
        _ => (180, 180, 180),
    }
}

fn mix(a: Color, b: Color, amount: f32) -> Color {
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    let t = amount.clamp(0.0, 1.0);
    let channel = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color::Rgb(channel(ar, br), channel(ag, bg), channel(ab, bb))
}

fn rgb_to_hsl(color: Color) -> (f32, f32, f32) {
    let (r, g, b) = rgb(color);
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lightness = (max + min) / 2.0;
    let delta = max - min;
    if delta <= f32::EPSILON {
        return (0.0, 0.0, lightness);
    }

    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
    let hue = if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    (hue, saturation, lightness)
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> Color {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let h = hue.rem_euclid(360.0) / 60.0;
    let x = chroma * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as usize {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = lightness - chroma / 2.0;
    let channel = |value: f32| ((value + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    Color::Rgb(channel(r), channel(g), channel(b))
}

/// Interpolate along the shortest hue path, with smooth easing at each color
/// stop. This stays vivid where direct RGB interpolation tends to turn muddy.
fn mix_hsl(a: Color, b: Color, amount: f32) -> Color {
    let (ah, as_, al) = rgb_to_hsl(a);
    let (bh, bs, bl) = rgb_to_hsl(b);
    let t = amount.clamp(0.0, 1.0);
    let eased = t * t * (3.0 - 2.0 * t);
    let hue_delta = (bh - ah + 180.0).rem_euclid(360.0) - 180.0;
    hsl_to_rgb(
        ah + hue_delta * eased,
        as_ + (bs - as_) * eased,
        al + (bl - al) * eased,
    )
}

fn shade(color: Color, amount: f32) -> Color {
    let (h, s, l) = rgb_to_hsl(color);
    hsl_to_rgb(h, s, (l * amount).clamp(0.0, 1.0))
}

/// Theme-aware frequency gradient with vertical depth. The optional tip boost
/// creates a crisp highlight without requiring a stateful peak-hold widget.
pub fn bar_color(
    index: usize,
    total: usize,
    vertical_position: f32,
    is_tip: bool,
    palette: SpectrumPalette,
) -> Color {
    let ratio = if total <= 1 {
        0.0
    } else {
        index as f32 / (total - 1) as f32
    };
    let base = if ratio < 0.5 {
        mix_hsl(palette.low, palette.mid, ratio * 2.0)
    } else {
        mix_hsl(palette.mid, palette.high, (ratio - 0.5) * 2.0)
    };
    let depth = 0.72 + vertical_position.clamp(0.0, 1.0) * 0.28;
    let shaded = shade(base, depth);
    if is_tip {
        mix(shaded, palette.highlight, 0.14)
    } else {
        shaded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_has_exact_terminal_dimensions() {
        let bars = vec![0.0, 0.5, 1.0];
        let lines = render_bars(&bars, 11, 4, &CharSet::Blocks);
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().all(|line| line.chars().count() == 11));
    }

    #[test]
    fn layout_keeps_configured_bar_count_without_full_cell_gutters() {
        let layout = bar_layout(4, 12);
        assert_eq!(layout.gap, 0);
        assert_eq!(layout.bar_width, 3);
        assert_eq!(column_map(4, 12).len(), 12);
    }

    #[test]
    fn full_bars_get_a_fractional_cell_separation() {
        let layout = bar_layout(3, 8);
        assert_eq!(layout.gap, 0);
        assert_eq!(layout.bar_width, 2);

        let lines = render_bars(&[1.0, 1.0], 4, 1, &CharSet::Blocks);
        assert_eq!(lines[0], "█▉█▉");
    }

    #[test]
    fn narrow_output_pools_instead_of_cropping() {
        let bars = vec![0.1, 0.2, 0.3, 0.4, 0.9, 0.5];
        let fitted = fit_bars(&bars, 3);
        assert_eq!(fitted, vec![0.2, 0.4, 0.9]);
    }

    #[test]
    fn tiny_nonzero_value_remains_visible() {
        let lines = render_bars(&[0.01], 1, 1, &CharSet::Blocks);
        assert_eq!(lines[0], "▁");
    }

    #[test]
    fn bar_color_uses_gradient_and_tip_highlight() {
        let low = Color::Rgb(20, 40, 80);
        let mid = Color::Rgb(80, 40, 120);
        let high = Color::Rgb(200, 80, 40);
        let palette = SpectrumPalette {
            low,
            mid,
            high,
            highlight: Color::White,
        };
        let a = bar_color(0, 10, 1.0, false, palette);
        let b = bar_color(9, 10, 1.0, false, palette);
        let tip = bar_color(0, 10, 1.0, true, palette);
        assert_ne!(a, b);
        assert_ne!(a, tip);
    }
}
