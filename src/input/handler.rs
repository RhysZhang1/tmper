use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent};

use crate::event::AppEvent;

pub struct KeyHandler {
    last_key: Option<(KeyEvent, Instant)>,
    timeout_ms: u64,
    quit_key: KeyEvent,
}

impl KeyHandler {
    pub fn new(timeout_ms: u64, quit_key: KeyEvent) -> Self {
        Self {
            last_key: None,
            timeout_ms,
            quit_key,
        }
    }

    pub fn process(&mut self, event: KeyEvent) -> Option<AppEvent> {
        // Direct quit (configurable via keybindings)
        if event.code == self.quit_key.code && event.modifiers == self.quit_key.modifiers {
            self.last_key = None;
            return Some(AppEvent::Quit);
        }

        // Check for pending double-key sequence
        if let Some((prev_key, prev_time)) = self.last_key {
            let elapsed = prev_time.elapsed().as_millis() as u64;

            if elapsed <= self.timeout_ms {
                self.last_key = None;

                let combo = (prev_key.code, event.code);
                match combo {
                    (KeyCode::Char('g'), KeyCode::Char('g')) => {
                        return Some(AppEvent::JumpTop);
                    }
                    (KeyCode::Char('d'), KeyCode::Char('d')) => {
                        return Some(AppEvent::RemoveSelected);
                    }
                    _ => {
                        // Not a valid combo, pass through the original key
                        return Some(AppEvent::Key(prev_key));
                    }
                }
            } else {
                // Timeout expired — pass through the stored key first
                let stored = prev_key;
                self.last_key = Some((event, Instant::now()));
                return Some(AppEvent::Key(stored));
            }
        }

        // Single-key sequences that start potential double-keys
        match event.code {
            KeyCode::Char('g') | KeyCode::Char('d') => {
                self.last_key = Some((event, Instant::now()));
                None // Waiting for second key
            }
            _ => {
                self.last_key = None;
                Some(AppEvent::Key(event))
            }
        }
    }
}
