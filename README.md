# tmper — 终端音乐播放器

一个运行在终端里的全功能音乐播放器，使用 Rust 编写，ratatui TUI 框架。

支持多格式音频解码、元数据显示、内嵌封面图展示（Kitty 协议 / SIXEL / 半块字符三层渐进）、LRC 歌词同步、cava 风格频谱可视化、歌单管理、SQLite 曲库，纯键盘 Vim 风格操作。

---

## 功能特性

### 音频播放
- 支持 MP3、FLAC、OGG、Opus、WAV、AAC、M4A、WMA、APE、WavPack、AIFF 等格式
- 基于 [Symphonia](https://github.com/pdeljanov/Symphonia) 纯 Rust 解码，**无需安装 ffmpeg**
- 音量控制、快进快退（← 后退 / → 前进，真实音频 seek）
- 顺序 / 随机 / 单曲 三种循环模式

### 封面图显示
- **三层渐进渲染**：
  1. **Kitty 图形协议** — 原生像素渲染（Kitty、WezTerm、Ghostty）
  2. **SIXEL** — 通过 `chafa` 子进程（Konsole Plasma 6+，可选）
  3. **半块字符** — Lanczos3 缩放 + Floyd-Steinberg 误差扩散抖动（通用回退）
- 自动读取内嵌封面（ID3v2 APIC / Vorbis Comments / MP4）

### 元数据
- ID3v1/v2、Vorbis Comments、APE、MP4 等标签自动读取
- 缺失字段自动回退（标题用文件名、艺术家显示 "Unknown Artist"）
- SQLite 曲库索引，支持全文搜索

### 歌词系统
- 标准 LRC 和增强 LRC（逐字时间戳）解析
- 自动编码检测：UTF-8 → GBK → Shift-JIS
- 同名 `.lrc` 文件放在音频旁即自动加载
- 实时同步高亮，偏移微调（`[` `]` ±0.5s / `{` `}` ±2s）
- 全屏 KTV 歌词视图

### 频谱可视化 (cava 风格)
- 实时 FFT 分析（2048 点 Hann 窗，~30 FPS）
- 对数频率分桶（32 柱），绿→黄→红 颜色渐变
- 暂停时自动衰减

### 用户界面
- **7 个视图**：播放器(1) / 曲库(2) / 全屏歌词(3) / 全屏频谱(4) / 歌单管理(5) / 文件浏览器(6) / 设置(7)
- 播放器两栏布局：左侧封面+歌单 / 右侧歌词+频谱+控制栏
- Vim 风格模态键盘、`gg`/`dd` 双键序列、`/` 搜索
- 命令模式（`:` 进入，类似 Vim 底栏）
- 帮助面板（按 `0`）

### 配置与持久化
- 5 套内置主题：Tokyo Night、Dracula、Nord、Solarized Dark、Catppuccin Mocha
- 自定义快捷键（`config/keybindings.toml`）
- 退出自动保存状态，下次启动恢复

---

## 系统要求

- **Linux** 操作系统（Arch、Ubuntu、Debian、Fedora 等）
- 音频输出设备（PulseAudio / PipeWire / ALSA）
- **Rust** 编译工具链 1.70+
- **可选**：`chafa` 命令行工具（提供 Konsole 等终端的 SIXEL 封面图渲染）

---

## 安装

### 第一步：安装 Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

### 第二步：编译

```bash
cd tmper
cargo build --release
```

编译产物在 `target/release/tmper`（单文件二进制，约 7MB）。

### 第三步（可选）：加入 PATH

在 `~/.bashrc` 末尾添加：

```bash
export PATH="$HOME/path/to/tmper/target/release:$PATH"
```

### 第四步（可选）：安装 chafa（封面图 SIXEL 渲染）

```bash
# Arch
sudo pacman -S chafa

# Ubuntu/Debian
sudo apt install chafa

# Fedora
sudo dnf install chafa
```

没有 chafa 也能正常使用——封面图会使用半块字符渲染。

### 运行

```bash
tmper                      # 交互模式
tmper play ~/Music/歌曲.flac  # 播放指定文件
```

---

## 使用教程

### 基本布局

启动后进入播放器视图，两栏布局：

```
┌─ Now Playing ───────┬─ Lyrics ──────────────────────────┐
│        ♫            │  第一行歌词 (淡色)                 │
│   Bohemian Rhapsody │  ▶ 当前行歌词 (高亮青色)          │
│   Queen             │  下一行歌词 (灰色)                 │
│   ▶ 03:12 / 05:55  │                                    │
├─ Playlists ────────┤─ Spectrum ────────────────────────┤
│  ▶ My Songs ▼      │  ▁▂▃▄▅▆▇█▇▆▅▄▃▂▁                  │
│      Song A ◄      │                                    │
│      Song B        ├────────────────────────────────────┤
│  ▶ Rock ▼          │ ▶ 03:12/05:55 [████░░] Vol:80% 🔁  │
└────────────────────┴────────────────────────────────────┘
```

### 播放音乐

```bash
tmper play ~/Music/song.flac   # 播放单曲
tmper play ~/Music/Queen/      # 播放整个目录
```

`j`/`k` 移动光标，`Enter` 播放选中曲目，`Space` 暂停/恢复。

### 加载歌词

将 `.lrc` 歌词文件放在音频文件同一目录下，保持同名即可：

```
~/Music/
├── song.flac
├── song.lrc        ← 自动加载
```

支持中文歌词（GBK 编码自动识别）。

### 切换主题

编辑 `config/config.toml`：

```toml
[ui]
theme = "dracula"
```

可选值：`tokyo-night`、`dracula`、`nord`、`solarized-dark`、`catppuccin-mocha`

---

## 全部快捷键

| 键 | 功能 |
|----|------|
| `Space` | 播放 / 暂停 |
| `n` / `p` | 下一首 / 上一首 |
| `-` / `=` | 音量减 / 加 |
| `Enter` | 播放选中曲目 |
| `j` / `k` / `↓` / `↑` | 下 / 上移动 |
| `←` / `→` | 快退 / 快进 5 秒（真实 seek） |
| `g` `g` | 跳到列表顶部 |
| `G` | 跳到列表底部 |
| `Ctrl+d` / `Ctrl+u` | 翻半页 |
| `d` `d` | 删除当前曲目 |
| `r` | 切换循环模式（顺序 / 随机 / 单曲） |
| `/` | 搜索过滤 |
| `1`–`7` | 切换视图 |
| `0` | 帮助面板 |
| `[` `]` `{` `}` | 歌词偏移微调 |
| `Ctrl+r` | 重置歌词偏移 |
| `q` | 退出 |
| `:` | 命令模式 (Vim 风格) |

---

## 命令模式（按 `:` 进入）

| 命令 | 说明 |
|------|------|
| `:q` / `:quit` | 退出程序 |
| `:help` | 显示帮助面板 |
| `:version` | 显示版本号 |
| `:theme <名称>` | 切换主题 |
| `:seek <秒数>` | 跳转（正数前进，负数后退） |
| `:volume <0-100>` | 设置音量 |
| `:repeat <模式>` | 循环模式 (sequential / shuffle / single) |
| `:view <名称>` | 切换视图 (player / library / lyrics / visualizer / playlists / browser / settings) |
| `:import <路径>` | 导入 M3U 歌单 |
| `:export <名称>` | 导出歌单为 M3U |

---

## 配置文件

所有配置文件在项目目录下的 `config/` 中：

### config/config.toml

```toml
[library]
music_dirs = ["~/Music"]
extensions = ["mp3", "flac", "ogg", "opus", "wav", "aac", "m4a", "ape", "wv", "aiff", "wma"]
scan_on_startup = false

[playback]
default_volume = 0.8
gapless = true
seek_step_small_secs = 5


[visualizer]
enabled = true
num_bars = 32
frame_rate = 30
smoothing = 0.35

[lyrics]
auto_load = true
encoding_fallbacks = ["utf-8", "gbk", "shift-jis"]

[ui]
theme = "tokyo-night"
show_cover_art = true
```

### config/keybindings.toml

```toml
play_pause = " "
next_track = "n"
prev_track = "p"
vol_down = "-"
vol_up = "="
quit = "q"
up = "k"
down = "j"
```

---

## 常见问题

### Q: 启动后按键没反应？

检查日志文件 `data/tmper.log`。终端窗口至少需要 10 行高度。

### Q: 播放没有声音？

确认 PulseAudio / PipeWire / ALSA 正常工作。先用其他播放器测试。

### Q: 封面图显示为像素块而非高清图？

需要满足以下条件之一：
- **Kitty / WezTerm / Ghostty 终端**：自动使用原生像素渲染
- **Konsole (Plasma 6+)**：安装 `chafa` 包后自动使用 SIXEL 渲染
- 其他终端：使用半块字符渲染（▄ + fg/bg 两倍垂直分辨率）

### Q: 歌词不显示？

确保 `.lrc` 文件与音频文件同名、同目录。检查歌词编码是否为 UTF-8 / GBK / Shift-JIS。按 `3` 切换到全屏歌词视图。

### Q: 如何添加更多音乐？

编辑 `config/config.toml` 的 `music_dirs`，或使用 `:import <path.m3u>` 导入 M3U 歌单。

### Q: 支持哪些音频格式？

MP3、FLAC、OGG Vorbis、Opus、WAV、AAC（.aac/.m4a）、ALAC（.m4a）、WavPack（.wv）、WMA、AIFF、APE。

### Q: 支持 macOS / Windows 吗？

目前仅支持 Linux。macOS 理论上可编译（rodio 支持 CoreAudio），但未测试。Windows 暂不支持。

---

## 项目结构

```
tmper/
├── Cargo.toml                    # Rust 项目配置
├── Cargo.lock
├── DESIGN.md                     # 架构设计文档
├── README.md                     # 本文件
│
├── config/                       # 配置文件（自包含）
│   ├── config.toml               #   主配置
│   └── keybindings.toml          #   快捷键
│
├── data/                         # 运行时数据（自动生成）
│   ├── tmper.log                 #   日志
│   ├── state.json                #   退出状态
│   ├── playlists.json            #   歌单
│   └── library.db                #   SQLite 曲库
│
├── src/                          # 源代码 (~7,400 行 Rust)
│   ├── main.rs                   #   入口
│   ├── constants.rs              #   运行时调优常量
│   ├── config.rs                 #   配置加载
│   ├── cli.rs                    #   命令行解析
│   ├── error.rs                  #   错误类型
│   ├── event.rs                  #   事件枚举
│   ├── playlist.rs               #   播放列表数据结构
│   ├── paths.rs                  #   路径工具
│   ├── app/                      #   应用核心
│   │   ├── mod.rs                #     App + 事件循环
│   │   ├── playback.rs           #     播放控制
│   │   ├── persistence.rs        #     状态持久化
│   │   └── handlers/             #     按键分发
│   ├── audio/                    #   音频引擎
│   │   ├── decoder.rs            #     Symphonia 解码
│   │   ├── output.rs             #     Rodio 输出
│   │   └── engine.rs             #     播放/暂停/seek
│   ├── metadata/                 #   元数据
│   │   └── reader.rs             #     Lofty 标签
│   ├── lyrics/                   #   歌词
│   │   ├── types.rs              #     数据结构
│   │   ├── parser.rs             #     LRC 解析
│   │   └── engine.rs             #     同步引擎
│   ├── visualizer/               #   频谱
│   │   ├── fft.rs                #     FFT 分析
│   │   ├── processor.rs          #     后处理
│   │   └── render.rs             #     字符渲染
│   ├── library/                  #   曲库
│   │   ├── database.rs           #     SQLite
│   │   ├── scanner.rs            #     目录扫描
│   │   └── playlist_manager.rs   #     M3U 导入/导出
│   ├── ui/                       #   界面
│   │   ├── mod.rs                #     UiState + render()
│   │   ├── theme.rs              #     主题
│   │   ├── cover/                #     封面渲染
│   │   │   └── mod.rs            #       Kitty / SIXEL 协议
│   │   ├── views/                #     7 个视图
│   │   └── widgets/              #     可复用组件
│   └── input/                    #   键盘
│       ├── handler.rs            #     KeyHandler
│       ├── keymap.rs             #     键位配置
│       └── command.rs            #     命令解析
│
└── tests/fixtures/               # 测试音频
    └── test.wav                  #   440Hz 正弦波
```

---

## 开发

```bash
cargo build                        # 调试编译
cargo build --release              # 发布编译（单文件 ~7MB）
cargo test                         # 全部测试（41 个）
cargo clippy -- -D warnings        # 代码检查
cargo fmt --all                    # 格式化

# 提交前完整检查
cargo fmt --all && cargo clippy -- -D warnings && cargo test
```

---

## 技术栈

| 分类 | 库 | 说明 |
|------|----|------|
| 异步 | tokio | 事件循环、定时器、后台线程 |
| TUI | ratatui + crossterm | 终端界面框架 |
| 音频 | symphonia + rodio | 多格式解码 + 音频输出 |
| 标签 | lofty | 元数据（ID3/Vorbis/APE/MP4） |
| FFT | rustfft | 2048 点频谱分析 |
| 数据库 | rusqlite (bundled) | SQLite 曲库索引 |
| 图像 | image | 封面图解码 + Lanczos3 缩放 |
| 配置 | toml + serde + clap | 配置文件 + 命令行参数 |
| 编码 | encoding_rs | 歌词编码自动检测 |

---

## 许可证

MIT License
