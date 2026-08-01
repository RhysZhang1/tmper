use crossterm::event::KeyEvent;

use crate::app::App;
use crate::ui::views::settings_view::SettingsState;
use crate::ui::ViewMode;

impl App {
    pub(super) fn handle_settings_key(&mut self, key: &KeyEvent) {
        use crossterm::event::KeyCode;
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
                if state.cursor == 20 {
                    self.export_all_playlists_m3u();
                    return;
                }
                // M3U import / keybinding items: show notification
                if (10..=17).contains(&state.cursor) || state.cursor == 19 {
                    self.ui_state.notification = Some((
                        "编辑 config/keybindings.toml 或使用 :import/:export 命令".into(),
                        std::time::Instant::now(),
                    ));
                    return;
                }
                // Section headers: skip
                if state.cursor == 9 || state.cursor == 18 {
                    return;
                }
                Self::cycle_setting(config, state, &self.key_bindings);
            }
            KeyCode::Char('l') | KeyCode::Right => {
                Self::cycle_setting(config, state, &self.key_bindings);
            }
            KeyCode::Char('h') | KeyCode::Left => {
                Self::cycle_setting_reverse(config, state, &self.key_bindings);
            }
            _ => {}
        }
    }

    /// Export all playlists to M3U files in the data directory.
    fn export_all_playlists_m3u(&mut self) {
        let playlists = self.ui_state.playlist_state.playlists.clone();
        if playlists.is_empty() {
            self.ui_state.notification =
                Some(("没有歌单可以导出".into(), std::time::Instant::now()));
            return;
        }
        let mut count = 0usize;
        for pl_data in &playlists {
            let export_path = crate::paths::data_dir().join(format!("{}.m3u", pl_data.name));
            if crate::library::playlist_manager::export_m3u(pl_data, &export_path).is_ok() {
                count += 1;
            }
        }
        self.ui_state.notification = Some((
            format!("已导出 {count}/{} 个歌单到 data/", playlists.len()),
            std::time::Instant::now(),
        ));
    }

    fn cycle_setting(
        config: &mut crate::config::Config,
        state: &mut SettingsState,
        key_bindings: &crate::input::keymap::KeyBindings,
    ) {
        let idx = state.cursor;
        let last = state.items.len().saturating_sub(1);
        if idx >= last
            || idx == 9
            || idx == 18
            || (10..=17).contains(&idx)
            || idx == 19
            || idx == 20
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
                // Color scheme
                let schemes = ["gradient", "solid", "fire", "ice"];
                let cur = &config.visualizer.color_scheme;
                let pos = schemes.iter().position(|s| *s == cur.as_str()).unwrap_or(0);
                let next = (pos + 1) % schemes.len();
                config.visualizer.color_scheme = schemes[next].to_string();
            }
            4 => {
                // Default volume (cycles 0..1 in 0.05 steps, wraps at 1.0)
                let new_vol = ((config.playback.default_volume + 0.05) * 100.0).round() / 100.0;
                config.playback.default_volume = if new_vol >= 1.0 { 0.0 } else { new_vol };
            }
            5 => {
                // Seek step
                let steps = [5, 10, 15, 30];
                let cur = config.playback.seek_step_small_secs;
                let pos = steps.iter().position(|&s| s == cur).unwrap_or(0);
                let next = (pos + 1) % steps.len();
                config.playback.seek_step_small_secs = steps[next];
            }
            6 => {
                // Scan on startup
                config.library.scan_on_startup = !config.library.scan_on_startup;
            }
            7 => {
                // Gapless
                config.playback.gapless = !config.playback.gapless;
            }
            8 => {
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
        if idx >= last
            || idx == 9
            || idx == 18
            || (10..=17).contains(&idx)
            || idx == 19
            || idx == 20
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
            3 => {
                let schemes = ["gradient", "solid", "fire", "ice"];
                let cur = &config.visualizer.color_scheme;
                let pos = schemes.iter().position(|s| *s == cur.as_str()).unwrap_or(0);
                let prev = if pos == 0 { schemes.len() - 1 } else { pos - 1 };
                config.visualizer.color_scheme = schemes[prev].to_string();
            }
            _ => {
                // For booleans and others, just cycle forward (simpler)
                Self::cycle_setting(config, state, key_bindings);
            }
        }

        crate::ui::views::settings_view::rebuild_settings(state, config, key_bindings);
        Self::write_config(config);
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
