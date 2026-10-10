use image::GenericImageView;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
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
    /// Pixel size of one terminal cell (measured per frame; see `UiState`).
    pub cell_px: (u16, u16),
    /// A native graphics layer already covers the cover rect, so the block
    /// art would only show through wherever the two disagree.
    pub native_cover: bool,
    pub cover_gen: u64,
    pub cover_lines_cache: &'a RefCell<Option<CoverLinesCache>>,
    pub lyric_track: Option<&'a LyricTrack>,
    pub current_lyric_index: usize,
    pub lyrics_offset_ms: i64,
    pub visualizer_data: &'a [f32],
    pub playlist_state: &'a PlaylistManagerState,
    pub playing_index: Option<usize>,
    pub tracks: &'a [TrackDisplay],
    pub active_playlist: Option<u64>,
    /// `/` search is active — left-top panel shows filtered results.
    pub search_active: bool,
    pub search_query: &'a str,
    pub selected_index: usize,
}

/// Largest box inside `area` whose pixel aspect ratio matches an
/// `img_w` × `img_h` artwork, centred in `area`.
///
/// The cover used to be handed the whole panel. The two layers that draw it
/// then disagree about the slack that creates: the block art stretches the
/// picture to fill the panel, while the graphics layer keeps the aspect ratio
/// and covers only part of it — so whichever dimension had room left over
/// showed dithered blocks beside the real cover. Sizing the box to the artwork
/// removes the slack instead of arguing about it.
///
/// The box is chosen in cells but measured in pixels, because a cell is about
/// twice as tall as it is wide: a square cover needs a box twice as wide as it
/// is tall, not a square box.
fn fit_cover_rect(area: Rect, img_w: u32, img_h: u32, cell_px: (u16, u16)) -> Rect {
    if area.width == 0 || area.height == 0 || img_w == 0 || img_h == 0 {
        return area;
    }
    let (cell_w, cell_h) = (cell_px.0.max(1) as f32, cell_px.1.max(1) as f32);
    // Cell columns needed per row to reproduce the artwork's aspect ratio.
    let cols_per_row = (cell_h / cell_w) * (img_w as f32 / img_h as f32);
    if !cols_per_row.is_finite() || cols_per_row <= 0.0 {
        return area;
    }

    // Try the full width first; fall back to the full height when that comes
    // out taller than the panel.
    let mut w = area.width;
    let mut h = (w as f32 / cols_per_row).round().max(1.0) as u16;
    if h > area.height {
        h = area.height;
        w = (h as f32 * cols_per_row).round().max(1.0) as u16;
    }
    let w = w.clamp(1, area.width);
    let h = h.clamp(1, area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

/// Decode cover art bytes and render as colored block characters.
/// Uses Lanczos3 resize + lower-half block (▄) with fg/bg for 2× vertical resolution.
///
/// Returns the box the art was rendered into along with the lines, so the
/// caller can tell the native graphics layer where to put the real image.
fn cover_as_colored_lines(
    inner: Rect,
    bytes: &[u8],
    cell_px: (u16, u16),
) -> Option<(Rect, Vec<Line<'static>>)> {
    if inner.width == 0 || inner.height == 0 {
        return None;
    }

    let img = image::load_from_memory(bytes).ok()?;
    let (img_w, img_h) = img.dimensions();
    let box_rect = fit_cover_rect(inner, img_w, img_h, cell_px);
    let cols = box_rect.width as usize;
    let rows = box_rect.height as usize;
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
    Some((box_rect, lines))
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
                *cache = cover_as_colored_lines(inner, &cover[..], params.cell_px).map(
                    |(box_rect, lines)| CoverLinesCache {
                        gen: params.cover_gen,
                        width: inner.width,
                        height: inner.height,
                        rect: (box_rect.x, box_rect.y, box_rect.width, box_rect.height),
                        lines,
                    },
                );
            }
            if let Some(cached) = cache.as_ref() {
                let rect = Rect::new(cached.rect.0, cached.rect.1, cached.rect.2, cached.rect.3);
                // Both layers are placed against this box — see `fit_cover_rect`.
                params.cover_rect.set(cached.rect);
                if params.native_cover {
                    // Blank the cells rather than simply not drawing them:
                    // `Block` leaves the previous symbols in place, so the art
                    // would sit there in default colours and any part the
                    // image does not cover would still read as pixel blocks.
                    f.render_widget(Clear, rect);
                } else {
                    let para = Paragraph::new(cached.lines.clone());
                    f.render_widget(para, rect);
                }
                return;
            }
        }
    }

    // No image to show. Nothing for the graphics layer to draw either, but the
    // rect still has to name the panel so a cover that just went away is
    // cleared rather than left on screen.
    params
        .cover_rect
        .set((inner.x, inner.y, inner.width, inner.height));

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
            Constraint::Length(1),
        ])
        .split(area);

    render_lyrics_section(f, split[0], params);
    render_spectrum_section(f, split[1], params);
    render_song_info(f, split[2], params);
    render_now_playing(f, split[3], params);
    render_control_bar(f, split[4], params);
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

                crate::ui::views::lyrics_view::lyric_line(
                    lyric,
                    track.adjusted_position(params.position, params.lyrics_offset_ms),
                    style,
                    params.theme,
                    is_current,
                )
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
    if let Some(id) = params.active_playlist {
        if let Some(playlist) = params
            .playlist_state
            .playlists
            .iter()
            .find(|playlist| playlist.id == id)
        {
            parts.push(Span::styled(
                format!(" {} {} ", '🎵', playlist.name),
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

/// The track's name and artist on their own line, directly above the transport.
///
/// The cover panel draws them too, but only on the branch where it has no
/// image to show — so with cover art on, the thing the window is *about* was
/// nowhere on screen. This line does not care whether there is a cover.
fn render_now_playing(f: &mut Frame, area: Rect, params: &PlayerViewParams) {
    let title = if params.title.is_empty() || params.title == "No track" {
        "No track"
    } else {
        params.title
    };

    let mut spans = vec![Span::styled(
        format!(" {}", title),
        Style::default()
            .fg(params.theme.text)
            .add_modifier(Modifier::BOLD),
    )];

    // "—" is what an absent artist looks like on the way in (same test the
    // cover fallback makes), and a separator with nothing after it reads as a
    // rendering bug rather than as missing metadata.
    if !params.artist.is_empty() && params.artist != "—" {
        spans.push(Span::styled(" — ", Style::default().fg(params.theme.muted)));
        spans.push(Span::styled(
            params.artist.to_string(),
            Style::default().fg(params.theme.secondary),
        ));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
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

    // ── fit_cover_rect ──

    /// A square cover needs a box twice as wide as it is tall, because a
    /// terminal cell is about twice as tall as it is wide.
    #[test]
    fn cover_box_matches_a_square_artwork() {
        let box_rect = fit_cover_rect(Rect::new(0, 0, 40, 20), 500, 500, (10, 20));
        assert_eq!((box_rect.width, box_rect.height), (40, 20));
    }

    /// Panels are rarely the artwork's aspect. The box gives up the dimension
    /// that has slack rather than stretching the picture into it.
    #[test]
    fn cover_box_shrinks_the_dimension_with_slack() {
        // A very wide panel: height is the binding constraint.
        let wide = fit_cover_rect(Rect::new(0, 0, 80, 10), 500, 500, (10, 20));
        assert_eq!((wide.width, wide.height), (20, 10));
        assert_eq!(wide.x, 30, "centred horizontally");

        // A tall panel: width binds, and a square cover wants 2× that in rows.
        let tall = fit_cover_rect(Rect::new(0, 0, 10, 40), 500, 500, (10, 20));
        assert_eq!((tall.width, tall.height), (10, 5));
        assert_eq!(tall.y, 17, "centred vertically");
    }

    /// The box follows the *artwork's* aspect, not a fixed one: a 2:1 cover in
    /// a square panel gets a box half as tall as a square cover would.
    #[test]
    fn cover_box_follows_the_artwork_aspect() {
        let square = fit_cover_rect(Rect::new(0, 0, 40, 20), 500, 500, (10, 20));
        let wide = fit_cover_rect(Rect::new(0, 0, 40, 20), 1000, 500, (10, 20));
        assert!(wide.height < square.height, "wider art, shorter box");
        assert_eq!((wide.width, wide.height), (40, 10));
    }

    /// A portrait cover is limited by the panel height, so its box is narrow.
    #[test]
    fn cover_box_handles_portrait_artwork() {
        let box_rect = fit_cover_rect(Rect::new(0, 0, 40, 20), 500, 1000, (10, 20));
        assert_eq!((box_rect.width, box_rect.height), (20, 20));
    }

    /// Degenerate inputs must not panic or hand back a zero-sized box.
    #[test]
    fn cover_box_survives_empty_inputs() {
        let empty = Rect::new(3, 4, 0, 0);
        assert_eq!(fit_cover_rect(empty, 500, 500, (10, 20)), empty);

        let area = Rect::new(0, 0, 8, 4);
        assert_eq!(fit_cover_rect(area, 0, 0, (10, 20)), area);

        // A cell size of zero would divide by zero; it is clamped to 1.
        let box_rect = fit_cover_rect(area, 500, 500, (0, 0));
        assert!(box_rect.width >= 1 && box_rect.height >= 1);
        assert!(box_rect.width <= area.width && box_rect.height <= area.height);
    }

    /// Whatever the panel and the artwork, the box stays inside the panel.
    #[test]
    fn cover_box_never_leaves_the_panel() {
        let panel = Rect::new(5, 3, 21, 9);
        for (w, h) in [(1u32, 1u32), (1000, 3), (3, 1000), (16, 9), (9, 16)] {
            for cell in [(10u16, 20u16), (8, 17), (20, 20), (1, 1)] {
                let b = fit_cover_rect(panel, w, h, cell);
                assert!(
                    b.x >= panel.x
                        && b.y >= panel.y
                        && b.right() <= panel.right()
                        && b.bottom() <= panel.bottom(),
                    "artwork {w}x{h} at cell {cell:?} escaped the panel: {b:?}"
                );
                assert!(b.width >= 1 && b.height >= 1);
            }
        }
    }

    // ── cover_as_colored_lines ──

    #[test]
    fn test_cover_empty_bytes_returns_none() {
        assert!(cover_as_colored_lines(Rect::new(0, 0, 8, 8), &[], (10, 20)).is_none());
    }

    #[test]
    fn test_cover_zero_area_returns_none() {
        let png = make_png();
        assert!(cover_as_colored_lines(Rect::new(0, 0, 0, 0), &png, (10, 20)).is_none());
        assert!(cover_as_colored_lines(Rect::new(0, 0, 8, 0), &png, (10, 20)).is_none());
    }

    /// One text line per row of the *fitted* box, one span per column.
    #[test]
    fn test_cover_generated_png_yields_one_line_per_row() {
        let png = make_png();
        // 8×8 art in a 4×4 panel: the box is 4×2 (a square picture needs twice
        // the columns as rows), so two lines of four spans.
        let (box_rect, lines) =
            cover_as_colored_lines(Rect::new(0, 0, 4, 4), &png, (10, 20)).expect("png decodes");
        assert_eq!((box_rect.width, box_rect.height), (4, 2));
        assert_eq!(lines.len(), 2, "one text line per box row");
        let spans: usize = lines.iter().map(|l| l.spans.len()).sum();
        assert_eq!(spans, 4 * 2);
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
            cell_px: (10, 20),
            native_cover: false,
            cover_gen: 0,
            cover_lines_cache: cache,
            lyric_track: None,
            current_lyric_index: 0,
            lyrics_offset_ms: 0,
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

    /// The point of the whole arrangement: when the terminal draws the cover
    /// itself, the panel must not also carry the block art. The two never
    /// cover exactly the same pixels, and the difference is what showed up as
    /// a fringe of blocks beside the picture.
    #[test]
    fn the_block_art_steps_aside_for_a_native_cover() {
        let theme = Theme::default();
        let cover_rect = Cell::new((0u16, 0u16, 0u16, 0u16));
        let cache = RefCell::new(None);
        let playlists = PlaylistManagerState::default();
        let art = Arc::new(make_png());

        let mut params = base_params(&theme, &cover_rect, &cache, &playlists, &[]);
        params.cover_art = Some(&art);

        // No graphics layer: the block art is all there is, so it must show.
        // (▄/▀ come only from the cover renderer — the control bar uses █/░.)
        let with_blocks = render_to_string(&params, 100, 30);
        assert!(
            with_blocks.contains('▄') || with_blocks.contains('▀'),
            "the fallback has to render when nothing else covers the panel"
        );
        let fitted = cover_rect.get();

        // A graphics layer is up: the same cached art must leave the buffer
        // blank, or it shows through wherever the image stops short.
        params.native_cover = true;
        let without = render_to_string(&params, 100, 30);
        assert!(
            !without.contains('▄') && !without.contains('▀'),
            "block art must not survive a native cover"
        );
        assert_eq!(cover_rect.get(), fitted, "same box, same native placement");
    }

    /// The cover box is fitted to the artwork, so it stops short of the panel
    /// in the dimension that has slack instead of stretching the picture.
    #[test]
    fn the_cover_box_leaves_the_panel_margin_empty() {
        let theme = Theme::default();
        let cover_rect = Cell::new((0u16, 0u16, 0u16, 0u16));
        let cache = RefCell::new(None);
        let playlists = PlaylistManagerState::default();
        let art = Arc::new(make_png()); // square
        let mut params = base_params(&theme, &cover_rect, &cache, &playlists, &[]);
        params.cover_art = Some(&art);

        let _ = render_to_string(&params, 100, 30);
        let (x, y, w, h) = cover_rect.get();

        // The panel's inner area is 31×13. Square art at a 10×20 cell wants
        // two columns per row, which does not fit 31 columns into 13 rows, so
        // height binds: the box fills the panel vertically and is centred
        // horizontally with room to spare.
        assert_eq!((w, h), (26, 13), "box is fitted to the artwork's aspect");
        assert_eq!(y, 1, "no vertical slack — the height is the binding one");

        // Centred in the panel's inner area (which starts inside the border):
        // the odd leftover column goes to the right.
        let inner_x = 1;
        let left_margin = x - inner_x;
        let right_margin = (inner_x + 31) - (x + w);
        assert!(
            right_margin - left_margin <= 1,
            "box should be centred, margins were {left_margin}/{right_margin}"
        );
    }

    // ── Panel sections ──

    /// Render the whole view at a fixed size and flatten the buffer to text.
    /// The now-playing row: the one the transport bar sits under, read out of
    /// the right panel only.
    ///
    /// Anchored to the transport rather than to the title, because the cover
    /// panel centres the same title in its no-image fallback — searching for
    /// the text finds that row first whenever there is no cover. The columns
    /// are indexed into the cell grid, not into a string: a wide glyph is one
    /// cell spanning two columns, so string offsets drift on any CJK content
    /// to the left of the cut.
    fn now_playing_row(params: &PlayerViewParams, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| render_player_view(f, f.area(), params))
            .unwrap();

        let cells = terminal.backend().buffer().content();
        let w = width as usize;
        let row_of = |text: &str| {
            cells.chunks(w).position(|row| {
                row.iter()
                    .map(|c| c.symbol())
                    .collect::<String>()
                    .contains(text)
            })
        };

        let transport = row_of("Vol:").expect("the transport bar is on screen");
        assert!(transport > 0, "nothing is drawn above the transport");
        // The left half of the row is the cover panel and its divider.
        let right_starts_at = w * 33 / 100 + 1;
        let row = &cells[(transport - 1) * w..transport * w];
        row[right_starts_at..]
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
            .trim()
            .to_string()
    }

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
                .enumerate()
                .map(|(index, name)| PlaylistData {
                    // 1-based, so "the active one" is never the zero the store
                    // uses to mean "not stored yet".
                    id: index as u64 + 1,
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
        params.active_playlist = Some(2); // the id of "Focus"

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

    /// The reason this line exists. Cover art on used to mean the track name
    /// and artist appeared *nowhere*: the only code that drew them was the
    /// cover panel's no-image fallback, so switching covers on deleted them
    /// from the screen. So this test supplies a cover and demands the text.
    #[test]
    fn the_track_name_and_artist_survive_a_cover() {
        let theme = Theme::default();
        let cover_rect = Cell::new((0u16, 0u16, 0u16, 0u16));
        let cache = RefCell::new(None);
        let playlists = PlaylistManagerState::default();
        let art = Arc::new(make_png());

        let mut params = base_params(&theme, &cover_rect, &cache, &playlists, &[]);
        params.title = "Bohemian Rhapsody";
        params.artist = "Queen";
        params.show_cover_art = true;
        params.cover_art = Some(&art);

        assert_eq!(
            now_playing_row(&params, 100, 30),
            "Bohemian Rhapsody — Queen"
        );
    }

    /// Without a cover the row must not change — the fallback panel already
    /// centres the same text, and this line is not a second copy of it that
    /// happens to sit elsewhere.
    #[test]
    fn the_track_name_row_does_not_depend_on_a_cover() {
        let theme = Theme::default();
        let cover_rect = Cell::new((0u16, 0u16, 0u16, 0u16));
        let cache = RefCell::new(None);
        let playlists = PlaylistManagerState::default();
        let art = Arc::new(make_png());

        let mut with_cover = base_params(&theme, &cover_rect, &cache, &playlists, &[]);
        with_cover.title = "Bohemian Rhapsody";
        with_cover.artist = "Queen";
        with_cover.cover_art = Some(&art);

        let mut without = base_params(&theme, &cover_rect, &cache, &playlists, &[]);
        without.title = "Bohemian Rhapsody";
        without.artist = "Queen";
        without.cover_art = None;

        assert_eq!(
            now_playing_row(&with_cover, 100, 30),
            now_playing_row(&without, 100, 30)
        );
    }

    /// An absent artist must not leave a separator dangling off the title.
    #[test]
    fn a_missing_artist_leaves_no_dangling_separator() {
        let theme = Theme::default();
        let cover_rect = Cell::new((0u16, 0u16, 0u16, 0u16));
        let cache = RefCell::new(None);
        let playlists = PlaylistManagerState::default();

        let mut params = base_params(&theme, &cover_rect, &cache, &playlists, &[]);
        params.title = "Untitled Demo";
        params.artist = "";

        assert_eq!(now_playing_row(&params, 100, 30), "Untitled Demo");

        // The player passes "—" through for an artist it does not know.
        params.artist = "—";
        assert_eq!(now_playing_row(&params, 100, 30), "Untitled Demo");
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
