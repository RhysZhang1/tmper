use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::ui::theme::Theme;

pub fn render_help(f: &mut Frame, theme: &Theme, scroll: usize) {
    let area = f.area();
    let margin = 1u16;
    let popup_area = Rect::new(
        margin,
        margin,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    f.render_widget(Clear, popup_area);

    let all_lines = build_lines(theme);

    let visible_rows = popup_area.height.saturating_sub(2) as usize;
    let max_scroll = all_lines.len().saturating_sub(visible_rows);
    let scroll = scroll.min(max_scroll);

    let visible: Vec<Line> = all_lines
        .into_iter()
        .skip(scroll)
        .take(visible_rows)
        .collect();

    let has_more_below = scroll < max_scroll;
    let has_more_above = scroll > 0;
    let indicator = match (has_more_above, has_more_below) {
        (true, true) => " ▲ 滚动: j/k ▼ ",
        (true, false) => " ▲ 滚动: j/k (已到底) ",
        (false, true) => " j/k 滚动 ▼ ",
        (false, false) => "",
    };
    let title = format!(" 帮助 — 按 8 或 Esc 关闭 {} ", indicator);

    let para = Paragraph::new(visible).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .style(Style::default().fg(theme.warning)),
    );

    f.render_widget(para, popup_area);
}

fn build_lines(theme: &Theme) -> Vec<Line<'static>> {
    vec![
        hdr(theme, "══════ 键位参考 ══════"),
        gap(),
        sec(theme, "▎播放控制"),
        key(theme, "Space", "播放 / 暂停"),
        key(theme, "n / p", "下一首 / 上一首"),
        key(theme, "- / =", "音量减 / 增 (±5%)"),
        key(theme, "← / →", "快退 / 快进"),
        gap(),
        sec(theme, "▎导航（Vim 风格）"),
        key(theme, "j / ↓", "向下移动"),
        key(theme, "k / ↑", "向上移动"),
        key(theme, "g g", "跳到顶部（双击 g）"),
        key(theme, "G", "跳到底部"),
        key(theme, "Ctrl+d", "向下翻半页"),
        key(theme, "Ctrl+u", "向上翻半页"),
        gap(),
        sec(theme, "▎播放列表"),
        key(theme, "Enter", "播放选中曲目"),
        key(theme, "d d", "删除当前曲目（双击 d）"),
        key(theme, "r", "切换播放模式（顺序/随机/单曲）"),
        gap(),
        sec(theme, "▎搜索"),
        key(theme, "/", "进入搜索（实时过滤）"),
        key(theme, "Esc", "清除搜索"),
        gap(),
        sec(theme, "▎命令模式（按 : 进入，Vim 风格）"),
        key(theme, ":q / :quit", "退出程序"),
        key(theme, ":help", "显示本帮助"),
        key(theme, ":version", "显示版本号"),
        key(
            theme,
            ":theme <名称>",
            "切换主题 (tokyo-night/dracula/nord/...)",
        ),
        key(theme, ":seek <秒数>", "跳转（正数前进，负数后退）"),
        key(theme, ":volume <0-100>", "设置音量"),
        key(
            theme,
            ":repeat <模式>",
            "循环模式 (sequential/shuffle/single)",
        ),
        key(
            theme,
            ":view <名称>",
            "切换视图 (player/library/lyrics/...)",
        ),
        key(theme, ":import <路径>", "导入 M3U 歌单"),
        key(theme, ":export <名称>", "导出指定歌单为 M3U"),
        gap(),
        sec(theme, "▎M3U 歌单导入/导出"),
        key(theme, ":import <路径>", "从 M3U 文件导入歌单"),
        key(theme, ":export <歌单名>", "导出歌单到 data/<name>.m3u"),
        key(theme, "e (歌单视图)", "导出当前展开的歌单"),
        gap(),
        sec(theme, "▎自定义键位"),
        key(theme, "", "编辑 config/keybindings.toml 可自定义 8 个键位"),
        key(theme, "", "play_pause next_track prev_track"),
        key(theme, "", "vol_down vol_up quit up down"),
        key(theme, "", "支持单字符、Space、Up/Down 等特殊名称"),
        gap(),
        sec(theme, "▎视图切换"),
        key(theme, "1", "播放器"),
        key(theme, "2", "曲库浏览器"),
        key(theme, "3", "全屏歌词 (KTV)"),
        key(theme, "4", "全屏频谱"),
        key(theme, "5", "播放列表管理"),
        key(theme, "6", "文件管理器"),
        key(theme, "7", "设置"),
        key(theme, "8", "本帮助"),
        gap(),
        sec(theme, "▎视图 5 — 播放列表管理"),
        key(theme, "h / l / Tab", "切换焦点 曲库↔歌单"),
        key(theme, "Enter(曲库)", "添加到展开歌单"),
        key(theme, "Enter(...)", "新建歌单"),
        key(theme, "Enter(歌单名)", "展开/收起"),
        key(theme, "Enter(歌单歌曲)", "删除该歌曲"),
        key(theme, "e", "导出当前歌单为 M3U"),
        gap(),
        sec(theme, "▎视图 6 — 文件管理器"),
        key(theme, "h / l / Tab", "切换焦点"),
        key(theme, "Enter(文件夹)", "进入文件夹"),
        key(theme, "Enter(音频)", "添加到曲库"),
        key(theme, "Backspace", "返回上级"),
        gap(),
        sec(theme, "▎歌词控制"),
        key(theme, "[ / ]", "偏移 ±0.5 秒"),
        key(theme, "{ / }", "偏移 ±2 秒"),
        key(theme, "Ctrl+r", "重置偏移"),
        gap(),
        sec(theme, "▎界面使用说明"),
        txt(
            theme,
            "视图 1: 播放器主界面 — 左侧封面+歌单, 右侧歌词+频谱+控制栏",
        ),
        txt(
            theme,
            "视图 2: 曲库浏览器 — 三栏 (艺术家→专辑→歌曲)，首次进入从数据库加载",
        ),
        txt(
            theme,
            "视图 3: 全屏歌词 — KTV 风格, 当前行高亮, 支持 LRC 增强歌词逐字时间戳",
        ),
        txt(
            theme,
            "视图 4: 全屏频谱 — 绿→黄→红渐变, 32根柱, 暂停时自动衰减",
        ),
        txt(
            theme,
            "视图 5: 歌单管理 — 左侧曲库+右侧歌单, 展开歌单后可按 e 导出 M3U",
        ),
        txt(
            theme,
            "视图 6: 文件浏览器 — 浏览本地目录, Enter 添加音频到曲库, Backspace 返回",
        ),
        txt(
            theme,
            "视图 7: 设置 — j/k 选择, Enter/l/→ 切换值, 修改即时保存到 config.toml",
        ),
        gap(),
        sec(theme, "▎配置与数据"),
        txt(
            theme,
            "配置文件: config/config.toml (主题/音量/频谱/音乐目录)",
        ),
        txt(
            theme,
            "键位文件: config/keybindings.toml (自定义键位, 9个可配置键)",
        ),
        txt(
            theme,
            "运行时数据: data/ 目录 (state.json, library.db, playlists.json)",
        ),
        gap(),
        grey(theme, "按 8 或 Esc 关闭帮助，j/k 滚动"),
    ]
}

fn hdr(theme: &Theme, text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default()
            .fg(theme.warning)
            .add_modifier(Modifier::BOLD),
    ))
}

fn sec(theme: &Theme, text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default()
            .fg(theme.primary)
            .add_modifier(Modifier::BOLD),
    ))
}

fn key(theme: &Theme, k: &str, desc: &str) -> Line<'static> {
    let width = 22usize;
    let key_part = format!("  {:<width$}", k, width = width);
    let desc_owned = desc.to_string();
    Line::from(vec![
        Span::styled(key_part, Style::default().fg(theme.success)),
        Span::styled(desc_owned, Style::default().fg(theme.text)),
    ])
}

fn txt(theme: &Theme, text: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {}", text),
        Style::default().fg(theme.secondary),
    ))
}

fn gap() -> Line<'static> {
    Line::from("")
}

fn grey(theme: &Theme, text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(theme.muted),
    ))
}
