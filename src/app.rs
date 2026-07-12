use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{Event as CrosstermEvent, EventStream, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures_util::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::audio::engine::AudioEngine;
use crate::cli::{Cli, Command};
use crate::config::Config;
use crate::event::AppEvent;
use crate::input::handler::handle_key;
use crate::ui::{self, UiState};

pub struct App {
    ui_state: UiState,
    engine: AudioEngine,
    should_quit: bool,
}

impl App {
    pub fn new(config: &Config) -> crate::error::AppResult<Self> {
        let engine = AudioEngine::new()?;
        Ok(Self {
            ui_state: UiState {
                volume: config.default_volume,
                ..Default::default()
            },
            engine,
            should_quit: false,
        })
    }

    pub async fn run(&mut self, cli: Cli) -> crate::error::AppResult<()> {
        // Set up terminal
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

        // Handle CLI command (play a file)
        if let Some(Command::Play { file }) = cli.command {
            self.play_file(&file);
        }

        // Event loop
        let mut reader = EventStream::new();
        let mut tick_interval = tokio::time::interval(Duration::from_millis(100));

        loop {
            tokio::select! {
                crossterm_event = reader.next() => {
                    match crossterm_event {
                        Some(Ok(event)) => {
                            match event {
                                CrosstermEvent::Key(key) => {
                                    if let Some(app_event) = handle_key(key) {
                                        self.handle_event(app_event);
                                    }
                                }
                                CrosstermEvent::Resize(_, _) => {
                                    // Terminal resize — ratatui handles layout on next draw
                                }
                                _ => {}
                            }
                        }
                        Some(Err(e)) => {
                            tracing::error!("Crossterm event error: {e}");
                        }
                        None => {
                            // Event stream ended
                            break;
                        }
                    }
                }
                _ = tick_interval.tick() => {
                    self.handle_event(AppEvent::Tick);
                }
            }

            if self.should_quit {
                break;
            }

            // Render
            if let Err(e) = terminal.draw(|f| ui::render(f, &self.ui_state)) {
                tracing::error!("Render error: {e}");
            }
        }

        // Restore terminal
        disable_raw_mode().ok();
        execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();

        Ok(())
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Quit => {
                self.should_quit = true;
            }
            AppEvent::Key(key) => match key.code {
                KeyCode::Char(' ') => {
                    if self.engine.is_playing() || self.ui_state.is_playing {
                        self.engine.pause();
                        self.ui_state.is_playing = false;
                    } else {
                        self.engine.resume();
                        self.ui_state.is_playing = true;
                    }
                }
                KeyCode::Char('-') => {
                    let new_vol = (self.ui_state.volume - 0.05).max(0.0);
                    self.ui_state.volume = new_vol;
                    self.engine.set_volume(new_vol);
                }
                KeyCode::Char('=') => {
                    let new_vol = (self.ui_state.volume + 0.05).min(1.0);
                    self.ui_state.volume = new_vol;
                    self.engine.set_volume(new_vol);
                }
                _ => {}
            },
            AppEvent::Tick => {
                // Update position from engine
                let pos = self.engine.position_secs();
                self.ui_state.position = pos;

                if let Some(dur) = self.engine.duration_secs() {
                    self.ui_state.duration = dur;
                }

                // Check if track ended
                if self.ui_state.is_playing
                    && self.ui_state.duration > 0.0
                    && pos >= self.ui_state.duration
                {
                    self.ui_state.is_playing = false;
                    self.engine.stop();
                }
            }
        }
    }

    fn play_file(&mut self, path: &PathBuf) {
        match self.engine.play_file(path) {
            Ok(()) => {
                // Extract title from filename
                let title = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string();

                self.ui_state.title = title;
                self.ui_state.artist = "—".to_string();
                self.ui_state.position = 0.0;
                self.ui_state.duration = self.engine.duration_secs().unwrap_or(0.0);
                self.ui_state.is_playing = true;
                self.engine.set_volume(self.ui_state.volume);

                tracing::info!("Now playing: {:?}", path);
            }
            Err(e) => {
                tracing::error!("Failed to play file: {e}");
                self.ui_state.title = format!("Error: {e}");
                self.ui_state.is_playing = false;
            }
        }
    }
}
