use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn render_help(f: &mut Frame) {
    let area = f.area();
    let margin = 1u16;
    let popup_area = Rect::new(
        margin,
        margin,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    f.render_widget(Clear, popup_area);

    // Two-column layout
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(popup_area);

    render_keys_column(f, columns[0]);
    render_guide_column(f, columns[1]);
}

fn render_keys_column(f: &mut Frame, area: Rect) {
    let lines = vec![
        hdr("══════ 键位参考 ══════"),
        gap(),
        sec("▎播放控制"),
        key("Space", "播放 / 暂停"),
        key("n / p", "下一首 / 上一首"),
        key("- / =", "音量减 / 增 (±5%)"),
        key("← / →", "快退 / 快进 5 秒"),
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
        key("r", "切换播放模式（顺序/随机/单曲循环）"),
        gap(),
        sec("▎搜索"),
        key("/", "进入搜索（实时过滤）"),
        key("Esc", "清除搜索"),
        key("Backspace", "删除搜索字符"),
        gap(),
        sec("▎视图切换"),
        key("1", "播放器"),
        key("2", "曲库浏览器"),
        key("3", "全屏歌词 (KTV)"),
        key("4", "全屏频谱 (鱼缸)"),
        key("5", "播放列表管理"),
        key("6", "文件管理器"),
        key("7", "设置"),
        key("0", "本帮助"),
        gap(),
        sec("▎视图 5（播放列表管理）"),
        key("h / l / Tab", "切换焦点 曲库↔歌单"),
        key("Enter(曲库)", "添加到展开歌单"),
        key("Enter(...)", "新建歌单"),
        key("Enter(歌单名)", "展开/收起"),
        key("Enter(歌单歌曲)", "删除该歌曲"),
        gap(),
        sec("▎视图 6（文件管理器）"),
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
        sec("▎其他"),
        key("q", "退出程序"),
        grey("按 0 或 Esc 关闭"),
    ];

    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" 快捷键 ")
            .style(Style::default().fg(Color::Yellow)),
    );
    f.render_widget(para, area);
}

fn render_guide_column(f: &mut Frame, area: Rect) {
    let lines = vec![
        hdr("══════ 界面使用说明 ══════"),
        gap(),
        sec("▎视图 1 — 播放器（主界面）"),
        txt("左侧上半部分为封面图区域，支持内嵌封面（chafa 风格半块字符渲染）。无封面时显示曲目标题、艺术家、播放进度。"),
        txt("左侧下半部分为歌单侧栏，显示已保存的歌单列表，j/k 导航、Enter 展开/收起歌单或播放歌曲。"),
        txt("右侧依次显示：歌词面板、频谱可视化、控制栏（播放时间、音量、循环/随机模式）。"),
        gap(),
        sec("▎视图 2 — 曲库浏览器"),
        txt("三栏布局：艺术家 → 专辑 → 歌曲。"),
        txt("首次进入自动从数据库加载艺术家列表。若数据库为空，需先在视图 6 中将音乐目录加入曲库，或配置 config.toml 中的 music_dirs 后重启扫描。"),
        txt("h / l 切换焦点栏，j / k 上下选择。选中艺术家后自动刷新其专辑列表，选中专辑后自动刷新歌曲列表。"),
        txt("在歌曲栏按 Enter 即可播放。"),
        gap(),
        sec("▎视图 3 — 全屏歌词"),
        txt("KTV 风格全屏歌词显示，当前行高亮青色加粗，已唱过的行渐变为深灰，未唱行浅灰。"),
        txt("支持标准 LRC 和增强 LRC（逐字时间戳）。歌词文件 (.lrc) 需与音频文件同名同目录。"),
        txt("支持编码自动检测：UTF-8 → GBK → Shift-JIS，中文歌词可直接加载。"),
        txt("可用 [ ] { } Ctrl+r 微调歌词同步偏移，偏移量会在状态栏显示。"),
        gap(),
        sec("▎视图 4 — 全屏频谱"),
        txt("鱼缸模式全屏频谱可视化，32 根柱子填满整个屏幕。"),
        txt("绿→黄→红渐变配色，统一缓慢平滑衰减（α=0.22），类似 cava 的流畅效果。"),
        txt("频谱范围 60Hz–8kHz，对数频率分桶。暂停时柱状图自动衰减至零。"),
        txt("配置文件中可调整柱数、刷新率、平滑系数、字符集等参数。"),
        gap(),
        sec("▎视图 5 — 播放列表管理器"),
        txt("两栏布局：左侧'曲库'面板显示已添加的音频文件，右侧'歌单'面板管理歌单。"),
        txt("歌单面板第一行 '...' 用于新建歌单：Enter 进入输入模式，输入名称后 Enter 确认，Esc 取消。"),
        txt("在歌单名上 Enter 展开/收起歌曲列表。展开后，在曲库面板选中歌曲按 Enter 可添加到当前展开的歌单。"),
        txt("在歌单内歌曲上按 Enter 会将该歌曲从歌单删除（不删除文件）。"),
        txt("所有修改自动保存到 data/playlists.json，下次启动恢复。"),
        gap(),
        sec("▎视图 6 — 文件管理器"),
        txt("两栏布局：左侧'曲库'面板列出已索引的音频文件，右侧'文件系统'浏览本地目录。"),
        txt("在文件系统中，文件夹按 Enter 进入，音频文件按 Enter 添加到曲库（仅记录路径，不复制文件）。"),
        txt("Backspace 返回上级目录（不会超出家目录范围）。"),
        txt("在曲库面板中按 Enter 可从曲库删除记录（不删除原文件）。"),
        txt("添加的音频会自动解析元数据并显示在播放列表中。"),
        gap(),
        sec("视图 7 — 设置"),
        txt("配置各种参数：主题、频谱柱数和色彩、默认音量、快进退步长等。"),
        txt("j / k 上下选择，Enter 或 l / → 切换到下一个值，h / ← 切换到上一个值。"),
        txt("修改立即生效并自动保存到 config/config.toml。"),
        gap(),
        sec("▎配置与数据"),
        txt("配置文件：config/config.toml（主题、音量、频谱参数、音乐目录等）。"),
        txt("快捷键：config/keybindings.toml（自定义键位）。"),
        txt("运行时数据：data/ 目录（state.json 保存状态、library.db 曲库索引、playlists.json 歌单数据）。"),
        grey("按 0 或 Esc 关闭帮助"),
    ];

    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" 使用说明 ")
                .style(Style::default().fg(Color::Cyan)),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(para, area);
}

// ── helpers ──

fn hdr(text: &str) -> Line<'_> {
    Line::from(Span::styled(
        text,
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ))
}

fn sec(text: &str) -> Line<'_> {
    Line::from(Span::styled(
        text,
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

fn txt(text: &str) -> Line<'_> {
    Line::from(Span::styled(
        format!("  {}", text),
        Style::default().fg(Color::Gray),
    ))
}

fn gap() -> Line<'static> {
    Line::from("")
}

fn grey(text: &str) -> Line<'_> {
    Line::from(Span::styled(text, Style::default().fg(Color::DarkGray)))
}
