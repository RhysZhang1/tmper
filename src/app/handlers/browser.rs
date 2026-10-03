use crossterm::event::{KeyCode, KeyEvent};

use crate::app::App;
use crate::ipc::proto::Request;
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
                    let removed = if idx < state.library_paths.len() {
                        let path = state.library_paths.remove(idx);
                        let new_len = state.library_paths.len();
                        state.selected_library_index = idx.min(new_len.saturating_sub(1));
                        Some(path)
                    } else {
                        None
                    };
                    if let Some(path) = removed {
                        // Taking a path out of the library takes its rows out
                        // of the index — for a directory, everything under it,
                        // and for a single file, that file. The queue is left
                        // alone: what is playing now is not a claim about what
                        // the collection holds.
                        self.dispatch(Request::RemoveLibraryPath { root: path });
                        self.save_library_paths();
                    }
                }
                BrowserPanel::Filesystem => {
                    let fs_idx = state.selected_fs_index;
                    // The path comes from the row itself — never from a
                    // parallel array indexed by the same number.
                    let target = state.fs_items.get(fs_idx).map(|item| {
                        (
                            matches!(item, FsItem::Dir { .. }),
                            item.path().to_path_buf(),
                        )
                    });
                    if let Some((is_dir, path)) = target {
                        if is_dir {
                            state.current_dir = path;
                            self.refresh_file_browser();
                        } else if !state.library_paths.contains(&path) {
                            state.library_paths.push(path.clone());
                            self.collect_track(&path);
                            self.save_library_paths();
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
        let mut items = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&dir) {
            let mut all: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            all.sort_by_key(|e| e.file_name());
            for entry in all {
                let path = entry.path();
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string();
                if path.is_dir() {
                    items.push(FsItem::Dir { name, path });
                } else if crate::library::scanner::is_audio_file(&path) {
                    items.push(FsItem::Audio { name, path });
                }
            }
        }
        self.ui_state.file_browser_state.fs_items = items;
        self.ui_state.file_browser_state.selected_fs_index = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::handlers::test_support::test_app;
    use crate::ui::views::file_browser_view::{BrowserPanel, FsItem};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static DIR_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Create a unique scratch directory under the OS temp dir.
    fn scratch(tag: &str) -> PathBuf {
        let n = DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("tmper-browser-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_file_browser_key(&key(code));
    }

    fn set_dir(app: &mut App, dir: PathBuf) {
        let s = &mut app.ui_state.file_browser_state;
        s.home_dir = dir.clone();
        s.current_dir = dir;
    }

    // ── Focus switching ──

    #[test]
    fn test_tab_and_l_right_toggle_focus() {
        let mut app = test_app();
        app.ui_state.file_browser_state.focused = BrowserPanel::Filesystem;
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.ui_state.file_browser_state.focused,
            BrowserPanel::Library
        );
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.ui_state.file_browser_state.focused,
            BrowserPanel::Filesystem
        );
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(
            app.ui_state.file_browser_state.focused,
            BrowserPanel::Library
        );
        press(&mut app, KeyCode::Right);
        assert_eq!(
            app.ui_state.file_browser_state.focused,
            BrowserPanel::Filesystem
        );
    }

    #[test]
    fn test_h_and_left_force_library_focus() {
        let mut app = test_app();
        app.ui_state.file_browser_state.focused = BrowserPanel::Filesystem;
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(
            app.ui_state.file_browser_state.focused,
            BrowserPanel::Library
        );
        press(&mut app, KeyCode::Char('h')); // idempotent
        assert_eq!(
            app.ui_state.file_browser_state.focused,
            BrowserPanel::Library
        );
        press(&mut app, KeyCode::Left);
        assert_eq!(
            app.ui_state.file_browser_state.focused,
            BrowserPanel::Library
        );
    }

    // ── Navigation ──

    #[test]
    fn test_down_moves_library_selection_and_clamps() {
        let mut app = test_app();
        {
            let s = &mut app.ui_state.file_browser_state;
            s.focused = BrowserPanel::Library;
            s.library_paths = vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c"),
            ];
        }
        press(&mut app, KeyCode::Down);
        assert_eq!(app.ui_state.file_browser_state.selected_library_index, 1);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.ui_state.file_browser_state.selected_library_index, 2);
    }

    #[test]
    fn test_up_moves_library_selection_and_saturates() {
        let mut app = test_app();
        {
            let s = &mut app.ui_state.file_browser_state;
            s.focused = BrowserPanel::Library;
            s.library_paths = vec![PathBuf::from("/a"), PathBuf::from("/b")];
            s.selected_library_index = 1;
        }
        press(&mut app, KeyCode::Up);
        assert_eq!(app.ui_state.file_browser_state.selected_library_index, 0);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.ui_state.file_browser_state.selected_library_index, 0);
    }

    #[test]
    fn test_down_moves_fs_selection_and_clamps() {
        let mut app = test_app();
        {
            let s = &mut app.ui_state.file_browser_state;
            s.focused = BrowserPanel::Filesystem;
            s.fs_items = vec![
                FsItem::Dir {
                    name: "a".into(),
                    path: PathBuf::from("/a"),
                },
                FsItem::Dir {
                    name: "b".into(),
                    path: PathBuf::from("/b"),
                },
                FsItem::Dir {
                    name: "c".into(),
                    path: PathBuf::from("/c"),
                },
            ];
        }
        press(&mut app, KeyCode::Down);
        assert_eq!(app.ui_state.file_browser_state.selected_fs_index, 1);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.ui_state.file_browser_state.selected_fs_index, 2);
    }

    // ── Enter actions ──

    /// Removing a library path takes its rows out of the index with it — the
    /// two describe the same files, and a row whose directory is no longer
    /// claimed would keep listing itself in every panel.
    #[test]
    fn test_enter_library_removes_path_and_forgets_its_rows() {
        let mut app = test_app();
        let removed = PathBuf::from("/music/gone.flac");
        let kept = PathBuf::from("/music/keep.flac");
        for path in [&removed, &kept] {
            app.player_mut()
                .library_mut()
                .db_mut()
                .upsert(
                    &path.to_string_lossy(),
                    "T",
                    Some("A"),
                    Some("Album"),
                    None,
                    Some(1),
                    Some(1),
                    None,
                    None,
                    1.0,
                    0,
                    44100,
                    2,
                    "FLAC",
                    10,
                    1,
                )
                .expect("upsert");
        }
        {
            let s = &mut app.ui_state.file_browser_state;
            s.focused = BrowserPanel::Library;
            s.library_paths = vec![removed.clone(), kept.clone()];
            s.selected_library_index = 0;
        }
        press(&mut app, KeyCode::Enter);
        let s = &app.ui_state.file_browser_state;
        assert!(!s.library_paths.contains(&removed));
        assert_eq!(s.library_paths, vec![kept.clone()]);
        let db = app.player().library().db();
        assert!(db
            .get_by_path(&removed.to_string_lossy())
            .unwrap()
            .is_none());
        assert!(
            db.get_by_path(&kept.to_string_lossy()).unwrap().is_some(),
            "only the removed path's rows go"
        );
    }

    #[test]
    fn test_enter_fs_dir_navigates_and_refreshes() {
        let mut app = test_app();
        let root = scratch("nav");
        let sub = root.join("subdir");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("inner.mp3"), b"x").unwrap();
        set_dir(&mut app, root.clone());
        app.refresh_file_browser();

        // Focus the directory row regardless of sort order.
        let dir_idx = {
            let s = &app.ui_state.file_browser_state;
            s.fs_items
                .iter()
                .position(|i| matches!(i, FsItem::Dir { .. }))
                .expect("dir present")
        };
        app.ui_state.file_browser_state.selected_fs_index = dir_idx;
        press(&mut app, KeyCode::Enter);

        assert_eq!(app.ui_state.file_browser_state.current_dir, sub);
        assert_eq!(app.ui_state.file_browser_state.fs_items.len(), 1);
        assert!(matches!(
            app.ui_state.file_browser_state.fs_items.first(),
            Some(FsItem::Audio { .. })
        ));
        assert_eq!(app.ui_state.file_browser_state.selected_fs_index, 0);
    }

    #[test]
    fn test_enter_fs_audio_adds_to_library_once() {
        let mut app = test_app();
        let root = scratch("audio");
        let audio = root.join("track.mp3");
        std::fs::write(&audio, b"x").unwrap();
        set_dir(&mut app, root);
        app.refresh_file_browser();
        assert!(matches!(
            app.ui_state.file_browser_state.fs_items.first(),
            Some(FsItem::Audio { .. })
        ));

        press(&mut app, KeyCode::Enter);
        assert!(app
            .ui_state
            .file_browser_state
            .library_paths
            .contains(&audio));
        // Duplicate guard: a second Enter must not push the same path twice.
        press(&mut app, KeyCode::Enter);
        let count = app
            .ui_state
            .file_browser_state
            .library_paths
            .iter()
            .filter(|p| **p == audio)
            .count();
        assert_eq!(count, 1);
    }

    /// A directory holding both subdirectories and audio files used to break
    /// Enter: the handler indexed `dirs` / `audio_files` (each compacted to
    /// only its own kind) with the position into `fs_items` (both kinds,
    /// merged and sorted). With `[Dir(aaa), Audio(bbb.mp3), Audio(ccc.mp3)]`
    /// that means picking row 1 opened `ccc.mp3`, and row 2 indexed past the
    /// end of `audio_files` and panicked.
    #[test]
    fn test_enter_fs_mixed_directory_targets_the_selected_row() {
        let mut app = test_app();
        let root = scratch("mixed");
        std::fs::create_dir_all(root.join("aaa")).unwrap();
        std::fs::write(root.join("bbb.mp3"), b"x").unwrap();
        std::fs::write(root.join("ccc.mp3"), b"x").unwrap();
        set_dir(&mut app, root.clone());
        app.refresh_file_browser();

        // Sorting puts the directory first, so a naive index into a
        // directories-only list would disagree from row 1 onward.
        let s = &app.ui_state.file_browser_state;
        assert!(matches!(s.fs_items.first(), Some(FsItem::Dir { .. })));
        assert_eq!(s.fs_items.len(), 3);

        // Row 1 is bbb.mp3 — it must add bbb.mp3, not ccc.mp3.
        app.ui_state.file_browser_state.selected_fs_index = 1;
        press(&mut app, KeyCode::Enter);
        assert!(
            app.ui_state
                .file_browser_state
                .library_paths
                .contains(&root.join("bbb.mp3")),
            "row 1 must target the row that is displayed there"
        );
        assert!(!app
            .ui_state
            .file_browser_state
            .library_paths
            .contains(&root.join("ccc.mp3")));

        // Row 2 is the last audio file — this used to index out of bounds.
        app.ui_state.file_browser_state.selected_fs_index = 2;
        press(&mut app, KeyCode::Enter);
        assert!(app
            .ui_state
            .file_browser_state
            .library_paths
            .contains(&root.join("ccc.mp3")));
    }

    /// The directory row must navigate to the directory shown on that row,
    /// not to whichever directory happens to sit at that index.
    #[test]
    fn test_enter_fs_dir_row_navigates_to_that_directory() {
        let mut app = test_app();
        let root = scratch("mixed-nav");
        std::fs::create_dir_all(root.join("zzz")).unwrap();
        std::fs::write(root.join("aaa.mp3"), b"x").unwrap();
        std::fs::write(root.join("bbb.mp3"), b"x").unwrap();
        set_dir(&mut app, root.clone());
        app.refresh_file_browser();

        // [Audio(aaa.mp3), Audio(bbb.mp3), Dir(zzz)] — `dirs` has length 1, so
        // the directory row at index 2 is the one that used to overflow.
        let dir_idx = {
            let s = &app.ui_state.file_browser_state;
            s.fs_items
                .iter()
                .position(|i| matches!(i, FsItem::Dir { .. }))
                .expect("dir present")
        };
        app.ui_state.file_browser_state.selected_fs_index = dir_idx;
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.ui_state.file_browser_state.current_dir,
            root.join("zzz")
        );
    }

    #[test]
    fn test_enter_library_out_of_bounds_is_noop() {
        let mut app = test_app();
        let s = &mut app.ui_state.file_browser_state;
        s.focused = BrowserPanel::Library;
        s.library_paths = vec![PathBuf::from("/only.flac")];
        s.selected_library_index = 5; // past the end
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.ui_state.file_browser_state.library_paths.len(), 1);
    }

    // ── Backspace ──

    #[test]
    fn test_backspace_navigates_up_within_home() {
        let mut app = test_app();
        let root = scratch("up");
        let sub = root.join("child");
        std::fs::create_dir_all(&sub).unwrap();
        set_dir(&mut app, root.clone());
        app.ui_state.file_browser_state.current_dir = sub.clone();
        app.refresh_file_browser();

        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.ui_state.file_browser_state.current_dir, root);
    }

    #[test]
    fn test_backspace_blocked_at_home() {
        let mut app = test_app();
        let root = scratch("home");
        set_dir(&mut app, root.clone());
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.ui_state.file_browser_state.current_dir, root);
    }

    #[test]
    fn test_backspace_ignored_in_library_panel() {
        let mut app = test_app();
        let root = scratch("lib");
        set_dir(&mut app, root.clone());
        app.ui_state.file_browser_state.focused = BrowserPanel::Library;
        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.ui_state.file_browser_state.current_dir, root);
    }

    // ── refresh_file_browser ──

    #[test]
    fn test_refresh_filters_extensions_and_sorts() {
        let mut app = test_app();
        let root = scratch("filt");
        std::fs::create_dir_all(root.join("dirA")).unwrap();
        std::fs::write(root.join("a.mp3"), b"x").unwrap();
        std::fs::write(root.join("b.flac"), b"x").unwrap();
        std::fs::write(root.join("notes.txt"), b"x").unwrap();
        std::fs::write(root.join("upper.WAV"), b"x").unwrap();
        set_dir(&mut app, root.clone());

        app.refresh_file_browser();
        let s = &app.ui_state.file_browser_state;
        let dirs: Vec<_> = s
            .fs_items
            .iter()
            .filter(|i| matches!(i, FsItem::Dir { .. }))
            .collect();
        let audios: Vec<_> = s
            .fs_items
            .iter()
            .filter(|i| matches!(i, FsItem::Audio { .. }))
            .collect();
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0].path(), root.join("dirA"));
        // a.mp3, b.flac, upper.WAV (case-insensitive); notes.txt excluded.
        assert_eq!(audios.len(), 3);
        assert!(audios.iter().any(|i| i.path() == root.join("upper.WAV")));
        assert_eq!(s.fs_items.len(), 4);
        assert_eq!(s.selected_fs_index, 0);
    }

    #[test]
    fn test_refresh_missing_dir_is_empty() {
        let mut app = test_app();
        let root = scratch("missing");
        std::fs::remove_dir_all(&root).ok();
        app.ui_state.file_browser_state.current_dir = root;
        app.refresh_file_browser();
        let s = &app.ui_state.file_browser_state;
        assert!(s.fs_items.is_empty());
    }
}
