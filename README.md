# termusic — 终端音乐播放器

一个运行在终端里的音乐播放器，使用 Rust 编写。

支持多格式音频解码、元数据显示、LRC 歌词同步、频谱可视化，纯键盘 Vim 风格操作。

---

## 功能特性

### 音频播放
- 支持 MP3、FLAC、OGG、Opus、WAV、AAC、M4A、WMA、APE、WavPack、AIFF 等格式
- 基于 [Symphonia](https://github.com/pdeljanov/Symphonia) 纯 Rust 解码，无需安装 ffmpeg
- 无缝播放（Gapless Playback）
- 音量控制、快进快退

### 元数据与曲库
- 自动读取 ID3v1/v2、Vorbis Comments、APE、MP4 等标签
- 缺失字段自动回退（标题用文件名、艺术家显示"Unknown Artist"）
- SQLite 曲库索引，支持全文搜索
- 三栏浏览器：艺术家 → 专辑 → 曲目逐级展开

### 歌词系统
- 标准 LRC 和增强 LRC（逐字时间戳）解析
- 自动编码检测：UTF-8 → GBK → Shift-JIS
- 同名 `.lrc` 文件放在音频旁即自动加载
- 实时同步高亮，偏移微调（±0.5s / ±2s）
- 全屏 KTV 歌词视图

### 频谱可视化
- 实时 FFT 分析（2048 点 Hann 窗，约 30 FPS）
- 对数频率分桶（20Hz–16kHz，32 柱）
- 指数移动平均平滑 + 动态范围归一化
- 绿→黄→红 颜色渐变（低频到高频）
- 全屏鱼缸模式，暂停时渐变衰减

### 用户界面
- Vim 风格模态键盘操作
- 四个视图：播放器、曲库浏览器、全屏歌词、全屏频谱
- 播放列表管理：添加、删除、排序、随机、循环
- 实时搜索过滤
- 帮助面板（按 `0` 查看全部键位）

### 配置与持久化
- 5 套内置主题：Tokyo Night、Dracula、Nord、Solarized Dark、Catppuccin Mocha
- 自定义快捷键
- 退出自动保存状态（音量、播放模式、当前曲目），下次启动可恢复
- 所有文件自包含在项目文件夹中

---

## 系统要求

- **Linux** 操作系统（Arch、Ubuntu、Debian、Fedora 等）
- 音频输出设备（PulseAudio / PipeWire / ALSA 均可）
- **Rust** 编译工具链（1.70+）

---

## 安装

### 第一步：安装 Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

### 第二步：编译

```bash
cd ~/Desktop/Terminal_music_player
cargo build --release
```

编译产物在 `target/release/termusic`，约 7MB 的单文件二进制。

### 第三步：加入 PATH

在 `~/.bashrc` 末尾添加：

```bash
export PATH="$HOME/Desktop/Terminal_music_player/target/release:$PATH"
```

然后执行 `source ~/.bashrc` 或重新打开终端。

### 第四步：运行

```bash
termusic                  # 交互模式
termusic play 歌曲.flac   # 播放指定文件
```

提示：已经创建了 `tmper` 符号链接，也可以用 `tmper` 命令启动。

---

## 使用教程

### 基本操作

启动后进入播放器视图，界面从上到下依次是：

```
▶ 歌曲名 — 艺术家                         01:23 / 04:56    ← 信息栏
████████████████░░░░░░░░░░  Vol: 80%  🔁 🔀                  ← 进度条
▁▂▃▄▅▆▇█▇▆▅▄▃▂▁                                        ← 频谱（5 行）
    歌词第一行                                              ← 歌词（6 行）
▶   歌词当前行（高亮）
    歌词下一行
  1. Bohemian Rhapsody — Queen                      05:55  ← 播放列表
▶ 2. Another One Bites the Dust                     03:36
  3. Don't Stop Me Now                              03:29
Playlist: Default | 3 tracks | 12:60 | 🔁           [Player]  ← 状态栏
```

### 播放音乐

```bash
# 播放单曲
termusic play ~/Music/song.flac

# 播放整个目录
termusic play ~/Music/Queen/
```

然后用 `j`/`k` 移动光标，`Enter` 播放选中的曲目，`Space` 暂停/恢复。

### 加载歌词

将 `.lrc` 歌词文件放在音频文件同一目录下，保持同名即可：

```
~/Music/
├── song.flac
├── song.lrc        ← 自动加载
```

支持中文歌词（GBK 编码自动识别）。

### 浏览曲库

按 `2` 进入曲库浏览器（三栏布局），按 `h`/`l` 切换焦点栏，`j`/`k` 浏览，`Enter` 播放。

### 切换主题

编辑 `config/config.toml`：

```toml
[ui]
theme = "dracula"    # 可选: tokyo-night, dracula, nord, solarized-dark, catppuccin-mocha
```

---

## 全部快捷键

| 键 | 功能 |
|----|------|
| `Space` | 播放 / 暂停 |
| `n` / `p` | 下一首 / 上一首 |
| `-` / `=` | 音量减 / 加 |
| `Enter` | 播放选中 |
| `j` / `k` | 下 / 上移动 |
| `g` `g` | 跳到顶部 |
| `G` | 跳到底部 |
| `Ctrl+d` / `Ctrl+u` | 翻半页 |
| `d` `d` | 删除当前 |
| `r` | 切换循环模式 |
| `R` | 切换随机 |
| `/` | 搜索，`Esc` 清除 |
| `1` `3` `4` | 切换视图 |
| `[` `]` `{` `}` | 歌词偏移 |
| `Ctrl+r` | 重置歌词偏移 |
| `0` | 帮助面板 |
| `q` | 退出 |

---

## 配置文件

所有配置文件都在 `config/` 目录下：

### config/config.toml

```toml
[library]
music_dirs = ["~/Music"]            # 音乐目录
extensions = ["mp3", "flac", ...]   # 扫描格式
scan_on_startup = false             # 启动时自动扫描

[playback]
default_volume = 0.8                # 默认音量（0.0–1.0）
gapless = true                      # 无缝播放
seek_step_small_secs = 5            # 快进退步长
seek_step_large_secs = 30           # 大步快进退

[visualizer]
enabled = true
num_bars = 32                       # 频谱柱数量
frame_rate = 30                     # 刷新率
smoothing = 0.35                    # 平滑系数（0–1）

[lyrics]
auto_load = true
encoding_fallbacks = ["utf-8", "gbk", "shift-jis"]

[ui]
theme = "tokyo-night"
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

A: 检查日志文件 `data/termusic.log`。如果终端窗口太小（少于 10 行），界面无法正常渲染，请调大窗口。

### Q: 播放没有声音？

A: 确认系统音频正常（PulseAudio/PipeWire/ALSA）。可以先用其他播放器测试。检查日志文件中是否有音频错误。

### Q: 歌词不显示？

A: 确保 `.lrc` 文件与音频文件同名、同目录。检查歌词文件编码是否为 UTF-8 / GBK / Shift-JIS。也可以按 `3` 切换到全屏歌词视图查看（即使无歌词也会显示"No lyrics found"）。

### Q: 频谱不跳动？

A: 确认 `config.toml` 中 `[visualizer] enabled = true`。频谱需要播放音频时才会显示。按 `4` 可以切换到全屏频谱视图。

### Q: 如何添加更多音乐？

A: 编辑 `config/config.toml` 的 `music_dirs`，添加音乐目录路径，然后按 `:` 输入 `:library scan`（命令模式功能开发中，当前重启后会自动检测新文件）。

### Q: 支持哪些音频格式？

A: MP3、FLAC、OGG Vorbis、Opus、WAV、AAC（.aac/.m4a）、ALAC（.m4a）、WavPack（.wv）、WMA、AIFF、APE。如果遇到不支持的格式，程序会跳过并记录日志。

### Q: 如何卸载？

A: 删除项目文件夹即可（所有文件都在里面）：
```bash
rm -rf ~/Desktop/Terminal_music_player
```
然后从 `~/.bashrc` 中删除对应的 PATH 行。

### Q: 支持 macOS / Windows 吗？

A: 目前仅支持 Linux。macOS 理论上可编译（rodio 支持 CoreAudio），但未测试。Windows 需要替换音频后端，暂不支持。

---

## 项目结构

```
Terminal_music_player/
├── Cargo.toml                 # Rust 项目配置
├── DESIGN.md                  # 完整设计文档
├── README.md                  # 本文件
│
├── config/                    # 配置文件目录
│   ├── default.toml           # 默认配置模板
│   ├── config.toml            # 用户配置（自动生成）
│   └── keybindings.toml       # 自定义快捷键（可选）
│
├── data/                      # 运行时数据（自动生成）
│   ├── termusic.log           # 日志文件
│   ├── state.json             # 退出时保存的状态
│   └── library.db             # 曲库索引数据库
│
├── themes/                    # 主题文件
│
├── progress/                  # 开发进度记录
│
├── src/                       # 源代码
│   ├── main.rs, app.rs        # 入口 + 事件循环
│   ├── config.rs, cli.rs      # 配置 + 命令行
│   ├── playlist.rs            # 播放列表
│   ├── audio/                 # 音频引擎
│   ├── metadata/              # 标签读取
│   ├── lyrics/                # 歌词解析 + 同步
│   ├── visualizer/            # FFT + 频谱渲染
│   ├── library/               # SQLite + 扫描 + M3U
│   ├── ui/                    # 界面渲染
│   └── input/                 # 键盘处理
│
└── tests/fixtures/            # 测试音频文件
```

---

## 开发命令

```bash
cargo build                   # 调试编译
cargo build --release         # 发布编译
cargo test                    # 运行全部测试
cargo test <测试名>            # 运行单个测试
cargo clippy -- -D warnings   # 代码检查
cargo fmt --all               # 格式化代码

# 提交前完整检查
cargo fmt --all && cargo clippy -- -D warnings && cargo test
```

---

## 技术栈

| 分类 | 库 | 说明 |
|------|----|------|
| 异步 | tokio | 事件循环、定时器 |
| TUI | ratatui + crossterm | 终端界面框架 |
| 音频 | symphonia + rodio | 解码 + 输出 |
| 标签 | lofty | 元数据读取 |
| FFT | rustfft | 频谱分析 |
| 数据库 | rusqlite | 曲库索引 |
| 配置 | toml + serde + clap | 配置文件 + 命令行 |

---

## 许可证

MIT License
