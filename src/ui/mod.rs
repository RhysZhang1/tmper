use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph};
use ratatui::Frame;

#[allow(dead_code)]
pub struct UiState {
    pub title: String,
    pub artist: String,
    pub position: f64,
    pub duration: f64,
    pub volume: f32,
    pub is_playing: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            title: "No track".into(),
            artist: "—".into(),
            position: 0.0,
            duration: 0.0,
            volume: 0.8,
            is_playing: false,
        }
    }
}

#[allow(dead_code)]
pub fn render(f: &mut Frame, state: &UiState) {
    let area = f.area();

    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title bar
            Constraint::Length(1), // progress bar
            Constraint::Min(0),    // empty space (for future spectrum/lyrics/tracklist)
        ])
        .split(area);

    render_title_bar(f, main_layout[0], state);
    render_progress_bar(f, main_layout[1], state);
}

fn render_title_bar(f: &mut Frame, area: Rect, state: &UiState) {
    let play_icon = if state.is_playing { "▶" } else { "⏸" };
    let pos_str = format_duration(state.position);
    let dur_str = format_duration(state.duration);
    let title = format!(
        "{} {} — {}   {} / {}",
        play_icon, state.title, state.artist, pos_str, dur_str
    );
    let para = Paragraph::new(title).style(Style::default().fg(Color::White));
    f.render_widget(para, area);
}

fn render_progress_bar(f: &mut Frame, area: Rect, state: &UiState) {
    let progress = if state.duration > 0.0 {
        (state.position / state.duration).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let vol_pct = (state.volume * 100.0) as u32;
    let label = format!("Vol: {}%", vol_pct);

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::NONE))
        .gauge_style(Style::default().fg(Color::Magenta))
        .label(label)
        .ratio(progress);

    f.render_widget(gauge, area);
}

fn format_duration(secs: f64) -> String {
    let total_secs = secs as u64;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{:02}:{:02}", mins, secs)
}
