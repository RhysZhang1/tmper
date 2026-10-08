use crossterm::event::KeyEvent;

use crate::app::App;
use crate::ipc::proto::Request;
use crate::ui::theme::Theme;
use crate::ui::views::settings_view::SettingsState;
use crate::ui::ViewMode;

impl App {
    pub(super) fn handle_settings_key(&mut self, key: &KeyEvent) {
        use crossterm::event::KeyCode;
        let visualizer_before = (
            self.config.visualizer.num_bars,
            self.config.visualizer.smoothing,
        );
        let state = &mut self.ui_state.settings_state;
        let config = &mut self.config;

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if state.cursor + 1 < state.items.len() {
                    state.cursor += 1;
                }
                let vis = self.ui_state.visible_rows.get();
                if state.cursor < state.scroll {
                    state.scroll = state.cursor;
                }
                if state.cursor >= state.scroll + vis {
                    state.scroll = state.cursor.saturating_sub(vis) + 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                state.cursor = state.cursor.saturating_sub(1);
                let vis = self.ui_state.visible_rows.get();
                if state.cursor < state.scroll {
                    state.scroll = state.cursor;
                }
                if state.cursor >= state.scroll + vis {
                    state.scroll = state.cursor.saturating_sub(vis) + 1;
                }
            }
            KeyCode::Enter => {
                let last = state.items.len().saturating_sub(1);
                if state.cursor == last {
                    // Confirm row
                    Self::write_config(config);
                    self.ui_state.view.active_view = ViewMode::Player;
                    return;
                }
                // M3U: export all playlists
                if state.cursor == 17 {
                    self.export_all_playlists_m3u();
                    return;
                }
                // M3U import / keybinding items: show notification
                if (7..=14).contains(&state.cursor) || state.cursor == 16 {
                    self.ui_state.notification = Some((
                        "编辑 config/keybindings.toml 或使用 :import/:export 命令".into(),
                        std::time::Instant::now(),
                    ));
                    return;
                }
                // Section headers: skip
                if state.cursor == 6 || state.cursor == 15 {
                    return;
                }
                Self::cycle_setting(config, state, &self.key_bindings);
                self.sync_theme_from_config();
            }
            KeyCode::Char('l') | KeyCode::Right => {
                Self::cycle_setting(config, state, &self.key_bindings);
                self.sync_theme_from_config();
            }
            KeyCode::Char('h') | KeyCode::Left => {
                Self::cycle_setting_reverse(config, state, &self.key_bindings);
                self.sync_theme_from_config();
            }
            _ => {}
        }

        self.sync_visualizer_from_config(visualizer_before);
    }

    /// Export all playlists to M3U files in the data directory.
    ///
    /// The daemon writes them and reports how many landed; the sentence the
    /// user reads is composed here, from the count and the failures.
    fn export_all_playlists_m3u(&mut self) {
        if self.ui_state.playlist_state.playlists.is_empty() {
            self.ui_state.notification =
                Some(("没有歌单可以导出".into(), std::time::Instant::now()));
            return;
        }
        self.dispatch(Request::PlaylistExport { id: None });
    }

    fn cycle_setting(
        config: &mut crate::config::Config,
        state: &mut SettingsState,
        key_bindings: &crate::input::keymap::KeyBindings,
    ) {
        let idx = state.cursor;
        let last = state.items.len().saturating_sub(1);
        if idx >= last || idx == 6 || idx == 15 || (7..=14).contains(&idx) || idx == 16 || idx == 17
        {
            return; // section headers, keybinding display, M3U actions
        }

        match idx {
            0 => {
                // Theme
                let themes = [
                    "tokyo-night",
                    "dracula",
                    "nord",
                    "solarized-dark",
                    "catppuccin-mocha",
                ];
                let cur = &config.ui.theme;
                let pos = themes.iter().position(|t| *t == cur.as_str()).unwrap_or(0);
                let next = (pos + 1) % themes.len();
                config.ui.theme = themes[next].to_string();
            }
            1 => {
                // Spectrum bars
                let bars = [16, 24, 32, 40, 48, 64];
                let cur = config.visualizer.num_bars;
                let pos = bars.iter().position(|&b| b == cur).unwrap_or(2);
                let next = (pos + 1) % bars.len();
                config.visualizer.num_bars = bars[next];
            }
            2 => {
                // Smoothing
                let smooths: [f32; 3] = [0.15, 0.35, 0.55];
                let cur = config.visualizer.smoothing;
                let pos = smooths
                    .iter()
                    .position(|&s| (s - cur).abs() < 0.01)
                    .unwrap_or(1);
                let next = (pos + 1) % smooths.len();
                config.visualizer.smoothing = smooths[next];
            }
            3 => {
                // Default volume (cycles 0..1 in 0.05 steps, wraps at 1.0)
                let new_vol = ((config.playback.default_volume + 0.05) * 100.0).round() / 100.0;
                config.playback.default_volume = if new_vol >= 1.0 { 0.0 } else { new_vol };
            }
            4 => {
                // Seek step
                let steps = [5, 10, 15, 30];
                let cur = config.playback.seek_step_small_secs;
                let pos = steps.iter().position(|&s| s == cur).unwrap_or(0);
                let next = (pos + 1) % steps.len();
                config.playback.seek_step_small_secs = steps[next];
            }
            5 => {
                // Show cover art
                config.ui.show_cover_art = !config.ui.show_cover_art;
            }
            _ => {}
        }

        // Rebuild display
        crate::ui::views::settings_view::rebuild_settings(state, config, key_bindings);

        // Persist
        Self::write_config(config);
    }

    fn cycle_setting_reverse(
        config: &mut crate::config::Config,
        state: &mut SettingsState,
        key_bindings: &crate::input::keymap::KeyBindings,
    ) {
        let idx = state.cursor;
        let last = state.items.len().saturating_sub(1);
        if idx >= last || idx == 6 || idx == 15 || (7..=14).contains(&idx) || idx == 16 || idx == 17
        {
            return; // section headers, keybinding display, M3U actions
        }

        match idx {
            0 => {
                let themes = [
                    "tokyo-night",
                    "dracula",
                    "nord",
                    "solarized-dark",
                    "catppuccin-mocha",
                ];
                let cur = &config.ui.theme;
                let pos = themes.iter().position(|t| *t == cur.as_str()).unwrap_or(0);
                let prev = if pos == 0 { themes.len() - 1 } else { pos - 1 };
                config.ui.theme = themes[prev].to_string();
            }
            1 => {
                let bars = [16, 24, 32, 40, 48, 64];
                let cur = config.visualizer.num_bars;
                let pos = bars.iter().position(|&b| b == cur).unwrap_or(2);
                let prev = if pos == 0 { bars.len() - 1 } else { pos - 1 };
                config.visualizer.num_bars = bars[prev];
            }
            2 => {
                let smooths: [f32; 3] = [0.15, 0.35, 0.55];
                let cur = config.visualizer.smoothing;
                let pos = smooths
                    .iter()
                    .position(|&s| (s - cur).abs() < 0.01)
                    .unwrap_or(1);
                let prev = if pos == 0 { smooths.len() - 1 } else { pos - 1 };
                config.visualizer.smoothing = smooths[prev];
            }
            _ => {
                // For booleans and others, just cycle forward (simpler)
                Self::cycle_setting(config, state, key_bindings);
            }
        }

        crate::ui::views::settings_view::rebuild_settings(state, config, key_bindings);
        Self::write_config(config);
    }

    /// Reload the active theme into UiState if the config theme changed.
    /// Called after any settings cycle that may have switched themes.
    fn sync_theme_from_config(&mut self) {
        if self.ui_state.theme.name != self.config.ui.theme {
            self.ui_state.theme = Theme::load(&self.config.ui.theme);
        }
    }

    /// Rebuild the spectrum pipeline as soon as a live visualizer setting is
    /// changed. The FFT worker captures these values when it starts, so merely
    /// updating `Config` is not enough.
    pub(crate) fn sync_visualizer_from_config(&mut self, previous: (u32, f32)) {
        let current = (
            self.config.visualizer.num_bars,
            self.config.visualizer.smoothing,
        );
        if current == previous {
            return;
        }

        self.ui_state.visualizer_data = vec![0.0; current.0 as usize];
        self.player_bars = self.ui_state.visualizer_data.clone();
        self.dispatch(Request::SetFftParams {
            num_bars: current.0,
            smoothing: current.1,
        });
    }

    pub(crate) fn write_config(config: &crate::config::Config) {
        let path = crate::paths::config_dir().join("config.toml");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match toml::to_string_pretty(config) {
            Ok(content) => {
                if let Err(e) = std::fs::write(&path, &content) {
                    tracing::warn!("Failed to write config: {e}");
                } else {
                    tracing::info!("Config written to {:?}", path);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to serialize config: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::handlers::test_support::{pl, seed_playlists, seed_settings, test_app};
    use crate::ui::views::settings_view::SettingItem;
    use crate::ui::ViewMode;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_settings_key(&key(code));
    }

    fn item(state: &SettingsState, i: usize) -> &SettingItem {
        &state.items[i]
    }

    // ── rebuild_settings ──

    #[test]
    fn test_rebuild_settings_layout() {
        let mut app = test_app();
        seed_settings(&mut app);
        let s = &app.ui_state.settings_state;
        assert_eq!(s.items.len(), 19);
        assert_eq!(item(s, 0).name, "主题");
        assert_eq!(item(s, 0).value, "tokyo-night");
        assert_eq!(item(s, 1).value, "32");
        assert_eq!(item(s, 2).value, "0.35");
        assert_eq!(item(s, 3).value, "80%");
        assert_eq!(item(s, 4).value, "5 秒");
        assert_eq!(item(s, 5).value, "是");
    }

    // ── Navigation ──

    #[test]
    fn test_j_k_navigation_clamps() {
        let mut app = test_app();
        seed_settings(&mut app);
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('j'));
        }
        assert_eq!(app.ui_state.settings_state.cursor, 3);
        for _ in 0..20 {
            press(&mut app, KeyCode::Char('j'));
        }
        assert_eq!(app.ui_state.settings_state.cursor, 18); // last row
        press(&mut app, KeyCode::Char('j')); // clamps
        assert_eq!(app.ui_state.settings_state.cursor, 18);
        for _ in 0..25 {
            press(&mut app, KeyCode::Char('k'));
        }
        assert_eq!(app.ui_state.settings_state.cursor, 0);
        press(&mut app, KeyCode::Char('k')); // saturates
        assert_eq!(app.ui_state.settings_state.cursor, 0);
    }

    // ── cycle_setting forward / reverse ──

    #[test]
    fn test_cycle_theme_forward_and_back() {
        let _guard = crate::paths::config_file_lock();
        let mut app = test_app();
        seed_settings(&mut app);
        app.ui_state.settings_state.cursor = 0;
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.config.ui.theme, "dracula");
        assert_eq!(item(&app.ui_state.settings_state, 0).value, "dracula");
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.config.ui.theme, "tokyo-night");
    }

    #[test]
    fn test_cycle_theme_wraps_around() {
        let _guard = crate::paths::config_file_lock();
        let mut app = test_app();
        seed_settings(&mut app);
        app.ui_state.settings_state.cursor = 0;
        for _ in 0..5 {
            press(&mut app, KeyCode::Char('l'));
        }
        assert_eq!(app.config.ui.theme, "tokyo-night"); // full cycle
    }

    #[test]
    fn test_cycle_bars_forward_and_reverse() {
        let _guard = crate::paths::config_file_lock();
        let mut app = test_app();
        seed_settings(&mut app);
        app.ui_state.settings_state.cursor = 1;
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.config.visualizer.num_bars, 40);
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.config.visualizer.num_bars, 32);
    }

    #[test]
    fn test_cycle_smoothing() {
        let _guard = crate::paths::config_file_lock();
        let mut app = test_app();
        seed_settings(&mut app);
        app.ui_state.settings_state.cursor = 2;
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.config.visualizer.smoothing, 0.55);
        assert_eq!(item(&app.ui_state.settings_state, 2).value, "0.55");
    }

    #[test]
    fn test_cycle_volume_wraps_to_zero() {
        let _guard = crate::paths::config_file_lock();
        let mut app = test_app();
        seed_settings(&mut app);
        app.ui_state.settings_state.cursor = 3;
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.config.playback.default_volume, 0.85);
        // Force near the top of the range, then wrap back to 0.
        app.config.playback.default_volume = 0.95;
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.config.playback.default_volume, 0.0);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.config.playback.default_volume, 0.05);
    }

    #[test]
    fn test_cycle_seek_step() {
        let _guard = crate::paths::config_file_lock();
        let mut app = test_app();
        seed_settings(&mut app);
        app.ui_state.settings_state.cursor = 4;
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.config.playback.seek_step_small_secs, 10);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.config.playback.seek_step_small_secs, 15);
        assert_eq!(item(&app.ui_state.settings_state, 4).value, "15 秒");
    }

    #[test]
    fn test_cycle_show_cover_art_toggles() {
        let _guard = crate::paths::config_file_lock();
        let mut app = test_app();
        seed_settings(&mut app);
        app.ui_state.settings_state.cursor = 5;
        assert!(app.config.ui.show_cover_art);
        press(&mut app, KeyCode::Char('l'));
        assert!(!app.config.ui.show_cover_art);
        assert_eq!(item(&app.ui_state.settings_state, 5).value, "否");
        press(&mut app, KeyCode::Char('l'));
        assert!(app.config.ui.show_cover_art);
    }

    // ── Skip rows ──

    #[test]
    fn test_header_and_keybinding_rows_are_noops() {
        let mut app = test_app();
        seed_settings(&mut app);
        // Section headers must not cycle.
        for cursor in [6usize, 15] {
            app.ui_state.settings_state.cursor = cursor;
            let theme = app.config.ui.theme.clone();
            press(&mut app, KeyCode::Char('l'));
            assert_eq!(app.config.ui.theme, theme, "cursor {cursor} must not cycle");
        }
        // Keybinding display + M3U action rows must not cycle.
        for cursor in 7..=17 {
            app.ui_state.settings_state.cursor = cursor;
            let bars = app.config.visualizer.num_bars;
            press(&mut app, KeyCode::Char('l'));
            assert_eq!(
                app.config.visualizer.num_bars, bars,
                "cursor {cursor} must not cycle"
            );
        }
    }

    // ── Enter actions ──

    #[test]
    fn test_confirm_row_returns_to_player() {
        let _guard = crate::paths::config_file_lock();
        let mut app = test_app();
        seed_settings(&mut app);
        app.ui_state.view.active_view = ViewMode::Settings;
        app.ui_state.settings_state.cursor = 18; // last (confirm) row
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.ui_state.view.active_view, ViewMode::Player);
    }

    #[test]
    fn test_enter_header_row_is_noop() {
        let mut app = test_app();
        seed_settings(&mut app);
        app.ui_state.view.active_view = ViewMode::Settings;
        app.ui_state.settings_state.cursor = 6;
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.ui_state.view.active_view, ViewMode::Settings);
    }

    #[test]
    fn test_enter_keybinding_row_notifies() {
        let mut app = test_app();
        seed_settings(&mut app);
        app.ui_state.settings_state.cursor = 7;
        press(&mut app, KeyCode::Enter);
        assert!(app.ui_state.notification.is_some());
    }

    #[test]
    fn test_enter_export_all_empty_notifies() {
        let mut app = test_app();
        seed_settings(&mut app);
        app.ui_state.settings_state.cursor = 17;
        press(&mut app, KeyCode::Enter);
        assert!(app.ui_state.notification.is_some());
    }

    #[test]
    fn test_enter_export_all_writes_m3u_files() {
        let mut app = test_app();
        seed_settings(&mut app);
        std::fs::create_dir_all(crate::paths::data_dir()).unwrap();
        seed_playlists(
            &mut app,
            vec![pl("P1", &["/a.flac"]), pl("P2", &["/b.flac"])],
        );
        app.ui_state.settings_state.cursor = 17;
        press(&mut app, KeyCode::Enter);
        let msg = app
            .ui_state
            .notification
            .as_ref()
            .map(|(m, _)| m.clone())
            .unwrap_or_default();
        assert!(msg.contains("已导出 2/2"), "unexpected: {msg}");
        assert!(crate::paths::data_dir().join("P1.m3u").exists());
        assert!(crate::paths::data_dir().join("P2.m3u").exists());
        std::fs::remove_file(crate::paths::data_dir().join("P1.m3u")).ok();
        std::fs::remove_file(crate::paths::data_dir().join("P2.m3u")).ok();
    }

    #[test]
    fn test_enter_on_theme_cycles_and_persists() {
        // Held across the press *and* the read: every other test that cycles a
        // setting writes this same file, and one of them landing in between
        // would leave `theme = "nord"` (or a half-truncated file) here.
        let _guard = crate::paths::config_file_lock();
        let mut app = test_app();
        seed_settings(&mut app);
        app.ui_state.settings_state.cursor = 0;
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.config.ui.theme, "dracula");
        // cycle_setting persists through write_config to the test-isolated dir.
        let path = crate::paths::config_dir().join("config.toml");
        let content = std::fs::read_to_string(&path).expect("config written");
        assert!(content.contains("theme = \"dracula\""));
    }
}
