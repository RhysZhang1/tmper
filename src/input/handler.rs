use crossterm::event::{KeyCode, KeyEvent};

use crate::event::AppEvent;

#[allow(dead_code)]
pub fn handle_key(event: KeyEvent) -> Option<AppEvent> {
    match event.code {
        KeyCode::Char('q') => Some(AppEvent::Quit),
        KeyCode::Char(' ') => {
            // Space — play/pause handled in app logic
            Some(AppEvent::Key(event))
        }
        KeyCode::Char('-') => Some(AppEvent::Key(event)),
        KeyCode::Char('=') => Some(AppEvent::Key(event)),
        _ => Some(AppEvent::Key(event)),
    }
}
