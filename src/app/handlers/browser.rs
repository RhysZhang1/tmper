use crossterm::event::{KeyCode, KeyEvent};

use crate::app::App;
use crate::ui::views::file_browser_view::{BrowserPanel, FsItem};

impl App {
    pub(super) fn handle_file_browser_key(&mut self, key: &KeyEvent) {
        if key.code == KeyCode::Char('a') {
            let root = self.ui_state.file_browser_state.current_dir.clone();
            if !self
                .ui_state
                .file_browser_state
                .library_paths
                .contains(&root)
            {
                self.ui_state
                    .file_browser_state
                    .library_paths
                    .push(root.clone());
                self.save_library_paths();
            }
            self.start_library_scan(root);
            return;
        }
        if key.code == KeyCode::Char('c') {
            self.cancel_library_scan();
            return;
        }
        let state = &mut self.ui_state.file_browser_state;
        match key.code {
            KeyCode::Tab | KeyCode::Char('l') | KeyCode::Right => {
                state.focused = match state.focused {
                    BrowserPanel::Library => BrowserPanel::Filesystem,
                    BrowserPanel::Filesystem => BrowserPanel::Library,
                };
            }
            KeyCode::Char('h') | KeyCode::Left => state.focused = BrowserPanel::Library,
            KeyCode::Char('j') | KeyCode::Down => {
                let vis_h = self.ui_state.visible_rows.get();
                match state.focused {
                    BrowserPanel::Library => {
                        let max = state.library_paths.len().saturating_sub(1);
                        state.selected_library_index = (state.selected_library_index + 1).min(max);
                        let sel = state.selected_library_index;
                        if sel < state.scroll_library {
                            state.scroll_library = sel;
                        }
                        if sel >= state.scroll_library + vis_h {
                            state.scroll_library = sel.saturating_sub(vis_h) + 1;
                        }
                    }
                    BrowserPanel::Filesystem => {
                        let max = state.fs_items.len().saturating_sub(1);
                        state.selected_fs_index = (state.selected_fs_index + 1).min(max);
                        let sel = state.selected_fs_index;
                        if sel < state.scroll_fs {
                            state.scroll_fs = sel;
                        }
                        if sel >= state.scroll_fs + vis_h {
                            state.scroll_fs = sel.saturating_sub(vis_h) + 1;
                        }
                    }
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let vis_h = self.ui_state.visible_rows.get();
                match state.focused {
                    BrowserPanel::Library => {
                        state.selected_library_index =
                            state.selected_library_index.saturating_sub(1);
                        let sel = state.selected_library_index;
                        if sel < state.scroll_library {
                            state.scroll_library = sel;
                        }
                        if sel >= state.scroll_library + vis_h {
                            state.scroll_library = sel.saturating_sub(vis_h) + 1;
                        }
                    }
                    BrowserPanel::Filesystem => {
                        state.selected_fs_index = state.selected_fs_index.saturating_sub(1);
                        let sel = state.selected_fs_index;
                        if sel < state.scroll_fs {
                            state.scroll_fs = sel;
                        }
                        if sel >= state.scroll_fs + vis_h {
                            state.scroll_fs = sel.saturating_sub(vis_h) + 1;
                        }
                    }
                }
            }
            KeyCode::Enter => match state.focused {
                BrowserPanel::Library => {
                    let idx = state.selected_library_index;
                    let lib_len = state.library_paths.len();
                    if idx < lib_len {
                        let path = state.library_paths.remove(idx);
                        self.ui_state.player.tracks.retain(|t| t.path != path);
                        if path.is_dir() {
                            let _ = self.library_db.delete_missing_under(&path, &[]);
                        }
                        let new_len = state.library_paths.len();
                        state.selected_library_index = idx.min(new_len.saturating_sub(1));
                        self.save_library_paths();
                    }
                }
                BrowserPanel::Filesystem => {
                    let fs_idx = state.selected_fs_index;
                    if fs_idx < state.fs_items.len() {
                        match &state.fs_items[fs_idx] {
                            FsItem::Dir(_) => {
                                state.current_dir = state.dirs[fs_idx].clone();
                                self.refresh_file_browser();
                            }
                            FsItem::Audio(_) => {
                                let path = state.audio_files[fs_idx].clone();
                                if !state.library_paths.contains(&path) {
                                    state.library_paths.push(path.clone());
                                    self.load_and_play_collect(&path);
                                    self.save_library_paths();
                                }
                            }
                        }
                    }
                }
            },
            KeyCode::Backspace if state.focused == BrowserPanel::Filesystem => {
                if let Some(parent) = state.current_dir.parent().map(|p| p.to_path_buf()) {
                    if parent.starts_with(&state.home_dir) || parent == state.home_dir {
                        state.current_dir = parent;
                        self.refresh_file_browser();
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn enter_file_browser(&mut self) {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"));
        self.ui_state.file_browser_state.current_dir = home;
        self.refresh_file_browser();
        if self.ui_state.file_browser_state.library_paths.is_empty() {
            self.ui_state.file_browser_state.library_paths = self
                .ui_state
                .player
                .tracks
                .iter()
                .map(|track| track.path.clone())
                .collect();
        }
    }

    pub(super) fn refresh_file_browser(&mut self) {
        let dir = self.ui_state.file_browser_state.current_dir.clone();
        let mut dirs = Vec::new();
        let mut audios = Vec::new();
        let mut items = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&dir) {
            let mut all: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            all.sort_by_key(|e| e.file_name());
            for entry in all {
                let path = entry.path();
                if path.is_dir() {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?")
                        .to_string();
                    dirs.push(path.clone());
                    items.push(FsItem::Dir(name));
                } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    let ext_lower = ext.to_lowercase();
                    if crate::library::scanner::AUDIO_EXTENSIONS.contains(&ext_lower.as_str()) {
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("?")
                            .to_string();
                        audios.push(path.clone());
                        items.push(FsItem::Audio(name));
                    }
                }
            }
        }
        self.ui_state.file_browser_state.dirs = dirs;
        self.ui_state.file_browser_state.audio_files = audios;
        self.ui_state.file_browser_state.fs_items = items;
        self.ui_state.file_browser_state.selected_fs_index = 0;
    }
}
