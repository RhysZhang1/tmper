# termusic — Terminal Music Player

A terminal-native music player for Linux, written in Rust.  
Play your music with a beautiful TUI, synchronized lyrics, and real-time spectrum visualization — all from the comfort of your terminal.

![Terminal](https://img.shields.io/badge/platform-Linux-blue)
![Language](https://img.shields.io/badge/language-Rust-orange)
![License](https://img.shields.io/badge/license-MIT-green)

---

## Features

### Core Playback
- **Multi-format decoding**: MP3, FLAC, OGG, Opus, WAV, AAC, M4A, WMA, APE, WavPack, AIFF via [Symphonia](https://github.com/pdeljanov/Symphonia)
- **Gapless playback**: Near-seamless track transitions
- **Volume control**: Logarithmic mapping, mute toggle
- **Seek**: Small step (5s) and large step (30s), configurable

### Metadata & Library
- **Rich tag reading**: ID3v1/v2, Vorbis Comments, APE, MP4/iTunes atoms via [Lofty](https://github.com/Serial-ATA/lofty-rs)
- **Fallback display**: Missing tags auto-fill from filename
- **SQLite index**: Fast incremental scanning, full-text search
- **Three-panel browser**: Artists → Albums → Tracks drill-down

### Lyrics
- **LRC parser**: Standard and enhanced LRC with word-level timestamps
- **Encoding auto-detection**: UTF-8 → GBK → Shift-JIS fallback
- **Auto-load**: Same-name `.lrc` files loaded alongside audio
- **Sync display**: Real-time line highlighting with binary search
- **Offset tuning**: Fine-tune ±0.5s / ±2s, persist per-session
- **Fullscreen KTV mode**: Centered lyrics view for singing along

### Visualizer
- **Real-time FFT**: 2048-point Hann-windowed FFT at ~30 FPS
- **Log-scale bars**: 32 frequency buckets (20Hz–16kHz) matching human hearing
- **EMA smoothing**: Smooth animation without jitter
- **Dynamic normalization**: Quiet passages still visible, loud sections not clipped
- **Color gradient**: Green (bass) → Yellow (mid) → Red (treble)
- **Fullscreen mode**: Immersive "aquarium" view
- **Pause decay**: Bars gracefully fade when paused

### User Interface
- **Vim-style modal keys**: `j`/`k`, `gg`, `G`, `dd`, `Ctrl+d`/`u`, `/` search
- **Four views**: Player, Library browser, Fullscreen lyrics, Fullscreen visualizer
- **Playlist management**: Add, remove, reorder, shuffle, repeat modes
- **Search**: Real-time filtering with `/`, `Esc` to clear
- **Help overlay**: Press `0` for keybinding reference
- **Progress bar**: Visual position indicator with volume display
- **Status bar**: Track count, total duration, playback mode

### Configuration & Persistence
- **5 built-in themes**: Tokyo Night, Dracula, Nord, Solarized Dark, Catppuccin Mocha
- **Custom keybindings**: Override any key in `config/keybindings.toml`
- **State save/restore**: Volume, playback mode, last position saved on exit
- **All files self-contained**: Everything lives in the project folder — no scattered dotfiles

---

## Requirements

- **Linux** with audio output (PulseAudio, PipeWire, or ALSA)
- **Rust** toolchain 1.70+ (install via [rustup](https://rustup.rs))

---

## Installation

```bash
# 1. Clone or navigate to the project
cd ~/Desktop/Terminal_music_player

# 2. Build (optimized release)
cargo build --release

# 3. Add to PATH (add this line to ~/.bashrc or ~/.zshrc)
export PATH="$HOME/Desktop/Terminal_music_player/target/release:$PATH"

# 4. Reload shell config
source ~/.bashrc

# 5. Run!
termusic
# Or use the alias:
tmper
```

The binary is at `target/release/termusic`. A symlink `tmper` is also provided for convenience.

---

## Quick Start

```bash
# Play a single file
termusic play ~/Music/song.flac

# Interactive mode (opens TUI)
termusic

# Play a directory (adds all supported files to playlist)
termusic play ~/Music/album/
```

---

## Key Bindings

### Normal Mode（默认模式）

| 键 | 功能 |
|----|------|
| `Space` | 播放 / 暂停 |
| `q` | 退出 |
| `n` / `p` | 下一首 / 上一首 |
| `-` / `=` | 音量减 / 加（5%） |
| `←` / `→` | 快退 / 快进 5 秒 |
| `Enter` | 播放选中的曲目 |

### 导航（Vim 风格）

| 键 | 功能 |
|----|------|
| `j` / `↓` | 向下移动 |
| `k` / `↑` | 向上移动 |
| `g` `g` | 跳到列表顶部（双击 g） |
| `G` | 跳到列表底部 |
| `Ctrl+d` | 向下翻半页 |
| `Ctrl+u` | 向上翻半页 |
| `d` `d` | 删除当前曲目（双击 d） |

### 播放模式

| 键 | 功能 |
|----|------|
| `r` | 切换循环模式（关 → 单曲 → 列表 → 关） |
| `R` | 切换随机播放 |

### 视图切换

| 键 | 视图 |
|----|------|
| `1` | 播放器（频谱 + 歌词 + 播放列表） |
| `3` | 全屏歌词（KTV 风格） |
| `4` | 全屏频谱（鱼缸模式） |
| `0` | 帮助面板（键位速查） |

### 歌词控制

| 键 | 功能 |
|----|------|
| `[` / `]` | 歌词提前/延后 0.5 秒 |
| `{` / `}` | 歌词提前/延后 2 秒 |
| `Ctrl+r` | 重置歌词偏移为 0 |

### 搜索

| 键 | 功能 |
|----|------|
| `/` | 进入搜索模式（实时过滤） |
| `Esc` | 清除搜索 / 退出搜索模式 |

---

## Configuration

所有配置文件都在项目文件夹的 `config/` 目录下：

### config/config.toml（主配置）

```toml
[library]
music_dirs = ["~/Music"]              # 音乐目录
extensions = ["mp3", "flac", ...]     # 支持的格式

[playback]
default_volume = 0.8                  # 默认音量
gapless = true                        # 无缝播放
seek_step_small_secs = 5              # 快进步长

[visualizer]
enabled = true
num_bars = 32                         # 频谱柱数
smoothing = 0.35                      # 平滑系数

[lyrics]
auto_load = true
encoding_fallbacks = ["utf-8", "gbk", "shift-jis"]

[ui]
theme = "tokyo-night"                 # 主题名
```

### config/keybindings.toml（自定义键位）

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

不配置则使用默认键位。

---

## Project Structure

```
Terminal_music_player/
├── Cargo.toml              # Rust 项目清单
├── README.md
├── DESIGN.md               # 完整设计文档
│
├── config/                 # 配置文件
│   ├── default.toml        # 默认配置模板
│   ├── config.toml         # 用户配置（自动生成）
│   └── keybindings.toml    # 自定义键位
│
├── data/                   # 运行时数据（自动生成）
│   ├── termusic.log        # 运行日志
│   ├── state.json          # 退出保存的状态
│   └── library.db          # SQLite 曲库索引
│
├── themes/                 # 主题文件
│   ├── tokyo-night.toml
│   ├── dracula.toml
│   ├── nord.toml
│   ├── solarized-dark.toml
│   └── catppuccin-mocha.toml
│
├── progress/               # 开发进度记录
│
├── src/                    # 源代码
│   ├── main.rs             # 入口
│   ├── app.rs              # 应用状态 + 事件循环
│   ├── event.rs            # 事件类型
│   ├── config.rs           # 配置加载
│   ├── cli.rs              # 命令行参数
│   ├── error.rs            # 错误类型
│   ├── paths.rs            # 路径工具
│   ├── playlist.rs         # 播放列表数据结构
│   ├── audio/              # 音频引擎
│   │   ├── decoder.rs      # Symphonia 解码
│   │   ├── output.rs       # Rodio 输出
│   │   └── engine.rs       # 播放引擎
│   ├── metadata/
│   │   └── reader.rs       # Lofty 标签读取
│   ├── lyrics/
│   │   ├── types.rs        # 歌词数据结构
│   │   ├── parser.rs       # LRC 解析器
│   │   └── engine.rs       # 歌词查找与同步
│   ├── visualizer/
│   │   ├── fft.rs          # FFT 分析
│   │   ├── processor.rs    # 频谱后处理
│   │   └── render.rs       # 字符渲染
│   ├── library/
│   │   ├── database.rs     # SQLite 数据库
│   │   ├── scanner.rs      # 目录扫描
│   │   └── playlist_manager.rs  # M3U 导入导出
│   ├── ui/
│   │   ├── mod.rs          # 渲染入口
│   │   ├── theme.rs        # 主题系统
│   │   ├── widgets/        # UI 组件
│   │   │   ├── lyrics_panel.rs
│   │   │   ├── visualizer_panel.rs
│   │   │   └── help_popup.rs
│   │   └── views/          # 全屏视图
│   │       ├── lyrics_view.rs
│   │       └── library_view.rs
│   └── input/
│       ├── handler.rs      # 键盘处理
│       ├── keymap.rs       # 键位配置
│       └── command.rs      # 命令解析
│
└── tests/
    └── fixtures/           # 测试用音频文件
        ├── test.wav
        ├── test.flac
        └── test_notags.wav
```

---

## Tech Stack

| 分类 | 库 | 用途 |
|------|-----|------|
| 异步 | tokio | 事件循环、定时器、通道 |
| TUI | ratatui + crossterm | 终端 UI 框架 |
| 音频 | symphonia + rodio | 多格式解码 + 音频输出 |
| 标签 | lofty | 元数据读取 |
| FFT | rustfft | 快速傅里叶变换 |
| 数据库 | rusqlite (bundled) | 曲库索引 |
| 配置 | toml + serde | 配置文件 |
| 命令行 | clap | 参数解析 |

---

## Commands

```bash
# Build
cargo build
cargo build --release

# Run
termusic                         # 交互模式
termusic play <file>             # 播放文件
termusic play <dir>              # 播放目录

# Test
cargo test                       # 全部测试
cargo test <test_name>           # 单个测试

# Lint & Format
cargo clippy -- -D warnings
cargo fmt --all

# Full check before commit
cargo fmt --all && cargo clippy -- -D warnings && cargo test
```

---

## License

MIT
