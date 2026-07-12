use crossterm::event::KeyEvent;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum AppEvent {
    Key(KeyEvent),
    Tick,
    Quit,
    JumpTop,
    JumpBottom,
    RemoveSelected,
}
