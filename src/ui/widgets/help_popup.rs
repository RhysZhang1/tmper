use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn render_help(f: &mut Frame, scroll: usize) {
    let area = f.area();
    let margin = 1u16;
    let popup_area = Rect::new(
        margin,
        margin,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    f.render_widget(Clear, popup_area);

    let all_lines = build_lines();

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
    let title = format!(" 帮助 — 按 0 或 Esc 关闭 {} ", indicator);

    let para = Paragraph::new(visible).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .style(Style::default().fg(Color::Yellow)),
    );

    f.render_widget(para, popup_area);
}

fn build_lines() -> Vec<Line<'static>> {
    vec![
        hdr("══════ 键位参考 ══════"),
        gap(),
        sec("▎播放控制"),
        key("Space", "播放 / 暂停"),
        key("n / p", "下一首 / 上一首"),
        key("- / =", "音量减 / 增 (±5%)"),
        key("← / →", "快退 / 快进"),
        gap(),
        sec("▎导航（Vim 风格）"),
        key("j / ↓", "向下移动"),
        key("k / ↑", "向上移动"),
        key("g g", "跳到顶部（双击 g）"),
        key("G", "跳到底部"),
        key("Ctrl+d", "向下翻半页"),
        key("Ctrl+u", "向上翻半页"),
        gap(),
        sec("▎播放列表"),
        key("Enter", "播放选中曲目"),
        key("d d", "删除当前曲目（双击 d）"),
        key("r", "切换播放模式（顺序/随机/单曲）"),
        gap(),
        sec("▎搜索"),
        key("/", "进入搜索（实时过滤）"),
        key("Esc", "清除搜索"),
        gap(),
        sec("▎命令模式（按 : 进入，Vim 风格）"),
        key(":q / :quit", "退出程序"),
        key(":help", "显示本帮助"),
        key(":version", "显示版本号"),
        key(":theme <名称>", "切换主题 (tokyo-night/dracula/nord/...)"),
        key(":seek <秒数>", "跳转（正数前进，负数后退）"),
        key(":volume <0-100>", "设置音量"),
        key(":repeat <模式>", "循环模式 (sequential/shuffle/single)"),
        key(":view <名称>", "切换视图 (player/library/lyrics/...)"),
        key(":import <路径>", "导入 M3U 歌单"),
        key(":export <名称>", "导出指定歌单为 M3U"),
        gap(),
        sec("▎M3U 歌单导入/导出"),
        key(":import <路径>", "从 M3U 文件导入歌单"),
        key(":export <歌单名>", "导出歌单到 data/<name>.m3u"),
        key("e (歌单视图)", "导出当前展开的歌单"),
        gap(),
        sec("▎自定义键位"),
        key("", "编辑 config/keybindings.toml 可自定义 9 个键位"),
        key("", "play_pause stop next_track prev_track"),
        key("", "vol_down vol_up quit up down"),
        key("", "支持单字符、Space、Up/Down 等特殊名称"),
        gap(),
        sec("▎视图切换"),
        key("1", "播放器"),
        key("2", "曲库浏览器"),
        key("3", "全屏歌词 (KTV)"),
        key("4", "全屏频谱"),
        key("5", "播放列表管理"),
        key("6", "文件管理器"),
        key("7", "设置"),
        key("0", "本帮助"),
        gap(),
        sec("▎视图 5 — 播放列表管理"),
        key("h / l / Tab", "切换焦点 曲库↔歌单"),
        key("Enter(曲库)", "添加到展开歌单"),
        key("Enter(...)", "新建歌单"),
        key("Enter(歌单名)", "展开/收起"),
        key("Enter(歌单歌曲)", "删除该歌曲"),
        key("e", "导出当前歌单为 M3U"),
        gap(),
        sec("▎视图 6 — 文件管理器"),
        key("h / l / Tab", "切换焦点"),
        key("Enter(文件夹)", "进入文件夹"),
        key("Enter(音频)", "添加到曲库"),
        key("Backspace", "返回上级"),
        gap(),
        sec("▎歌词控制"),
        key("[ / ]", "偏移 ±0.5 秒"),
        key("{ / }", "偏移 ±2 秒"),
        key("Ctrl+r", "重置偏移"),
        gap(),
        sec("▎界面使用说明"),
        txt("视图 1: 播放器主界面 — 左侧封面+歌单, 右侧歌词+频谱+控制栏"),
        txt("视图 2: 曲库浏览器 — 三栏 (艺术家→专辑→歌曲)，首次进入从数据库加载"),
        txt("视图 3: 全屏歌词 — KTV 风格, 当前行高亮, 支持 LRC 增强歌词逐字时间戳"),
        txt("视图 4: 全屏频谱 — 绿→黄→红渐变, 32根柱, 暂停时自动衰减"),
        txt("视图 5: 歌单管理 — 左侧曲库+右侧歌单, 展开歌单后可按 e 导出 M3U"),
        txt("视图 6: 文件浏览器 — 浏览本地目录, Enter 添加音频到曲库, Backspace 返回"),
        txt("视图 7: 设置 — j/k 选择, Enter/l/→ 切换值, 修改即时保存到 config.toml"),
        gap(),
        sec("▎配置与数据"),
        txt("配置文件: config/config.toml (主题/音量/频谱/音乐目录)"),
        txt("键位文件: config/keybindings.toml (自定义键位, 9个可配置键)"),
        txt("运行时数据: data/ 目录 (state.json, library.db, playlists.json)"),
        gap(),
        grey("按 0 或 Esc 关闭帮助，j/k 滚动"),
    ]
}

fn hdr(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ))
}

fn sec(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn key(k: &str, desc: &str) -> Line<'static> {
    let width = 22usize;
    let key_part = format!("  {:<width$}", k, width = width);
    let desc_owned = desc.to_string();
    Line::from(vec![
        Span::styled(key_part, Style::default().fg(Color::Green)),
        Span::styled(desc_owned, Style::default().fg(Color::White)),
    ])
}

fn txt(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {}", text),
        Style::default().fg(Color::Gray),
    ))
}

fn gap() -> Line<'static> {
    Line::from("")
}

fn grey(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(Color::DarkGray),
    ))
}
