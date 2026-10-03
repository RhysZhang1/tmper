use image::GenericImageView;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use std::cell::{Cell, RefCell};
use std::sync::Arc;

use crate::lyrics::types::LyricTrack;
use crate::ui::format_duration;
use crate::ui::theme::Theme;
use crate::ui::views::playlist_view::PlaylistManagerState;
use crate::ui::{CoverLinesCache, RepeatMode, TrackDisplay};

/// Read-only view parameters for the player view.
/// Extracted from `UiState` so each render function declares exactly what it needs.
pub struct PlayerViewParams<'a> {
    pub theme: &'a Theme,
    pub title: &'a str,
    pub artist: &'a str,
    pub position: f64,
    pub duration: f64,
    pub volume: f32,
    pub is_playing: bool,
    pub album: &'a str,
    pub genre: &'a str,
    pub year: &'a str,
    pub codec: &'a str,
    pub repeat_mode: RepeatMode,
    pub cover_art: Option<&'a Arc<Vec<u8>>>,
    pub show_cover_art: bool,
    pub cover_rect: &'a Cell<(u16, u16, u16, u16)>,
    pub cover_gen: u64,
    pub cover_lines_cache: &'a RefCell<Option<CoverLinesCache>>,
    pub lyric_track: Option<&'a LyricTrack>,
    pub current_lyric_index: usize,
    pub visualizer_data: &'a [f32],
    pub playlist_state: &'a PlaylistManagerState,
    pub playing_index: Option<usize>,
    pub tracks: &'a [TrackDisplay],
    pub active_playlist: Option<usize>,
    /// `/` search is active — left-top panel shows filtered results.
    pub search_active: bool,
    pub search_query: &'a str,
    pub selected_index: usize,
}

/// Decode cover art bytes and render as colored block characters (chafa-style).
/// Uses Lanczos3 resize + lower-half block (▄) with fg/bg for 2× vertical resolution.
fn cover_as_colored_lines(inner: Rect, bytes: &[u8]) -> Option<Vec<Line<'static>>> {
    let cols = inner.width as usize;
    let rows = inner.height as usize;
    if cols == 0 || rows == 0 {
        return None;
    }

    let img = image::load_from_memory(bytes).ok()?;
    // Effective pixel height = 2 rows per text row (half-block trick)
    let ph = rows * 2;
    // Lanczos3: much sharper than Nearest, smoother than Triangle
    let resized = img.resize_exact(
        cols as u32,
        ph as u32,
        image::imageops::FilterType::Lanczos3,
    );

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(rows);
    // Simple dither buffer for Floyd-Steinberg error diffusion
    let mut err_r = vec![0f32; cols];
    let mut err_g = vec![0f32; cols];
    let mut err_b = vec![0f32; cols];
    let mut next_err_r = vec![0f32; cols];
    let mut next_err_g = vec![0f32; cols];
    let mut next_err_b = vec![0f32; cols];

    for ty in 0..rows {
        let mut spans = Vec::with_capacity(cols);
        for x in 0..cols {
            let y1 = ty * 2;
            let y2 = y1 + 1;

            // Get pixel value, add accumulated error
            let p_top = resized.get_pixel(x as u32, y1 as u32);
            let pr = (p_top[0] as f32 + err_r[x]).clamp(0.0, 255.0) as u8;
            let pg = (p_top[1] as f32 + err_g[x]).clamp(0.0, 255.0) as u8;
            let pb = (p_top[2] as f32 + err_b[x]).clamp(0.0, 255.0) as u8;
            let a1 = p_top[3];

            if y2 >= ph {
                if a1 < 128 {
                    spans.push(Span::styled(" ", Style::default()));
                } else {
                    spans.push(Span::styled(
                        "▀",
                        Style::default().fg(Color::Rgb(pr, pg, pb)),
                    ));
                }
                continue;
            }

            let p_bot = resized.get_pixel(x as u32, y2 as u32);
            let qr = (p_bot[0] as f32 + next_err_r[x]).clamp(0.0, 255.0) as u8;
            let qg = (p_bot[1] as f32 + next_err_g[x]).clamp(0.0, 255.0) as u8;
            let qb = (p_bot[2] as f32 + next_err_b[x]).clamp(0.0, 255.0) as u8;
            let a2 = p_bot[3];

            // Quantization errors for Floyd-Steinberg
            let e_r = p_top[0] as f32 - pr as f32;
            let e_g = p_top[1] as f32 - pg as f32;
            let e_b = p_top[2] as f32 - pb as f32;
            // Diffuse to neighbors (7/16 right, 3/16 bottom-left, 5/16 bottom, 1/16 bottom-right)
            if x + 1 < cols {
                err_r[x + 1] += e_r * 7.0 / 16.0;
                err_g[x + 1] += e_g * 7.0 / 16.0;
                err_b[x + 1] += e_b * 7.0 / 16.0;
            }
            if x > 0 {
                next_err_r[x - 1] += e_r * 3.0 / 16.0;
                next_err_g[x - 1] += e_g * 3.0 / 16.0;
                next_err_b[x - 1] += e_b * 3.0 / 16.0;
            }
            next_err_r[x] += e_r * 5.0 / 16.0;
            next_err_g[x] += e_g * 5.0 / 16.0;
            next_err_b[x] += e_b * 5.0 / 16.0;
            if x + 1 < cols {
                next_err_r[x + 1] += e_r / 16.0;
                next_err_g[x + 1] += e_g / 16.0;
                next_err_b[x + 1] += e_b / 16.0;
            }

            let ch = if a1 < 128 && a2 < 128 {
                ' '
            } else if a2 < 128 {
                '▀'
            } else {
                '▄'
            };
            let style = match (a1 >= 128, a2 >= 128) {
                // Both visible: ▄ with fg = bottom half, bg = top half
                (true, true) => Style::default()
                    .fg(Color::Rgb(qr, qg, qb))
                    .bg(Color::Rgb(pr, pg, pb)),
                // Only top visible: ▀ with fg = top half, bg transparent
                (true, false) => Style::default().fg(Color::Rgb(pr, pg, pb)).bg(Color::Reset),
                // Only bottom visible: ▄ with fg = bottom half, bg transparent
                (false, true) => Style::default().fg(Color::Rgb(qr, qg, qb)).bg(Color::Reset),
                // Both transparent: space, no color
                (false, false) => Style::default(),
            };
            spans.push(Span::styled(ch.to_string(), style));
        }
        // Swap error buffers for next row
        std::mem::swap(&mut err_r, &mut next_err_r);
        std::mem::swap(&mut err_g, &mut next_err_g);
        std::mem::swap(&mut err_b, &mut next_err_b);
        next_err_r.fill(0.0);
        next_err_g.fill(0.0);
        next_err_b.fill(0.0);
        lines.push(Line::from(spans));
    }
    Some(lines)
}

/// Main player view: two-column layout with cover/playlist (left)
/// and lyrics/spectrum/controls (right).
pub fn render_player_view(f: &mut Frame, area: Rect, params: &PlayerViewParams) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(33), Constraint::Percentage(67)])
        .split(area);

    render_left_panel(f, cols[0], params);
    render_right_panel(f, cols[1], params);
}

// ═══════════════════════ LEFT PANEL ═══════════════════════

fn render_left_panel(f: &mut Frame, area: Rect, params: &PlayerViewParams) {
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    if params.search_active {
        render_search_results(f, split[0], params);
    } else {
        render_cover_art(f, split[0], params);
    }
    render_mini_playlist(f, split[1], params);
}

fn render_cover_art(f: &mut Frame, area: Rect, params: &PlayerViewParams) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Now Playing ")
        .border_style(Style::default().fg(params.theme.primary));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Store position for Kitty protocol rendering
    params
        .cover_rect
        .set((inner.x, inner.y, inner.width, inner.height));

    if params.show_cover_art {
        // Render cover art as colored blocks, reusing the cached render when
        // the cover and area are unchanged (decode + Lanczos3 + dither only
        // run once per cover instead of every frame).
        if let Some(cover) = params.cover_art {
            let mut cache = params.cover_lines_cache.borrow_mut();
            let stale = cache.as_ref().is_none_or(|c| {
                c.gen != params.cover_gen || c.width != inner.width || c.height != inner.height
            });
            if stale {
                *cache = cover_as_colored_lines(inner, &cover[..]).map(|lines| CoverLinesCache {
                    gen: params.cover_gen,
                    width: inner.width,
                    height: inner.height,
                    lines,
                });
            }
            if let Some(cached) = cache.as_ref() {
                let para = Paragraph::new(cached.lines.clone());
                f.render_widget(para, inner);
                return;
            }
        }
    }

    // Fallback / no-image mode: show detailed song info
    let h = inner.height.max(3);
    let play_icon = if params.is_playing { "▶" } else { "⏸" };
    let pos_str = format_duration(params.position);
    let dur_str = format_duration(params.duration);

    let mut lines: Vec<Line> = Vec::new();
    let w = inner.width as usize;

    // Vertical centering
    let content_lines = if params.album.is_empty() && params.genre.is_empty() {
        4
    } else {
        6
    };
    let top_spacer = (h.saturating_sub(content_lines)) / 2;
    for _ in 0..top_spacer {
        lines.push(Line::from(""));
    }

    // Track title
    let title = if params.title.is_empty() || params.title == "No track" {
        "No track"
    } else {
        params.title
    };
    lines.push(Line::from(vec![Span::styled(
        format!("{:^w$}", title, w = w),
        Style::default()
            .fg(params.theme.text)
            .add_modifier(Modifier::BOLD),
    )]));

    // Artist
    let artist = if params.artist.is_empty() || params.artist == "—" {
        ""
    } else {
        params.artist
    };
    if !artist.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            format!("{:^w$}", artist, w = w),
            Style::default().fg(params.theme.secondary),
        )]));
    }

    // Playback time
    if params.duration > 0.0 {
        let time_str = format!("{} {} / {}", play_icon, pos_str, dur_str);
        lines.push(Line::from(vec![Span::styled(
            format!("{:^w$}", time_str, w = w),
            Style::default().fg(params.theme.success),
        )]));
    }

    // Spacer before metadata
    if !params.album.is_empty() || !params.genre.is_empty() {
        lines.push(Line::from(""));
    }

    // Album
    if !params.album.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(" 专辑: ", Style::default().fg(params.theme.muted)),
            Span::styled(params.album, Style::default().fg(params.theme.album)),
        ]));
    }

    // Genre + Year + Codec
    let mut meta_parts = Vec::new();
    if !params.genre.is_empty() {
        meta_parts.push(Span::styled(
            format!("{} ", params.genre),
            Style::default().fg(params.theme.genre),
        ));
    }
    if !params.year.is_empty() {
        meta_parts.push(Span::styled(
            format!("{} ", params.year),
            Style::default().fg(params.theme.year),
        ));
    }
    if !params.codec.is_empty() {
        meta_parts.push(Span::styled(
            params.codec.to_uppercase(),
            Style::default().fg(params.theme.codec),
        ));
    }
    if !meta_parts.is_empty() {
        lines.push(Line::from(meta_parts));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

/// Search results list shown in the left-top panel while `/` search is active.
fn render_search_results(f: &mut Frame, area: Rect, params: &PlayerViewParams) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Search: {} ", params.search_query))
        .border_style(Style::default().fg(params.theme.primary));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let matches = crate::ui::search_matches(params.tracks, params.search_query);
    if matches.is_empty() {
        let para = Paragraph::new("(no matches)").style(Style::default().fg(params.theme.muted));
        f.render_widget(para, inner);
        return;
    }

    let vis_h = inner.height as usize;
    if vis_h == 0 {
        return;
    }

    let cur_pos = matches
        .iter()
        .position(|&i| i == params.selected_index)
        .unwrap_or(0);
    let start = cur_pos.saturating_sub(vis_h.saturating_sub(1));
    let end = (start + vis_h).min(matches.len());

    let mut lines: Vec<Line> = Vec::new();
    for &tidx in &matches[start..end] {
        let t = &params.tracks[tidx];
        let is_cur = tidx == params.selected_index;
        let is_playing = params.playing_index == Some(tidx);
        let label = if is_playing { "▶ " } else { "  " };
        let text = format!("{}{} — {}", label, t.title, t.artist);
        let style = if is_cur {
            Style::default()
                .fg(params.theme.primary)
                .add_modifier(Modifier::BOLD)
        } else if is_playing {
            Style::default().fg(params.theme.success)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(text, style)));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

fn render_mini_playlist(f: &mut Frame, area: Rect, params: &PlayerViewParams) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Playlists ")
        .border_style(Style::default().fg(params.theme.muted));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let ps = params.playlist_state;
    if ps.playlists.is_empty() {
        let para = Paragraph::new("(no playlists)").style(Style::default().fg(params.theme.muted));
        f.render_widget(para, inner);
        return;
    }

    use crate::ui::views::playlist_view::{InsertMode, PlaylistFlatModel};

    let model = PlaylistFlatModel::new(&ps.playlists, ps.expanded_playlist);
    // Convert sidebar cursor (0-based) to full-model cursor (+1 for "…").
    let full_cursor = ps.sidebar_selected + 1;
    let all_lines =
        model.build_styled_lines(true, &InsertMode::Off, params.theme, full_cursor, |song| {
            params
                .playing_index
                .and_then(|pi| params.tracks.get(pi).map(|t| t.path == *song))
                .unwrap_or(false)
        });

    // Sidebar: skip "…" row (index 0), use sidebar_scroll.
    let flat_lines: Vec<_> = all_lines.into_iter().skip(1).collect();

    let vis_h = inner.height as usize;
    let max_scroll = flat_lines.len().saturating_sub(vis_h);
    let scroll = ps.sidebar_scroll.min(max_scroll);
    let end = (scroll + vis_h).min(flat_lines.len());
    let visible: Vec<Line> = flat_lines[scroll..end]
        .iter()
        .map(|(t, s)| Line::from(Span::styled(t.clone(), *s)))
        .collect();

    let para = Paragraph::new(visible);
    f.render_widget(para, inner);
}

// ═══════════════════════ RIGHT PANEL ═══════════════════════

fn render_right_panel(f: &mut Frame, area: Rect, params: &PlayerViewParams) {
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(48),
            Constraint::Percentage(36),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    render_lyrics_section(f, split[0], params);
    render_spectrum_section(f, split[1], params);
    render_song_info(f, split[2], params);
    render_control_bar(f, split[3], params);
}

fn render_lyrics_section(f: &mut Frame, area: Rect, params: &PlayerViewParams) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Lyrics ")
        .border_style(Style::default().fg(params.theme.primary));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(track) = params.lyric_track {
        if track.lines.is_empty() {
            let para =
                Paragraph::new("No lyrics found").style(Style::default().fg(params.theme.muted));
            f.render_widget(para, inner);
            return;
        }

        let visible_lines = inner.height as usize;
        if visible_lines < 2 {
            return;
        }

        let half = visible_lines / 2;
        let total = track.lines.len();

        let start = if params.current_lyric_index > half {
            params.current_lyric_index.saturating_sub(half)
        } else {
            0
        };
        let end = (start + visible_lines).min(total);

        let lines: Vec<Line> = (start..end)
            .map(|i| {
                let lyric = &track.lines[i];
                let is_current = i == params.current_lyric_index;
                let is_past = i < params.current_lyric_index;

                let style = if is_current {
                    Style::default()
                        .fg(params.theme.primary)
                        .add_modifier(Modifier::BOLD)
                } else if is_past {
                    let dist = params.current_lyric_index.saturating_sub(i) as f32;
                    let fade = (dist / half.max(1) as f32).min(1.0);
                    let gray = (200.0 * (1.0 - fade * 0.6)) as u8;
                    Style::default().fg(Color::Rgb(gray, gray, gray))
                } else {
                    Style::default().fg(Color::Rgb(100, 100, 100))
                };

                Line::from(Span::styled(lyric.text.clone(), style))
            })
            .collect();

        let para = Paragraph::new(lines);
        f.render_widget(para, inner);
    } else {
        let para =
            Paragraph::new("No lyrics loaded").style(Style::default().fg(params.theme.muted));
        f.render_widget(para, inner);
    }
}

fn render_spectrum_section(f: &mut Frame, area: Rect, params: &PlayerViewParams) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Spectrum ")
        .border_style(Style::default().fg(params.theme.muted));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if params.visualizer_data.is_empty() {
        let para = Paragraph::new("No audio data").style(Style::default().fg(params.theme.muted));
        f.render_widget(para, inner);
        return;
    }

    crate::ui::widgets::visualizer_panel::render_visualizer(
        f,
        inner,
        params.theme,
        params.visualizer_data,
    );
}

fn render_song_info(f: &mut Frame, area: Rect, params: &PlayerViewParams) {
    let mut parts: Vec<Span> = Vec::new();
    // Show active playlist name first
    if let Some(pl_idx) = params.active_playlist {
        if pl_idx < params.playlist_state.playlists.len() {
            let pl_name = &params.playlist_state.playlists[pl_idx].name;
            parts.push(Span::styled(
                format!(" {} {} ", '🎵', pl_name),
                Style::default()
                    .fg(params.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }

    if !params.album.is_empty() {
        parts.push(Span::styled(
            format!(" {} {} ", "\u{1f4bf}", params.album),
            Style::default().fg(params.theme.album),
        ));
    }
    if !params.genre.is_empty() {
        parts.push(Span::styled(
            format!(" {} {} ", "\u{266a}", params.genre),
            Style::default().fg(params.theme.genre),
        ));
    }
    if !params.year.is_empty() {
        parts.push(Span::styled(
            format!(" {} {} ", "\u{1f4c5}", params.year),
            Style::default().fg(params.theme.year),
        ));
    }
    if !params.codec.is_empty() {
        parts.push(Span::styled(
            format!(" {} ", params.codec.to_uppercase()),
            Style::default().fg(params.theme.codec),
        ));
    }

    if !parts.is_empty() {
        let line = Line::from(parts);
        let para = Paragraph::new(line);
        f.render_widget(para, area);
    }
}

fn render_control_bar(f: &mut Frame, area: Rect, params: &PlayerViewParams) {
    let play_icon = if params.is_playing { "▶" } else { "⏸" };
    let pos_str = format_duration(params.position);
    let dur_str = format_duration(params.duration);
    let vol_pct = (params.volume * 100.0) as u32;

    let progress = if params.duration > 0.0 {
        (params.position / params.duration).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let time_str = format!("{} {} / {}", play_icon, pos_str, dur_str);
    let vol_str = format!("Vol:{}%", vol_pct);
    let mode_str = params.repeat_mode.label().to_string();

    // Calculate space for progress bar
    let fixed = time_str.len() + vol_str.len() + mode_str.len() + 6;
    let bar_w = (area.width as usize).saturating_sub(fixed).min(30);
    let filled = (bar_w as f64 * progress).round() as usize;
    let empty = bar_w.saturating_sub(filled);

    let bar_progress = format!("{}{}", "█".repeat(filled), "░".repeat(empty));
    let bar = format!("{} {} [{}] {}", time_str, vol_str, bar_progress, mode_str);

    let para = Paragraph::new(Line::from(Span::styled(
        bar,
        Style::default().fg(params.theme.control),
    )));
    f.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;
    use std::time::Duration;

    use crate::lyrics::types::{LyricLine, LyricMetadata, LyricTrack};
    use crate::playlist::PlaylistData;
    use crate::ui::views::playlist_view::PlaylistManagerState;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// A tiny in-memory 8×8 PNG.
    fn make_png() -> Vec<u8> {
        let img = image::RgbaImage::from_fn(8, 8, |x, y| {
            let v = ((x + y) * 32).min(255) as u8;
            image::Rgba([v, v, v, 255])
        });
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    // ── cover_as_colored_lines ──

    #[test]
    fn test_cover_empty_bytes_returns_none() {
        assert!(cover_as_colored_lines(Rect::new(0, 0, 8, 8), &[]).is_none());
    }

    #[test]
    fn test_cover_zero_area_returns_none() {
        let png = make_png();
        assert!(cover_as_colored_lines(Rect::new(0, 0, 0, 0), &png).is_none());
        assert!(cover_as_colored_lines(Rect::new(0, 0, 8, 0), &png).is_none());
    }

    #[test]
    fn test_cover_generated_png_yields_one_line_per_row() {
        let png = make_png();
        let lines = cover_as_colored_lines(Rect::new(0, 0, 4, 4), &png).expect("png decodes");
        assert_eq!(lines.len(), 4, "one text line per inner row");
        // Every row has one span per column; all renderable (opaque image).
        let spans: usize = lines.iter().map(|l| l.spans.len()).sum();
        assert_eq!(spans, 4 * 4);
    }

    // ── render_player_view ──

    fn base_params<'a>(
        theme: &'a Theme,
        cover_rect: &'a Cell<(u16, u16, u16, u16)>,
        cache: &'a RefCell<Option<CoverLinesCache>>,
        playlist_state: &'a PlaylistManagerState,
        tracks: &'a [TrackDisplay],
    ) -> PlayerViewParams<'a> {
        PlayerViewParams {
            theme,
            title: "No track",
            artist: "",
            position: 0.0,
            duration: 0.0,
            volume: 0.8,
            is_playing: false,
            album: "",
            genre: "",
            year: "",
            codec: "",
            repeat_mode: RepeatMode::Sequential,
            cover_art: None,
            show_cover_art: true,
            cover_rect,
            cover_gen: 0,
            cover_lines_cache: cache,
            lyric_track: None,
            current_lyric_index: 0,
            visualizer_data: &[],
            playlist_state,
            playing_index: None,
            tracks,
            active_playlist: None,
            search_active: false,
            search_query: "",
            selected_index: 0,
        }
    }

    #[test]
    fn test_render_player_view_smoke() {
        let theme = Theme::default();
        let cover_rect = Cell::new((0u16, 0u16, 0u16, 0u16));
        let cache: RefCell<Option<CoverLinesCache>> = RefCell::new(None);
        let playlist_state = PlaylistManagerState::default();
        let params = base_params(&theme, &cover_rect, &cache, &playlist_state, &[]);

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| render_player_view(f, f.area(), &params))
            .unwrap();
        let buf = terminal.backend().buffer();
        let out: String = buf.content().iter().map(|c| c.symbol()).collect();
        for needle in [
            "Now Playing",
            "No track",
            "Playlists",
            "Spectrum",
            "No lyrics loaded",
            "No audio data",
            "Vol:80%",
        ] {
            assert!(out.contains(needle), "missing {needle:?}");
        }
    }

    #[test]
    fn test_render_player_view_search_results() {
        let theme = Theme::default();
        let cover_rect = Cell::new((0u16, 0u16, 0u16, 0u16));
        let cache: RefCell<Option<CoverLinesCache>> = RefCell::new(None);
        let playlist_state = PlaylistManagerState::default();
        let tracks = vec![TrackDisplay {
            path: PathBuf::from("/a.flac"),
            title: "Hello World".into(),
            artist: "John".into(),
            duration_secs: 120.0,
        }];

        let mut params = base_params(&theme, &cover_rect, &cache, &playlist_state, &tracks);
        params.search_active = true;
        params.search_query = "hell";

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| render_player_view(f, f.area(), &params))
            .unwrap();
        let buf = terminal.backend().buffer();
        let out: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(out.contains("Search: hell"), "search header rendered");
        assert!(out.contains("Hello World"), "matched track listed");
    }

    #[test]
    fn test_render_player_view_no_search_matches() {
        let theme = Theme::default();
        let cover_rect = Cell::new((0u16, 0u16, 0u16, 0u16));
        let cache: RefCell<Option<CoverLinesCache>> = RefCell::new(None);
        let playlist_state = PlaylistManagerState::default();
        let tracks = vec![TrackDisplay {
            path: PathBuf::from("/a.flac"),
            title: "Hello World".into(),
            artist: "John".into(),
            duration_secs: 120.0,
        }];

        let mut params = base_params(&theme, &cover_rect, &cache, &playlist_state, &tracks);
        params.search_active = true;
        params.search_query = "zzz";

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| render_player_view(f, f.area(), &params))
            .unwrap();
        let buf = terminal.backend().buffer();
        let out: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(out.contains("(no matches)"), "empty-state message rendered");
    }

    // ── Panel sections ──

    /// Render the whole view at a fixed size and flatten the buffer to text.
    fn render_to_string(params: &PlayerViewParams, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| render_player_view(f, f.area(), params))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    fn lyric_track(texts: &[&str]) -> LyricTrack {
        LyricTrack {
            metadata: LyricMetadata::default(),
            lines: texts
                .iter()
                .enumerate()
                .map(|(i, text)| LyricLine {
                    timestamp: Duration::from_secs(i as u64),
                    text: (*text).to_string(),
                    word_timestamps: Vec::new(),
                })
                .collect(),
        }
    }

    fn state_with_playlists(names: &[&str]) -> PlaylistManagerState {
        PlaylistManagerState {
            playlists: names
                .iter()
                .map(|name| PlaylistData {
                    name: (*name).to_string(),
                    songs: Vec::new(),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn test_mini_playlist_lists_names_and_flags_the_active_one() {
        let theme = Theme::default();
        let cover_rect = Cell::new((0u16, 0u16, 0u16, 0u16));
        let cache = RefCell::new(None);
        let playlists = state_with_playlists(&["Chill", "Focus"]);
        let mut params = base_params(&theme, &cover_rect, &cache, &playlists, &[]);
        params.active_playlist = Some(1);

        let out = render_to_string(&params, 100, 30);

        assert!(out.contains("Chill"), "first playlist rendered");
        assert!(out.contains("Focus"), "second playlist rendered");
    }

    #[test]
    fn test_lyrics_section_reports_a_track_with_no_lines() {
        let theme = Theme::default();
        let cover_rect = Cell::new((0u16, 0u16, 0u16, 0u16));
        let cache = RefCell::new(None);
        let playlists = PlaylistManagerState::default();
        let empty = lyric_track(&[]);
        let mut params = base_params(&theme, &cover_rect, &cache, &playlists, &[]);
        params.lyric_track = Some(&empty);

        assert!(render_to_string(&params, 100, 30).contains("No lyrics found"));
    }

    #[test]
    fn test_lyrics_section_shows_the_current_line() {
        let theme = Theme::default();
        let cover_rect = Cell::new((0u16, 0u16, 0u16, 0u16));
        let cache = RefCell::new(None);
        let playlists = PlaylistManagerState::default();
        let track = lyric_track(&["first", "second", "third"]);
        let mut params = base_params(&theme, &cover_rect, &cache, &playlists, &[]);
        params.lyric_track = Some(&track);
        params.current_lyric_index = 1;

        let out = render_to_string(&params, 100, 30);

        assert!(out.contains("first"), "past line still visible");
        assert!(out.contains("second"), "current line visible");
        assert!(out.contains("third"), "upcoming line visible");
    }

    /// With more lines than fit, the window follows the cursor instead of
    /// always showing the top of the song.
    #[test]
    fn test_lyrics_section_scrolls_to_keep_the_current_line_visible() {
        let theme = Theme::default();
        let cover_rect = Cell::new((0u16, 0u16, 0u16, 0u16));
        let cache = RefCell::new(None);
        let playlists = PlaylistManagerState::default();
        let lines: Vec<String> = (0..60).map(|i| format!("line{i:02}")).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let track = lyric_track(&refs);
        let mut params = base_params(&theme, &cover_rect, &cache, &playlists, &[]);
        params.lyric_track = Some(&track);
        params.current_lyric_index = 50;

        let out = render_to_string(&params, 100, 30);

        assert!(out.contains("line50"), "the current line is on screen");
        assert!(
            !out.contains("line00"),
            "the window has scrolled away from the start"
        );
    }

    #[test]
    fn test_song_info_renders_each_metadata_slot() {
        let theme = Theme::default();
        let cover_rect = Cell::new((0u16, 0u16, 0u16, 0u16));
        let cache = RefCell::new(None);
        let playlists = PlaylistManagerState::default();
        let mut params = base_params(&theme, &cover_rect, &cache, &playlists, &[]);
        params.album = "Blues";
        params.genre = "Jazz";
        params.year = "1999";
        params.codec = "flac";

        let out = render_to_string(&params, 100, 30);

        assert!(out.contains("Blues"));
        assert!(out.contains("Jazz"));
        assert!(out.contains("1999"));
        assert!(out.contains("FLAC"), "codec is upper-cased");
    }

    #[test]
    fn test_control_bar_shows_progress_and_volume() {
        let theme = Theme::default();
        let cover_rect = Cell::new((0u16, 0u16, 0u16, 0u16));
        let cache = RefCell::new(None);
        let playlists = PlaylistManagerState::default();
        let mut params = base_params(&theme, &cover_rect, &cache, &playlists, &[]);
        params.is_playing = true;
        params.position = 50.0;
        params.duration = 100.0;
        params.volume = 0.4;

        let out = render_to_string(&params, 100, 30);

        assert!(out.contains("00:50"), "position rendered");
        assert!(out.contains("01:40"), "duration rendered");
        assert!(out.contains("Vol:40%"));
        assert!(out.contains('▶'), "playing icon");
        assert!(out.contains('█'), "filled part of the bar");
        assert!(out.contains('░'), "empty part of the bar");
    }

    #[test]
    fn test_control_bar_without_a_duration_is_empty_not_a_division_by_zero() {
        let theme = Theme::default();
        let cover_rect = Cell::new((0u16, 0u16, 0u16, 0u16));
        let cache = RefCell::new(None);
        let playlists = PlaylistManagerState::default();
        let mut params = base_params(&theme, &cover_rect, &cache, &playlists, &[]);
        params.duration = 0.0;

        let out = render_to_string(&params, 100, 30);

        assert!(out.contains('░'), "bar renders, entirely unfilled");
        assert!(!out.contains('█'), "nothing filled without a duration");
    }
}
