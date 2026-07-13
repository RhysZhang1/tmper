use crossterm::event::KeyEvent;

#[derive(Debug, Clone)]
pub enum AppEvent {
    Key(KeyEvent),
    Tick,
    Quit,
    JumpTop,
    #[allow(dead_code)]
    JumpBottom,
    RemoveSelected,
    #[allow(dead_code)]
    VisualizerData(Vec<f32>),
}
