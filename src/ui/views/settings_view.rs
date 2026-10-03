use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

use crate::config::Config;
use crate::input::keymap::KeyBindings;
use crate::ui::theme::Theme;

/// One row in the settings list.
pub struct SettingItem {
    pub name: String,
    pub value: String,
}

#[derive(Default)]
pub struct SettingsState {
    pub items: Vec<SettingItem>,
    pub cursor: usize,
    pub scroll: usize,
}

/// Rebuild the flat settings list from current config values.
pub fn rebuild_settings(state: &mut SettingsState, config: &Config, key_bindings: &KeyBindings) {
    let items = vec![
        SettingItem {
            name: "主题".into(),
            value: config.ui.theme.clone(),
        },
        SettingItem {
            name: "频谱柱数".into(),
            value: config.visualizer.num_bars.to_string(),
        },
        SettingItem {
            name: "频谱平滑度".into(),
            value: format!("{:.2}", config.visualizer.smoothing),
        },
        SettingItem {
            name: "默认音量".into(),
            value: format!("{:.0}%", config.playback.default_volume * 100.0),
        },
        SettingItem {
            name: "快进退步长".into(),
            value: format!("{} 秒", config.playback.seek_step_small_secs),
        },
        SettingItem {
            name: "显示封面图".into(),
            value: if config.ui.show_cover_art {
                "是".into()
            } else {
                "否".into()
            },
        },
        // ── Keybinding display items ──
        SettingItem {
            name: "── 键位配置 ──".into(),
            value: "config/keybindings.toml".into(),
        },
        SettingItem {
            name: "  播放/暂停".into(),
            value: key_bindings.play_pause.clone(),
        },
        SettingItem {
            name: "  下一首".into(),
            value: key_bindings.next_track.clone(),
        },
        SettingItem {
            name: "  上一首".into(),
            value: key_bindings.prev_track.clone(),
        },
        SettingItem {
            name: "  音量减".into(),
            value: key_bindings.vol_down.clone(),
        },
        SettingItem {
            name: "  音量增".into(),
            value: key_bindings.vol_up.clone(),
        },
        SettingItem {
            name: "  退出".into(),
            value: key_bindings.quit.clone(),
        },
        SettingItem {
            name: "  上移".into(),
            value: key_bindings.up.clone(),
        },
        SettingItem {
            name: "  下移".into(),
            value: key_bindings.down.clone(),
        },
        // ── M3U actions ──
        SettingItem {
            name: "── 歌单导入导出 ──".into(),
            value: "Enter 执行".into(),
        },
        SettingItem {
            name: "  导入 M3U 歌单".into(),
            value: "使用 :import <路径>".into(),
        },
        SettingItem {
            name: "  导出所有歌单".into(),
            value: "保存到 data/*.m3u".into(),
        },
        SettingItem {
            name: "── 确认并返回 ──".into(),
            value: String::new(),
        },
    ];
    state.items = items;
}

pub fn render_settings_view(f: &mut Frame, area: Rect, theme: &Theme, state: &SettingsState) {
    let vis_h = area.height.saturating_sub(2) as usize;
    let total = state.items.len();
    let start = state.scroll.min(total.saturating_sub(1));
    let end = (start + vis_h).min(total);

    let is_confirm_row = |i: usize| -> bool { i == state.items.len().saturating_sub(1) };

    let items: Vec<ListItem> = (start..end)
        .map(|i| {
            let item = &state.items[i];
            let is_cursor = i == state.cursor;
            let confirm = is_confirm_row(i);

            if confirm {
                let style = if is_cursor {
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme.success)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(theme.success)
                        .add_modifier(Modifier::BOLD)
                };
                ListItem::new(Line::from(Span::styled(
                    format!("  {:^50}", item.name),
                    style,
                )))
            } else {
                let val_style = if is_cursor {
                    Style::default()
                        .fg(theme.text)
                        .bg(theme.muted)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.success)
                };

                let line = Line::from(vec![
                    Span::styled(
                        format!("  {:<24}", item.name),
                        Style::default()
                            .fg(theme.primary)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(&item.value, val_style),
                ]);
                ListItem::new(line)
            }
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" 设置 -- j/k 导航  Enter/←→ 修改  q/7 返回 ")
            .border_style(Style::default().fg(theme.warning)),
    );
    f.render_widget(list, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::input::keymap::KeyBindings;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render(state: &SettingsState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| render_settings_view(f, f.area(), &Theme::default(), state))
            .unwrap();
        let buf = terminal.backend().buffer();
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn test_rebuild_settings_flat_layout() {
        let config = Config::default();
        let keys = KeyBindings::default();
        let mut state = SettingsState::default();
        rebuild_settings(&mut state, &config, &keys);
        assert_eq!(state.items.len(), 19);
        assert_eq!(state.items[0].name, "主题");
        assert_eq!(state.items[0].value, "tokyo-night");
        assert_eq!(state.items[1].value, "32");
        assert_eq!(state.items[2].value, "0.35");
        assert_eq!(state.items[3].value, "80%");
        assert_eq!(state.items[4].value, "5 秒");
        assert_eq!(state.items[5].value, "是");
        // Keybinding display rows carry non-empty default bindings.
        assert!(!state.items[7].value.is_empty(), "keybinding row populated");
        // Last row is the confirm row.
        assert_eq!(state.items[18].name, "── 确认并返回 ──");
    }

    #[test]
    fn test_rebuild_settings_reflects_config_values() {
        let mut config = Config::default();
        config.ui.theme = "dracula".into();
        config.visualizer.num_bars = 64;
        config.visualizer.smoothing = 0.55;
        config.playback.default_volume = 0.5;
        config.playback.seek_step_small_secs = 30;
        config.ui.show_cover_art = false;
        let keys = KeyBindings::default();
        let mut state = SettingsState::default();
        rebuild_settings(&mut state, &config, &keys);
        assert_eq!(state.items[0].value, "dracula");
        assert_eq!(state.items[1].value, "64");
        assert_eq!(state.items[2].value, "0.55");
        assert_eq!(state.items[3].value, "50%");
        assert_eq!(state.items[4].value, "30 秒");
        assert_eq!(state.items[5].value, "否");
    }

    #[test]
    fn test_render_settings_smoke() {
        let mut state = SettingsState::default();
        rebuild_settings(&mut state, &Config::default(), &KeyBindings::default());
        state.cursor = 0;
        let out = render(&state);
        // Block title and first rows render. (ratatui pads each CJK char with a
        // continuation cell, so assert on single characters, not multi-char strings.)
        assert!(out.contains('设'), "title rendered");
        assert!(out.contains("tokyo-night"), "theme row rendered");
        // Scroll clamps: cursor beyond the list end renders the last (confirm) row.
        state.cursor = 99;
        state.scroll = 99;
        let out = render(&state);
        assert!(out.contains('确'), "confirm row rendered after clamp");
    }
}
