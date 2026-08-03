use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::Event as CrosstermEvent;
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::audio::engine::AudioEngine;
use crate::cli::{Cli, Command};
use crate::config::Config;
use crate::constants::runtime;
use crate::event::AppEvent;
use crate::input::handler::KeyHandler;
use crate::input::keymap::{self, KeyBindings};
use crate::library::database::LibraryDb;
use crate::ui::cover::{CoverParams, CoverRenderer};
use crate::ui::theme::Theme;
use crate::ui::{self, PlayerCore, UiState};
use serde::{Deserialize, Serialize};

pub(crate) mod handlers;
pub(crate) mod persistence;
pub(crate) mod playback;

pub struct App {
    config: Config,
    ui_state: UiState,
    engine: AudioEngine,
    should_quit: bool,
    key_handler: KeyHandler,
    key_bindings: KeyBindings,
    fft_cancel_tx: Option<tokio::sync::watch::Sender<()>>,
    library_db: LibraryDb,
    fft_data: Arc<Mutex<Vec<f32>>>,
    cover_renderer: CoverRenderer,
    last_seek_time: Option<std::time::Instant>,
}

#[derive(Serialize, Deserialize)]
struct SavedState {
    volume: f32,
    repeat_mode: crate::ui::RepeatMode,
    lyrics_offset_ms: i64,
    last_track_path: Option<String>,
}

impl App {
    pub fn new(config: &Config) -> crate::error::AppResult<Self> {
        let engine = AudioEngine::new()?;
        let library_db = LibraryDb::open(&crate::paths::data_dir().join("library.db"))
            .unwrap_or_else(|_| LibraryDb::open_memory().expect("in-memory db"));
        let key_bindings = KeyBindings::load();
        let quit_key = keymap::parse_key_str(&key_bindings.quit);
        let mut app = Self {
            config: config.clone(),
            ui_state: UiState {
                theme: Theme::load(&config.ui.theme),
                volume: config.playback.default_volume,
                player: PlayerCore {
                    show_cover_art: config.ui.show_cover_art,
                    ..Default::default()
                },
                ..Default::default()
            },
            engine,
            should_quit: false,
            key_handler: KeyHandler::new(runtime::KEY_TIMEOUT_MS, quit_key),
            key_bindings,
            library_db,
            fft_cancel_tx: None,
            fft_data: Arc::new(Mutex::new(Vec::new())),
            cover_renderer: CoverRenderer::new(),
            last_seek_time: None,
        };
        app.load_state();
        Ok(app)
    }

    pub async fn run(&mut self, cli: Cli) -> crate::error::AppResult<()> {
        enable_raw_mode().map_err(|e| {
            crate::error::AppError::Config(format!("Failed to enable raw mode: {e}"))
        })?;

        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen).map_err(|e| {
            crate::error::AppError::Config(format!("Failed to enter alternate screen: {e}"))
        })?;

        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).map_err(|e| {
            crate::error::AppError::Config(format!("Failed to create terminal: {e}"))
        })?;

        // Seed visible_rows from actual terminal size so the first keypress
        // uses the correct value instead of the hardcoded default (20).
        if let Ok((_, rows)) = crossterm::terminal::size() {
            self.ui_state
                .visible_rows
                .set(rows.saturating_sub(2) as usize);
        }

        self.load_library_paths();
        self.load_playlists();
        if let Some(Command::Play { file }) = cli.command {
            self.load_and_play(&file);
            self.start_fft();
        }

        // ── Burst-mode input ──
        // Instead of EventStream (one event → one draw), use a background
        // thread that batch-reads events with poll(). When the user holds a
        // key, terminal auto-repeat piles events into the PTY buffer. We
        // drain them all at once, process the batch, then draw ONCE — no
        // more « released the key but it kept scrolling ».
        let (event_tx, mut event_rx) =
            tokio::sync::mpsc::unbounded_channel::<Vec<CrosstermEvent>>();

        tokio::task::spawn_blocking(move || loop {
            // Check whether the main loop has exited before blocking
            // on poll().  Without this the thread would spin in the
            // poll→timeout→poll loop forever, never reaching send()
            // to discover the channel is closed.
            if event_tx.is_closed() {
                break;
            }
            // Wait up to 80ms for the first event.  During hold, terminal
            // repeats at ~33ms so we catch 2–3 repeats per batch.
            if crossterm::event::poll(Duration::from_millis(80)).unwrap_or(false) {
                let mut batch = Vec::with_capacity(8);
                if let Ok(e) = crossterm::event::read() {
                    batch.push(e);
                }
                // Drain whatever is already waiting — this catches events
                // the PTY buffered while we were reading the first one.
                while crossterm::event::poll(Duration::ZERO).unwrap_or(false) {
                    if let Ok(e) = crossterm::event::read() {
                        batch.push(e);
                    }
                }
                if event_tx.send(batch).is_err() {
                    break; // receiver dropped (main loop exited)
                }
            }
        });

        let mut tick_interval = tokio::time::interval(Duration::from_millis(
            (1000 / self.config.visualizer.frame_rate.max(1)) as u64,
        ));

        // Tick-only draw throttling: don't redraw faster than every 50ms
        // on tick events (key batches always draw).
        let min_draw_interval = Duration::from_millis(50);
        let mut last_draw = std::time::Instant::now();
        let mut needs_draw = true;

        loop {
            tokio::select! {
                batch = event_rx.recv() => {
                    match batch {
                        Some(events) => {
                            for event in events {
                                match event {
                                    CrosstermEvent::Key(key) => {
                                        let needs_bypass = matches!(self.ui_state.playlist_state.insert_mode, crate::ui::views::playlist_view::InsertMode::Typing(_));
                                        if needs_bypass {
                                            self.handle_event(crate::event::AppEvent::Key(key));
                                        } else if let Some(app_event) = self.key_handler.process(key) {
                                            self.handle_event(app_event);
                                        }
                                    }
                                    CrosstermEvent::Resize(_, _) => {}
                                    _ => {}
                                }
                            }
                            needs_draw = true;
                        }
                        None => break, // channel closed
                    }
                }
                _ = tick_interval.tick() => {
                    self.handle_event(AppEvent::Tick);
                    let now = std::time::Instant::now();
                    if now.duration_since(last_draw) >= min_draw_interval {
                        needs_draw = true;
                    }
                }
            }

            if self.should_quit {
                self.engine.stop();
                break;
            }

            if needs_draw {
                if self.cover_renderer.needs_clear() {
                    let _ = terminal.clear();
                    self.cover_renderer.clear_done();
                }

                if let Err(e) = terminal.draw(|f| ui::render(f, &self.ui_state)) {
                    tracing::error!("Render error: {e}");
                }

                let cover_params = CoverParams {
                    active_view: self.ui_state.view.active_view,
                    show_help: self.ui_state.view.show_help,
                    command_mode: self.ui_state.command_mode,
                    show_cover_art: self.ui_state.player.show_cover_art,
                    cover_gen: self.ui_state.player.cover_gen.get(),
                    cover_art: self.ui_state.player.cover_art.clone(),
                    cover_rect: self.ui_state.cover_rect.get(),
                };
                // Dispatch to the active protocol (Kitty or chafa SIXEL).
                self.cover_renderer.render(&cover_params);

                last_draw = std::time::Instant::now();
                needs_draw = false;
            }
        }

        // Stop FFT
        self.fft_cancel_tx = None;
        disable_raw_mode().ok();
        execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::event::AppEvent;
    use crate::ui::RepeatMode;
    use crate::ui::ViewMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;

    struct TestApp {
        app: App,
    }

    impl Drop for TestApp {
        fn drop(&mut self) {
            // Cancel any background decode task so the tokio runtime
            // can shut down cleanly.
            self.app.engine.stop();
        }
    }

    impl TestApp {
        fn new() -> Self {
            let config = Config::default();
            let app = App::new(&config).expect("Failed to create App");
            Self { app }
        }

        fn press_key(&mut self, code: KeyCode) {
            self.app
                .handle_event(AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        }

        fn press_char(&mut self, c: char) {
            self.press_key(KeyCode::Char(c));
        }

        fn tick(&mut self) {
            self.app.handle_event(AppEvent::Tick);
        }

        fn active_view(&self) -> ViewMode {
            self.app.ui_state.view.active_view
        }

        fn volume(&self) -> f32 {
            self.app.ui_state.volume
        }

        fn show_help(&self) -> bool {
            self.app.ui_state.view.show_help
        }

        fn repeat_mode(&self) -> RepeatMode {
            self.app.ui_state.repeat_mode
        }

        fn load_and_play(&mut self, path: &std::path::Path) {
            self.app.load_and_play(&path.to_path_buf());
        }
    }

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from("tests/fixtures").join(name)
    }

    // ── P0: View switching ──

    #[test]
    fn test_view_toggle_returns_to_player() {
        let mut ta = TestApp::new();
        assert_eq!(ta.active_view(), ViewMode::Player);
        ta.press_char('3');
        assert_eq!(ta.active_view(), ViewMode::Lyrics);
        ta.press_char('3');
        assert_eq!(ta.active_view(), ViewMode::Player);
    }

    #[test]
    fn test_view_switch_all_seven_views() {
        let mut ta = TestApp::new();
        let keys = ['1', '2', '3', '4', '5', '6', '7'];
        let expected = [
            ViewMode::Player,
            ViewMode::Library,
            ViewMode::Lyrics,
            ViewMode::Visualizer,
            ViewMode::Playlists,
            ViewMode::Browser,
            ViewMode::Settings,
        ];
        for (&key, &exp) in keys.iter().zip(expected.iter()) {
            ta.press_char(key);
            assert_eq!(ta.active_view(), exp, "Key '{key}'");
            ta.press_char(key); // toggle back
            assert_eq!(ta.active_view(), ViewMode::Player);
        }
    }

    // ── P0: Volume after seek ──

    #[tokio::test]
    async fn test_volume_unchanged_after_seek() {
        let mut ta = TestApp::new();
        ta.load_and_play(&fixture("test.wav"));
        ta.tick();
        let before = ta.volume();
        assert!(before > 0.0);
        ta.press_key(KeyCode::Right);
        ta.tick();
        let after = ta.volume();
        assert!(
            (before - after).abs() < 0.01,
            "Volume changed: {before} → {after}"
        );
    }

    #[test]
    fn test_volume_up_down_keys() {
        let mut ta = TestApp::new();
        let before = ta.volume();
        ta.press_char('=');
        assert!(ta.volume() > before);
        let mid = ta.volume();
        ta.press_char('-');
        assert!(ta.volume() < mid);
    }

    #[test]
    fn test_volume_clamped() {
        let mut ta = TestApp::new();
        for _ in 0..50 {
            ta.press_char('-');
        }
        assert!((ta.volume() - 0.0).abs() < 0.001);
        for _ in 0..50 {
            ta.press_char('=');
        }
        assert!((ta.volume() - 1.0).abs() < 0.001);
    }

    // ── P1: Repeat mode ──

    #[test]
    fn test_repeat_mode_cycles() {
        let mut ta = TestApp::new();
        assert_eq!(ta.repeat_mode(), RepeatMode::Sequential);
        ta.press_char('r');
        assert_eq!(ta.repeat_mode(), RepeatMode::Shuffle);
        ta.press_char('r');
        assert_eq!(ta.repeat_mode(), RepeatMode::SingleTrack);
        ta.press_char('r');
        assert_eq!(ta.repeat_mode(), RepeatMode::Sequential);
    }

    // ── P1: Help toggle + cooldown ──

    #[test]
    fn test_help_toggle() {
        let mut ta = TestApp::new();
        assert!(!ta.show_help());
        ta.press_char('8');
        assert!(ta.show_help());
        ta.press_char('8');
        assert!(!ta.show_help());
    }

    #[test]
    fn test_help_cooldown_blocks_reopen() {
        let mut ta = TestApp::new();
        ta.press_char('8');
        assert!(ta.show_help());
        ta.press_char('8');
        assert!(!ta.show_help());
        // Rapid reopen blocked by 500ms cooldown
        ta.press_char('8');
        assert!(!ta.show_help());
    }

    // ── P1: Load and play ──

    #[tokio::test]
    async fn test_load_and_play_sets_state() {
        let mut ta = TestApp::new();
        ta.load_and_play(&fixture("test.wav"));
        ta.tick();
        assert!(!ta.app.ui_state.player.title.is_empty());
        assert!(ta.app.ui_state.player.duration > 0.0);
        assert!(ta.app.ui_state.player.is_playing);
    }

    #[tokio::test]
    async fn test_tagless_file_uses_filename() {
        let mut ta = TestApp::new();
        ta.load_and_play(&fixture("test_notags.wav"));
        assert_eq!(ta.app.ui_state.player.title, "test_notags");
        assert_eq!(ta.app.ui_state.player.artist, "Unknown Artist");
    }

    // ── P2: Stop clears state ──

    #[tokio::test]
    async fn test_stop_clears_engine_state() {
        let mut ta = TestApp::new();
        ta.load_and_play(&fixture("test.wav"));
        ta.tick();
        assert!(ta.app.ui_state.player.is_playing);
        ta.app.engine.stop();
        // Engine-level stop clears engine state; UI state updates in handle_tick
        assert!(!ta.app.engine.is_playing());
        assert!(ta.app.engine.duration_secs().is_none());
    }

    // ── P2: Command mode ──

    #[test]
    fn test_command_mode_enter_exit() {
        let mut ta = TestApp::new();
        assert!(!ta.app.ui_state.command_mode);
        ta.press_char(':');
        assert!(ta.app.ui_state.command_mode);
        ta.press_key(KeyCode::Esc);
        assert!(!ta.app.ui_state.command_mode);
    }

    #[test]
    fn test_search_enter_filter_navigate_exit() {
        let mut ta = TestApp::new();
        // Seed the player queue directly (no audio device needed).
        let raw = [
            ("song_a.flac", "Alpha One", "Artist X"),
            ("song_b.flac", "Beta Two", "Artist Y"),
            ("song_c.flac", "Alpha Three", "Artist X"),
        ];
        ta.app.ui_state.player.tracks = raw
            .iter()
            .map(|(p, t, a)| crate::ui::TrackDisplay {
                path: std::path::PathBuf::from(p),
                title: t.to_string(),
                artist: a.to_string(),
                duration_secs: 1.0,
            })
            .collect();

        // Enter search with '/'
        ta.press_char('/');
        assert!(ta.app.ui_state.search_mode);

        // Typing filters the queue; 'alpha' matches tracks 0 and 2.
        for c in "alpha".chars() {
            ta.press_char(c);
        }
        assert_eq!(ta.app.ui_state.search_query, "alpha");
        let matches = crate::ui::search_matches(&ta.app.ui_state.player.tracks, "alpha");
        assert_eq!(matches, vec![0, 2]);

        // j/k navigate within matches; Enter plays + exits; Esc cancels.
        ta.press_char('j');
        assert_eq!(ta.app.ui_state.player.selected_index, 2);
        ta.press_char('k');
        assert_eq!(ta.app.ui_state.player.selected_index, 0);
        ta.press_key(KeyCode::Esc);
        assert!(!ta.app.ui_state.search_mode);
        assert!(ta.app.ui_state.search_query.is_empty());
    }
}
