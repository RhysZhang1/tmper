use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn render_help(f: &mut Frame) {
    let area = f.area();
    let popup_width = 50u16;
    let popup_height = 22u16;
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    f.render_widget(Clear, popup_area);

    let help_text = vec![
        Line::from(Span::styled(
            " Key Bindings",
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(Span::styled(" Playback", Style::default().fg(Color::Cyan))),
        Line::from("  Space       Play / Pause"),
        Line::from("  n / p       Next / Previous track"),
        Line::from("  - / =       Volume down / up"),
        Line::from("  r           Cycle repeat (Off→Track→Playlist)"),
        Line::from("  R           Toggle shuffle"),
        Line::from(""),
        Line::from(Span::styled(
            " Navigation",
            Style::default().fg(Color::Cyan),
        )),
        Line::from("  j / k       Move down / up"),
        Line::from("  gg / G      Jump to top / bottom"),
        Line::from("  Ctrl+d/u    Page down / up"),
        Line::from("  dd          Remove from playlist"),
        Line::from(""),
        Line::from(Span::styled(" Views", Style::default().fg(Color::Cyan))),
        Line::from("  1           Player view"),
        Line::from("  3           Fullscreen lyrics"),
        Line::from("  4           Fullscreen visualizer"),
        Line::from(""),
        Line::from(Span::styled(" Other", Style::default().fg(Color::Cyan))),
        Line::from("  / / Esc     Search / Clear"),
        Line::from("  [ ] { }     Lyrics offset ±0.5s / ±2s"),
        Line::from("  Ctrl+r      Reset lyrics offset"),
        Line::from("  q           Quit"),
        Line::from("  0           Toggle this help"),
    ];

    let para = Paragraph::new(help_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Help ")
                .style(Style::default().fg(Color::Yellow)),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(para, popup_area);
}
