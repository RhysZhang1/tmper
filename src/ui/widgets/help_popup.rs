use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
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

    let lines = vec![
        section_header("══════════════════ termusic 键位参考 ══════════════════"),
        line("", Color::White, false),
        section_header("▎播放控制"),
        line("  Space              播放 / 暂停", Color::White, false),
        line("  n / p              下一首 / 上一首", Color::White, false),
        line(
            "  - / =              音量减小 / 增大（每次 5%）",
            Color::White,
            false,
        ),
        line("  ← / →              快退 / 快进 5 秒", Color::White, false),
        line(
            "  Shift+←/→          快退 / 快进 30 秒",
            Color::White,
            false,
        ),
        line("", Color::White, false),
        section_header("▎导航（Vim 风格）"),
        line("  j / ↓              向下移动光标", Color::White, false),
        line("  k / ↑              向上移动光标", Color::White, false),
        line(
            "  g g                跳到列表顶部（双击 g）",
            Color::White,
            false,
        ),
        line("  G                  跳到列表底部", Color::White, false),
        line("  Ctrl+d             向下翻半页", Color::White, false),
        line("  Ctrl+u             向上翻半页", Color::White, false),
        line("", Color::White, false),
        section_header("▎播放列表操作"),
        line("  Enter              播放选中的曲目", Color::White, false),
        line(
            "  d d                从列表中删除当前曲目（双击 d）",
            Color::White,
            false,
        ),
        line(
            "  r                  切换循环模式（关 → 单曲 → 列表 → 关）",
            Color::White,
            false,
        ),
        line("  R                  切换随机播放", Color::White, false),
        line("", Color::White, false),
        section_header("▎搜索"),
        line(
            "  /                  进入搜索模式（实时过滤）",
            Color::White,
            false,
        ),
        line(
            "  Esc                清除搜索 / 退出搜索模式",
            Color::White,
            false,
        ),
        line("  Backspace          删除搜索字符", Color::White, false),
        line("", Color::White, false),
        section_header("▎视图切换"),
        line("  1                  播放器视图", Color::White, false),
        line("  2                  曲库浏览器", Color::White, false),
        line(
            "  3                  全屏歌词视图（KTV 风格）",
            Color::White,
            false,
        ),
        line(
            "  4                  全屏频谱视图（鱼缸模式）",
            Color::White,
            false,
        ),
        line("  5                  播放列表管理器", Color::White, false),
        line("  6                  文件管理器", Color::White, false),
        line(
            "  0                  显示 / 关闭本帮助",
            Color::White,
            false,
        ),
        line("", Color::White, false),
        section_header("▎播放列表管理器（视图 5）"),
        line(
            "  h / l / Tab        切换焦点（曲库 ↔ 歌单）",
            Color::White,
            false,
        ),
        line(
            "  Enter(曲库歌曲)     添加到当前展开的歌单",
            Color::White,
            false,
        ),
        line(
            "  Enter(...)         新建歌单（输入模式）",
            Color::White,
            false,
        ),
        line("  Enter(歌单名)       展开 / 收起歌单", Color::White, false),
        line(
            "  Enter(歌单内歌曲)   从歌单中删除该歌曲",
            Color::White,
            false,
        ),
        line("", Color::White, false),
        section_header("▎文件管理器（视图 6）"),
        line(
            "  h / l / Tab        切换焦点（曲库 ↔ 文件系统）",
            Color::White,
            false,
        ),
        line("  Enter(文件夹)       进入该文件夹", Color::White, false),
        line(
            "  Enter(音频文件)     添加到曲库（仅记录路径）",
            Color::White,
            false,
        ),
        line("  Backspace          返回上级目录", Color::White, false),
        line(
            "  Enter(曲库歌曲)     从曲库删除记录（不删文件）",
            Color::White,
            false,
        ),
        line("", Color::White, false),
        section_header("▎歌词控制"),
        line(
            "  [ / ]              歌词提前 / 延后 0.5 秒",
            Color::White,
            false,
        ),
        line(
            "  { / }              歌词提前 / 延后 2 秒",
            Color::White,
            false,
        ),
        line("  Ctrl+r             重置歌词偏移为 0", Color::White, false),
        line("", Color::White, false),
        section_header("▎命令行"),
        line(
            "  termusic                      启动交互模式",
            Color::White,
            false,
        ),
        line(
            "  termusic play <文件>           播放指定文件",
            Color::White,
            false,
        ),
        line(
            "  termusic play <目录>           播放目录中的所有音频",
            Color::White,
            false,
        ),
        line("", Color::White, false),
        section_header("▎配置文件"),
        line(
            "  config/config.toml        主配置（音乐目录、音量、频谱等）",
            Color::White,
            false,
        ),
        line(
            "  config/keybindings.toml   自定义快捷键",
            Color::White,
            false,
        ),
        line("", Color::White, false),
        line("按 0 或 Esc 关闭帮助", Color::DarkGray, false),
    ];

    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" 帮助 — 按 0 或 Esc 关闭 ")
            .style(Style::default().fg(Color::Yellow)),
    );
    f.render_widget(para, popup_area);
}

fn section_header(text: &str) -> Line<'_> {
    Line::from(Span::styled(
        text,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn line(text: &str, color: Color, bold: bool) -> Line<'_> {
    let mut style = Style::default().fg(color);
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    Line::from(Span::styled(text, style))
}
