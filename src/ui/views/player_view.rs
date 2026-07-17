use image::GenericImageView;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::ui::format_duration;
use crate::ui::UiState;

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
pub fn render_player_view(f: &mut Frame, area: Rect, state: &UiState) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(33), Constraint::Percentage(67)])
        .split(area);

    render_left_panel(f, cols[0], state);
    render_right_panel(f, cols[1], state);
}

// ═══════════════════════ LEFT PANEL ═══════════════════════

fn render_left_panel(f: &mut Frame, area: Rect, state: &UiState) {
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_cover_art(f, split[0], state);
    render_mini_playlist(f, split[1], state);
}

fn render_cover_art(f: &mut Frame, area: Rect, state: &UiState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Now Playing ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Store position for Kitty protocol rendering (read-only, no state mut)
    state
        .cover_rect
        .set((inner.x, inner.y, inner.width, inner.height));

    if state.show_cover_art {
        // Try to render cover art as colored blocks
        if let Some(ref cover) = state.cover_art {
            if let Some(lines) = cover_as_colored_lines(inner, &cover[..]) {
                let para = Paragraph::new(lines);
                f.render_widget(para, inner);
                return;
            }
        }
    }

    // Fallback / no-image mode: show detailed song info
    let h = inner.height.max(3);
    let play_icon = if state.is_playing { "▶" } else { "⏸" };
    let pos_str = format_duration(state.position);
    let dur_str = format_duration(state.duration);

    let mut lines: Vec<Line> = Vec::new();
    let w = inner.width as usize;

    // Vertical centering
    let content_lines = if state.album.is_empty() && state.genre.is_empty() {
        4
    } else {
        6
    };
    let top_spacer = (h.saturating_sub(content_lines)) / 2;
    for _ in 0..top_spacer {
        lines.push(Line::from(""));
    }

    // Track title
    let title = if state.title.is_empty() || state.title == "No track" {
        "No track"
    } else {
        state.title.as_str()
    };
    lines.push(Line::from(vec![Span::styled(
        format!("{:^w$}", title, w = w),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )]));

    // Artist
    let artist = if state.artist.is_empty() || state.artist == "—" {
        ""
    } else {
        state.artist.as_str()
    };
    if !artist.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            format!("{:^w$}", artist, w = w),
            Style::default().fg(Color::Gray),
        )]));
    }

    // Playback time
    if state.duration > 0.0 {
        let time_str = format!("{} {} / {}", play_icon, pos_str, dur_str);
        lines.push(Line::from(vec![Span::styled(
            format!("{:^w$}", time_str, w = w),
            Style::default().fg(Color::Green),
        )]));
    }

    // Spacer before metadata
    if !state.album.is_empty() || !state.genre.is_empty() {
        lines.push(Line::from(""));
    }

    // Album
    if !state.album.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(" 专辑: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                state.album.as_str(),
                Style::default().fg(Color::Rgb(180, 180, 200)),
            ),
        ]));
    }

    // Genre + Year + Codec
    let mut meta_parts = Vec::new();
    if !state.genre.is_empty() {
        meta_parts.push(Span::styled(
            format!("{} ", state.genre),
            Style::default().fg(Color::Rgb(160, 200, 160)),
        ));
    }
    if !state.year.is_empty() {
        meta_parts.push(Span::styled(
            format!("{} ", state.year),
            Style::default().fg(Color::Rgb(200, 180, 140)),
        ));
    }
    if !state.codec.is_empty() {
        meta_parts.push(Span::styled(
            state.codec.to_uppercase(),
            Style::default().fg(Color::Rgb(140, 140, 180)),
        ));
    }
    if !meta_parts.is_empty() {
        lines.push(Line::from(meta_parts));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

fn render_mini_playlist(f: &mut Frame, area: Rect, state: &UiState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Playlists ")
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let ps = &state.playlist_state;
    if ps.playlists.is_empty() {
        let para = Paragraph::new("(no playlists)").style(Style::default().fg(Color::DarkGray));
        f.render_widget(para, inner);
        return;
    }

    use crate::ui::views::playlist_view::{InsertMode, PlaylistFlatModel};

    let model = PlaylistFlatModel::new(&ps.playlists, ps.expanded_playlist);
    let all_lines =
        model.build_styled_lines(true, &InsertMode::Off, ps.selected_playlist, |song| {
            state
                .playing_index
                .and_then(|pi| state.tracks.get(pi).map(|t| t.path == *song))
                .unwrap_or(false)
        });

    // Skip the "..." row (index 0) for Player View sidebar
    let flat_lines: Vec<_> = all_lines.into_iter().skip(1).collect();

    let vis_h = inner.height as usize;
    let max_scroll = flat_lines.len().saturating_sub(vis_h);
    let scroll = ps.scroll_playlists.min(max_scroll);
    let end = (scroll + vis_h).min(flat_lines.len());
    let visible: Vec<Line> = flat_lines[scroll..end]
        .iter()
        .map(|(t, s)| Line::from(Span::styled(t.clone(), *s)))
        .collect();

    let para = Paragraph::new(visible);
    f.render_widget(para, inner);
}

// ═══════════════════════ RIGHT PANEL ═══════════════════════

fn render_right_panel(f: &mut Frame, area: Rect, state: &UiState) {
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(48),
            Constraint::Percentage(36),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    render_lyrics_section(f, split[0], state);
    render_spectrum_section(f, split[1], state);
    render_song_info(f, split[2], state);
    render_control_bar(f, split[3], state);
}

fn render_lyrics_section(f: &mut Frame, area: Rect, state: &UiState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Lyrics ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(ref track) = state.lyric_track {
        if track.lines.is_empty() {
            let para =
                Paragraph::new("No lyrics found").style(Style::default().fg(Color::DarkGray));
            f.render_widget(para, inner);
            return;
        }

        let visible_lines = inner.height as usize;
        if visible_lines < 2 {
            return;
        }

        let half = visible_lines / 2;
        let total = track.lines.len();

        let start = if state.current_lyric_index > half {
            state.current_lyric_index.saturating_sub(half)
        } else {
            0
        };
        let end = (start + visible_lines).min(total);

        let lines: Vec<Line> = (start..end)
            .map(|i| {
                let lyric = &track.lines[i];
                let is_current = i == state.current_lyric_index;
                let is_past = i < state.current_lyric_index;

                let style = if is_current {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else if is_past {
                    let dist = state.current_lyric_index.saturating_sub(i) as f32;
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
        let para = Paragraph::new("No lyrics loaded").style(Style::default().fg(Color::DarkGray));
        f.render_widget(para, inner);
    }
}

fn render_spectrum_section(f: &mut Frame, area: Rect, state: &UiState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Spectrum ")
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if state.visualizer_data.is_empty() {
        let para = Paragraph::new("No audio data").style(Style::default().fg(Color::DarkGray));
        f.render_widget(para, inner);
        return;
    }

    crate::ui::widgets::visualizer_panel::render_visualizer(f, inner, &state.visualizer_data);
}

fn render_song_info(f: &mut Frame, area: Rect, state: &UiState) {
    let mut parts: Vec<Span> = Vec::new();
    // Show active playlist name first
    if let Some(pl_idx) = state.active_playlist {
        if pl_idx < state.playlist_state.playlists.len() {
            let pl_name = &state.playlist_state.playlists[pl_idx].name;
            parts.push(Span::styled(
                format!(" {} {} ", '🎵', pl_name),
                Style::default()
                    .fg(Color::Rgb(255, 200, 100))
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }

    if !state.album.is_empty() {
        parts.push(Span::styled(
            format!(" {} {} ", "\u{1f4bf}", state.album),
            Style::default().fg(Color::Rgb(180, 180, 200)),
        ));
    }
    if !state.genre.is_empty() {
        parts.push(Span::styled(
            format!(" {} {} ", "\u{266a}", state.genre),
            Style::default().fg(Color::Rgb(160, 200, 160)),
        ));
    }
    if !state.year.is_empty() {
        parts.push(Span::styled(
            format!(" {} {} ", "\u{1f4c5}", state.year),
            Style::default().fg(Color::Rgb(200, 180, 140)),
        ));
    }
    if !state.codec.is_empty() {
        parts.push(Span::styled(
            format!(" {} ", state.codec.to_uppercase()),
            Style::default().fg(Color::Rgb(140, 140, 180)),
        ));
    }

    if !parts.is_empty() {
        let line = Line::from(parts);
        let para = Paragraph::new(line);
        f.render_widget(para, area);
    }
}

fn render_control_bar(f: &mut Frame, area: Rect, state: &UiState) {
    let play_icon = if state.is_playing { "▶" } else { "⏸" };
    let pos_str = format_duration(state.position);
    let dur_str = format_duration(state.duration);
    let vol_pct = (state.volume * 100.0) as u32;

    let progress = if state.duration > 0.0 {
        (state.position / state.duration).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let time_str = format!("{} {} / {}", play_icon, pos_str, dur_str);
    let vol_str = format!("Vol:{}%", vol_pct);
    let mode_str = state.repeat_mode.label().to_string();

    // Calculate space for progress bar
    let fixed = time_str.len() + vol_str.len() + mode_str.len() + 6;
    let bar_w = (area.width as usize).saturating_sub(fixed).min(30);
    let filled = (bar_w as f64 * progress).round() as usize;
    let empty = bar_w.saturating_sub(filled);

    let bar_progress = format!("{}{}", "█".repeat(filled), "░".repeat(empty));
    let bar = format!(
        "{} {} [{}] {}",
        time_str,
        vol_str,
        bar_progress,
        state.repeat_mode.label(),
    );

    let para = Paragraph::new(Line::from(Span::styled(
        bar,
        Style::default().fg(Color::Magenta),
    )));
    f.render_widget(para, area);
}
