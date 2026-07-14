use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

use crate::config::Config;
use crate::input::keymap::KeyBindings;

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
            name: "频谱色彩".into(),
            value: config.visualizer.color_scheme.clone(),
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
            name: "启动扫描".into(),
            value: if config.library.scan_on_startup {
                "是".into()
            } else {
                "否".into()
            },
        },
        SettingItem {
            name: "无缝播放".into(),
            value: if config.playback.gapless {
                "开启".into()
            } else {
                "关闭".into()
            },
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

pub fn render_settings_view(f: &mut Frame, area: Rect, state: &SettingsState) {
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
                        .bg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                };
                ListItem::new(Line::from(Span::styled(
                    format!("  {:^50}", item.name),
                    style,
                )))
            } else {
                let val_style = if is_cursor {
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Green)
                };

                let line = Line::from(vec![
                    Span::styled(
                        format!("  {:<24}", item.name),
                        Style::default()
                            .fg(Color::Cyan)
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
            .border_style(Style::default().fg(Color::Yellow)),
    );
    f.render_widget(list, area);
}
