use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn render_help(f: &mut Frame) {
    let area = f.area();

    // Take almost full screen
    let margin = 2u16;
    let popup_area = Rect::new(
        margin,
        margin,
        area.width.saturating_sub(margin * 2),
        area.height.saturating_sub(margin * 2),
    );

    f.render_widget(Clear, popup_area);

    let help_text = vec![
        Line::from(Span::styled(
            "═══════════════ termusic 键盘参考 ═══════════════",
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "▎播放控制",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  Space        播放 / 暂停"),
        Line::from("  n / p        下一首 / 上一首"),
        Line::from("  - / =        音量减小 / 增大（每次 5%）"),
        Line::from("  ← / →        快退 / 快进 5 秒"),
        Line::from("  Shift+←/→    快退 / 快进 30 秒"),
        Line::from(""),
        Line::from(Span::styled(
            "▎导航（Vim 风格）",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  j / ↓        向下移动光标"),
        Line::from("  k / ↑        向上移动光标"),
        Line::from("  g g          跳到列表顶部（双击 g）"),
        Line::from("  G            跳到列表底部"),
        Line::from("  Ctrl+d       向下翻半页"),
        Line::from("  Ctrl+u       向上翻半页"),
        Line::from(""),
        Line::from(Span::styled(
            "▎播放列表操作",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  Enter        播放选中的曲目"),
        Line::from("  d d          从列表中删除当前曲目（双击 d）"),
        Line::from("  r            切换循环模式（关 → 单曲 → 列表 → 关）"),
        Line::from("  R            切换随机播放"),
        Line::from(""),
        Line::from(Span::styled(
            "▎搜索",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  /            进入搜索模式（实时过滤）"),
        Line::from("  Esc          清除搜索 / 退出搜索模式"),
        Line::from("  键入字符     输入搜索关键词（匹配标题和艺术家）"),
        Line::from("  Backspace    删除搜索字符"),
        Line::from(""),
        Line::from(Span::styled(
            "▎视图切换",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  1            播放器视图（默认：频谱 + 歌词 + 播放列表）"),
        Line::from("  3            全屏歌词视图（KTV 风格，居中显示）"),
        Line::from("  4            全屏频谱视图（鱼缸模式，铺满终端）"),
        Line::from("  0            显示 / 关闭本帮助"),
        Line::from(""),
        Line::from(Span::styled(
            "▎歌词控制",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  [ / ]        歌词提前 / 延后 0.5 秒"),
        Line::from("  { / }        歌词提前 / 延后 2 秒"),
        Line::from("  Ctrl+r       重置歌词偏移为 0"),
        Line::from("  歌词文件      将同名 .lrc 文件放在音频旁自动加载"),
        Line::from("  支持编码      UTF-8 / GBK / Shift-JIS 自动识别"),
        Line::from(""),
        Line::from(Span::styled(
            "▎命令行",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  termusic                      启动交互模式"),
        Line::from("  termusic play <文件>          播放指定的音频文件"),
        Line::from("  termusic play <目录>          播放目录中所有支持的音频"),
        Line::from(""),
        Line::from(Span::styled(
            "▎配置文件",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  config/config.toml            主配置文件（首次运行自动生成）"),
        Line::from("  config/keybindings.toml       自定义快捷键"),
        Line::from("  themes/*.toml                 主题文件（5 套内置主题）"),
        Line::from(""),
        Line::from(Span::styled(
            "▎其他",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  q            退出程序"),
        Line::from("  Ctrl+l       强制刷新屏幕"),
        Line::from("  日志文件      data/termusic.log（调试用）"),
        Line::from(""),
        Line::from(Span::styled(
            "按 0 或 Esc 关闭帮助",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let para = Paragraph::new(help_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" 帮助 — 按 0 关闭 ")
            .style(Style::default().fg(Color::Yellow)),
    );

    f.render_widget(para, popup_area);
}
