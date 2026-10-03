# tmper

**终端里的音乐播放器 —— 关掉界面，音乐继续。**

[![CI](https://github.com/RhysZhang1/tmper/actions/workflows/ci.yml/badge.svg)](https://github.com/RhysZhang1/tmper/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/RhysZhang1/tmper?label=release)](https://github.com/RhysZhang1/tmper/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.90%2B-orange.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-Linux-lightgrey.svg)](#系统要求)

中文 | [English](README.en.md)

---

## 这是什么

tmper 是一个跑在终端里的本地音乐播放器：Vim 风格键盘操作、LRC 歌词滚动、实时频谱、
封面图原生渲染、SQLite 曲库全文搜索，全部在一个二进制里，**不需要 ffmpeg，也不依赖任何外部程序**。

但它和大多数终端播放器有一处根本不同：**它是两个进程。** 一个常驻的播放器守护进程
（daemon）持有声音、队列和曲库；TUI 只是连上去的一个视图。所以

- 关掉终端窗口，音乐不会停；重开 TUI，接回同一首歌、同一秒；
- 在会话总线上它是一个标准的 MPRIS2 播放器，**Plasma 媒体控件、键盘媒体键、`playerctl`
  操作的就是 TUI 里显示的那份状态**；
- 曲库扫描、歌单编辑、播放控制都归 daemon，多个界面可以同时连上去看同一份真相。

界面关掉之后 daemon 不会永远赖着：没人连着、也没出声满 5 分钟，它把队列和播放位置写进
`state.json` 再退出，下次 `tmper play` 从那一秒接着放。

```
┌ Now Playing ─────────────────────┐┌ Lyrics ────────────────────────────────────────────────────────────────┐
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││Sends shivers down my spine                                             │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││Body's aching all the time                                              │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││Goodbye, everybody, I've got to go                                      │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││Gotta leave you all behind and face the truth                           │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││Mama, ooh (any way the wind blows)                                      │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││I don't wanna die                                                       │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││I sometimes wish I'd never been born at all                             │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││I see a little silhouetto of a man                                      │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││Scaramouche, Scaramouche, will you do the Fandango?                     │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││Thunderbolt and lightning, very, very frightening me                    │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││(Galileo) Galileo, (Galileo) Galileo, Galileo Figaro                    │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││                                                                        │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││                                                                        │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││                                                                        │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  │└────────────────────────────────────────────────────────────────────────┘
└──────────────────────────────────┘┌ Spectrum ──────────────────────────────────────────────────────────────┐
┌ Playlists ───────────────────────┐│            ▂▂                                                          │
│▼ Late Night                      ││            ██                                                          │
│◄ Bohemian Rhapsody               ││            ██      ▇▇        ▃▃                                        │
│   Love of My Life                ││          ▆▆██▂▂    ██        ██        ▃▃                              │
│   Don't Stop Me Now              ││          ██████    ██      ▅▅██        ██                              │
│   Somebody to Love               ││          ██████  ████▁▁    ████      ▆▆██                              │
│   Under Pressure                 ││        ▅▅██████▁▁██████    ████▄▄    ████        ▇▇                    │
│▶ Focus                           ││        ████████████████  ▆▆██████  ▁▁████▇▇      ██▃▃      ▂▂          │
│                                  ││        ████████████████▃▃████████▁▁████████    ▄▄████      ██▂▂        │
│                                  ││      ██████████████████████████████████████▄▄  ██████▃▃  ▆▆████▂▂      │
│                                  ││    ▄▄██████████████████████████████████████████████████▅▅████████▄▄    │
│                                  │└────────────────────────────────────────────────────────────────────────┘
│                                  │ 🎵  Late Night  💿  A Night at the Opera  ♪ Rock  📅  1975  FLAC
│                                  │▶ 03:12 / 05:55 Vol:80% [███████████████░░░░░░░░░░░░░] ⟳ 顺 序 循 环
│                                  │
│                                  │
└──────────────────────────────────┘
```

> 上图是渲染器真实输出（`cargo test -- --ignored --nocapture print_the_player_view`），
> 不是画的示意图——示意图是第二份实现，它会悄悄和真的不一致。
>
> 这里没有颜色：GitHub 的 Markdown 不认 ```` ```ansi ```` 代码块，彩色版本贴上来只会变成
> 乱码。所以封面在这张图里是一片半块字符，频谱也只有明暗——在终端里两者都是主题配色的。
> 在本机跑上面那条命令就能看到彩色版。

---

## 目录

- [功能特性](#功能特性)
- [系统要求](#系统要求)
- [安装](#安装)
- [快速开始](#快速开始)
- [使用教程](#使用教程)
- [全部快捷键](#全部快捷键)
- [命令模式](#命令模式按--进入)
- [配置文件](#配置文件)
- [常见问题](#常见问题)
- [工作原理](#工作原理)
- [项目结构](#项目结构)
- [开发](#开发)
- [技术栈](#技术栈)
- [许可证](#许可证)

---

## 功能特性

### 播放与进程模型
- 支持 MP3、FLAC、OGG、Opus、WAV、AAC、M4A、WMA、APE、WavPack、AIFF 等格式
- 基于 [Symphonia](https://github.com/pdeljanov/Symphonia) 纯 Rust 解码，**无需安装 ffmpeg**
- 有界流式解码，只预缓冲约 2 秒；快速切歌和 seek 会取消旧会话，长文件不会整个读进内存
- 真实音频 seek、音量控制、顺序 / 随机 / 单曲三种循环
- **关掉界面音乐继续放**：重开 TUI 接回同一首、同一个进度
- **播放器意外死掉时界面不跟着退**：顶部出一条横幅，后台自动重连；这期间按的键排队，
  恢复后按顺序送达

### 桌面集成（MPRIS2）
- 在会话总线上占名 `org.mpris.MediaPlayer2.tmper`，**Plasma 媒体控件、媒体键、`playerctl` 直接可用**
- 播放状态、曲目元数据（含微秒级时长）、循环 / 随机、音量都是双向的：桌面上改，TUI 里跟着变
- 封面以 `file://` URL 指向 `$XDG_CACHE_HOME/tmper/` 里的缓存文件（按 mtime 保留最新 8 张）
- 没有会话总线时（SSH、纯控制台）记一条日志照常播放，不因此拒绝启动

### 封面图显示
- **三层渐进渲染**，由启动探测决定走哪一层：
  1. **Kitty 图形协议** — 原生像素渲染（Kitty、WezTerm、Ghostty、Konsole 26.08+）
  2. **SIXEL** — 程序内编码（Konsole Plasma 6+、xterm、foot…），无外部依赖
  3. **半块字符** — Lanczos3 缩放 + Floyd–Steinberg 误差扩散抖动（通用回退）
- 图形载荷只在封面变化时发送一次；栅格尺寸就是封面矩形的像素尺寸，与半块字符图层共用同一个盒子
- 启动时探测一次终端能力：Kitty 协议自己的能力查询（`ESC _ G … a=q`）+ `CSI c`（DA1）的
  SIXEL 属性位，**只有终端明确回答支持**才会发送对应载荷
- 封面区域按图片宽高比自适应，并读取终端上报的单元格像素尺寸对齐原生图像
- 自动读取内嵌封面（ID3v2 APIC / Vorbis Comments / MP4）

### 曲库与元数据
- ID3v1/v2、Vorbis Comments、APE、MP4 等标签自动读取，缺失字段自动回退
- 后台增量扫描音乐目录，按指纹跳过没变过的文件；扫描可取消
- SQLite + FTS5 全文搜索，覆盖标题、艺术家、专辑和流派
- M3U 歌单导入 / 导出；歌单的身份是 id，不是名字或位置

### 歌词
- 标准 LRC 和增强 LRC（逐字时间戳）解析
- 自动编码检测：UTF-8 → GBK → Shift-JIS
- 同名 `.lrc` 放在音频旁即自动加载
- 实时同步高亮，偏移微调（`[` `]` ±0.5s / `{` `}` ±2s）
- 全屏 KTV 歌词视图

### 频谱可视化
- 实时 FFT 分析（2048 点 Hann 窗，约 30 FPS）
- 对数频率分桶，绿 → 黄 → 红颜色渐变
- 暂停时自动衰减；**只有客户端订阅时才算**，没人看就不占 CPU 和带宽

### 界面
- **7 个视图**：播放器 `1` / 曲库 `2` / 全屏歌词 `3` / 全屏频谱 `4` / 歌单管理 `5` /
  文件浏览器 `6` / 设置 `7`
- 播放器两栏布局：左侧封面 + 歌单，右侧歌词 + 频谱 + 信息栏 + 控制栏
- Vim 风格模态键盘、`gg` / `dd` 双键序列、`/` 实时搜索
- 命令模式（`:` 进入）、帮助面板（`8`）
- 5 套内置主题：Tokyo Night、Dracula、Nord、Solarized Dark、Catppuccin Mocha
- 自定义快捷键（`$XDG_CONFIG_HOME/tmper/keybindings.toml`）
- 退出自动保存状态；下次启动恢复音量、循环模式、歌词偏移，以及**队列和上次的播放位置**
  ——曲目已在架上、进度条停在上次那一秒，按播放键接着放（启动本身不出声）

---

## 系统要求

- **Linux**（Arch、Ubuntu、Debian、Fedora 等；macOS / Windows 未验证）
- 音频输出：PulseAudio / PipeWire / ALSA 之一
- **Rust 1.90 或更新**（封面图的 SIXEL 编码在程序内完成，不需要 `chafa` 等外部工具；
  这个下限来自依赖链而不是本项目的代码——`icy_sixel` 的量化依赖 `quantette` 声明了 1.90，
  cargo 会直接拒绝更旧的工具链）
- 编译期需要 ALSA 开发头文件（`alsa-lib-devel` / `libasound2-dev`），因为 rodio 经由 cpal
  链接 `libasound`

---

## 安装

### 第一步：拿到 Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

### 第二步：编译安装

```bash
git clone https://github.com/RhysZhang1/tmper.git
cd tmper
cargo build --release
```

产物是 `target/release/tmper`。运行所需的默认配置和五套主题都编译进了可执行文件，
**复制这一个文件到别的机器就能跑**；用户配置、曲库和状态仍然写在 XDG 目录里。

也可以直接装进 Cargo 的可执行目录：

```bash
cargo install --path .
```

### 第三步：运行

```bash
tmper                          # 打开界面
tmper play ~/Music/歌曲.flac    # 打开界面并播放这个文件
tmper status                   # 看正在运行的播放器在做什么
```

### 从旧版本升级

首次启动会自动把项目目录里的旧 `config/`、`data/` 布局迁移到 XDG 目录（复制，不删原件）。
无需手动操作。

---

## 快速开始

```bash
tmper play ~/Music/song.flac   # 播放单曲（CLI 接受单个文件；目录请在界面里添加）
```

`j` / `k` 移动光标，`Enter` 播放选中项，`Space` 暂停 / 恢复，`q` 关掉界面。

**想放一整个音乐库**：按 `6` 打开文件浏览器，进到音乐目录，按 `a` 添加并后台扫描。
扫描完成后按 `2` 进曲库视图，`/` 搜索，`Enter` 播放。

---

## 使用教程

### 后台播放：关掉窗口，音乐不停

播放器与界面是两个进程。界面关掉后 daemon 继续放，重新打开 TUI 会接回同一首歌、同一个进度。

不带文件时，下面这些动词是**发给正在运行的播放器**的消息，不会自己拉起一个——
`tmper pause` 在没东西在放的时候会说实话，而不是悄悄起一个播放器好把它暂停：

| 命令 | 作用 |
|------|------|
| `tmper play` | 接着放（等价于媒体键的播放；`stop` 之后则从头开始） |
| `tmper pause` | 暂停，曲目留在架上 |
| `tmper next` / `tmper prev` | 下一首 / 上一首 |
| `tmper stop` | 停声、释放音频设备并回到开头（曲目仍在架上，`tmper play` 从头再放） |
| `tmper volume <0-100>` | 设置音量 |
| `tmper status` | 打印当前状态、曲目、进度、音量与队列长度 |
| `tmper quit` | 彻底停止播放器并退出 daemon |

`q` 和 `:quit` **只关界面**，音乐继续；`:quit!`（或 `tmper quit`）才是把播放器也停掉。

daemon 在**没有客户端连着、也没有出声**（播放或暂停）满 5 分钟后自己退出；退出前把队列和
播放位置写进 `state.json`，所以下次 `tmper play` 接着放，不丢东西。

想在终端里直接盯着播放器看（排查问题时），用 `tmper daemon` 在前台跑，日志在
`~/.local/state/tmper/tmper-daemon.log`。

### 加载歌词

把 `.lrc` 歌词文件和音频放在同一目录、保持同名即可：

```
~/Music/
├── song.flac
└── song.lrc        ← 自动加载
```

支持中文歌词（GBK 编码自动识别）。

### 添加音乐到曲库

按 `6` 打开文件浏览器，进入音乐目录后按 `a` 添加并后台扫描；以后再次按 `a` 只会读取新增
或发生变化的文件。扫描中按 `c` 可取消。也可以用 `:import <path.m3u>` 导入 M3U 歌单。

### 换主题

编辑 `~/.config/tmper/config.toml`：

```toml
[ui]
theme = "dracula"
```

可选值：`tokyo-night`、`dracula`、`nord`、`solarized-dark`、`catppuccin-mocha`。
也可以直接在界面里按 `7` 进设置视图改，或者用 `:theme dracula`。

---

## 全部快捷键

| 键 | 功能 |
|----|------|
| `Space` | 播放 / 暂停 |
| `n` / `p` | 下一首 / 上一首 |
| `Enter` | 播放选中曲目 |
| `-` / `=` | 音量减 / 加 |
| `j` / `k` / `↓` / `↑` | 下 / 上移动 |
| `←` / `→` | 快退 / 快进 5 秒（真实 seek） |
| `g` `g` / `G` | 跳到列表顶部 / 底部 |
| `Ctrl+d` / `Ctrl+u` | 翻半页 |
| `d` `d` | 删除当前曲目 |
| `r` | 切换循环模式（顺序 / 随机 / 单曲） |
| `/` | 播放器队列搜索（实时过滤，j/k 选结果，Enter 播放） |
| `1`–`7` | 切换视图 |
| `8` | 帮助面板 |
| `a` / `c` | 文件浏览器中：添加 / 重新扫描目录、取消后台扫描 |
| `[` `]` `{` `}` | 歌词偏移微调（±0.5s / ±2s） |
| `Ctrl+r` | 重置歌词偏移 |
| `:` | 命令模式（Vim 风格） |
| `q` | 退出界面（音乐继续） |

---

## 命令模式（按 `:` 进入）

| 命令 | 说明 |
|------|------|
| `:q` / `:quit` | 退出界面（音乐继续） |
| `:q!` / `:quit!` | 停止播放并退出 daemon |
| `:help` | 显示帮助面板 |
| `:version` | 显示版本号 |
| `:theme <名称>` | 切换主题 |
| `:seek <秒数>` | 跳转（正数前进，负数后退） |
| `:volume <0-100>` | 设置音量 |
| `:repeat <模式>` | 循环模式（sequential / shuffle / single） |
| `:view <名称>` | 切换视图（player / library / lyrics / visualizer / playlists / browser / settings） |
| `:import <路径>` | 导入 M3U 歌单 |
| `:export <名称>` | 导出歌单为 M3U |

---

## 配置文件

配置遵循 XDG 目录规范：

- 配置：`$XDG_CONFIG_HOME/tmper/`（通常为 `~/.config/tmper/`）
- 曲库与导出的 M3U：`$XDG_DATA_HOME/tmper/`
- 状态、歌单与日志：`$XDG_STATE_HOME/tmper/`（`tmper.log` 是界面的，`tmper-daemon.log` 是播放器的）
- 封面缓存：`$XDG_CACHE_HOME/tmper/`（供桌面控件读取，保留最新 8 张）
- 通信 socket：`$XDG_RUNTIME_DIR/tmper/socket`（权限 0600，退出时删除）
- 可用 `TMPER_CONFIG_DIR`、`TMPER_DATA_DIR`、`TMPER_STATE_DIR`、`TMPER_RUNTIME_DIR` 覆盖，
  便于测试和便携使用

### ~/.config/tmper/config.toml

首次运行时由二进制内嵌模板生成。

```toml
[playback]
default_volume = 0.8
seek_step_small_secs = 5

[visualizer]
num_bars = 32
frame_rate = 30
smoothing = 0.35

[ui]
theme = "tokyo-night"
show_cover_art = true
# 终端单元格的像素尺寸。留空表示自动探测：先用 CSI 16 t（Konsole 走这条），
# 再用 CSI 14 t ÷ CSI 18 t。两者都不上报时封面按 10×20 的假设值排版，
# 此时可手动指定，例如 cell_px = [10, 20]
# cell_px = [10, 20]
```

模板之外的多余键会被忽略——加注释、留旧字段都不会让程序拒绝启动。

### ~/.config/tmper/keybindings.toml

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

检查日志 `~/.local/state/tmper/tmper.log`。终端小于 30×8 时会显示尺寸提示页。

另外确认界面**连上了播放器**：连不上时顶部会有横幅，日志里会写它每次重试的结果。

### Q: 播放没有声音？

确认 PulseAudio / PipeWire / ALSA 正常工作，先用别的播放器测一下。播放器的日志在
`~/.local/state/tmper/tmper-daemon.log`，音频设备打开失败会写在那里。

### Q: 桌面控件 / 媒体键看不到 tmper？

先看播放器在不在总线上：

```bash
playerctl -p tmper status                    # 能打印 Playing/Paused/Stopped 就说明这一层是通的
busctl --user list | grep tmper              # 看它有没有占到 org.mpris.MediaPlayer2.tmper
```

没有输出说明 daemon 没在跑（先 `tmper play`），或者环境里没有会话总线
（`echo $DBUS_SESSION_BUS_ADDRESS` 为空——SSH、纯控制台就是这种，此时播放照常，只是桌面
看不到）。总线这一层通了但 Plasma 仍然不显示，原因在桌面侧（比如媒体控件被设置成不显示、
或媒体键被别的程序抢占），daemon 的日志会写清它启动时接没接上总线。

### Q: 封面图显示为像素块而非高清图？

需要满足以下条件之一：

- **Kitty / WezTerm / Ghostty / Konsole 26.08+**：自动使用原生像素渲染（Kitty 协议）
- **Konsole (Plasma 6+)、xterm、foot 等**：自动使用 SIXEL 渲染（无需额外安装任何东西）
- 其他终端：使用半块字符渲染（`▄` + 前后景两倍垂直分辨率）

封面区域会按图片宽高比自动调整，并读取终端上报的单元格像素尺寸来对齐原生图像。单元格
尺寸在启动时探测一次，结果写在日志里：`grep "cell size" ~/.local/state/tmper/tmper.log`。
两个来源都不上报的终端（`tmux` 的部分配置、少数模拟器）会退回 10×20 的假设值，封面比例
仍然正确但可能小一圈，此时在 `config.toml` 里指定 `cell_px = [宽, 高]` 即可。

同一次探测还会问终端支不支持图形协议：Kitty 用协议自己的能力查询，SIXEL 用 `CSI c`（DA1，
属性位含 `4` 即支持）。**只有终端明确回答支持**才会发送对应载荷，否则一律使用半块字符封面
——这是 VTE 系终端（GNOME Terminal、xfce4-terminal）和 Alacritty 上的正常路径。

`tmux` / `screen` / `zellij` 下图形协议默认被复用器吞掉，而查询回答可能仍来自底下的真终端，
所以识别到复用器后**连问都不问**，直接使用字符画（`grep "graphics" …/tmper.log` 可以看到
这次判断的结果）。

顺带一提，这也是 Konsole 26.08 上能出原生像素封面的原因：它实现了 Kitty 图形协议，但一个
相关环境变量都不设，靠环境变量判断是认不出来的。

### Q: 歌词不显示？

确保 `.lrc` 文件与音频文件同名、同目录，编码是 UTF-8 / GBK / Shift-JIS 之一。
按 `3` 切换到全屏歌词视图。

### Q: `q` 之后音乐还在响，是 bug 吗？

不是，这是设计。`q` 只关界面。要连播放器一起停掉，用 `:quit!` 或 `tmper quit`。

### Q: 支持哪些音频格式？

MP3、FLAC、OGG Vorbis、Opus、WAV、AAC（.aac/.m4a）、ALAC（.m4a）、WavPack（.wv）、
WMA、AIFF、APE。

### Q: 支持 macOS / Windows 吗？

目前仅支持 Linux。macOS 理论上可编译（rodio 支持 CoreAudio），但未测试；Windows 暂不支持。

---

## 工作原理

它是一个二进制，但**扮演两个角色**：

```
$ tmper                    $ tmper daemon
   TUI 客户端                  播放器守护进程
   ├ 事件循环 / 渲染            ├ 音频引擎（rodio + symphonia）
   ├ 按键分发 / 歌词同步         ├ 播放队列与续播策略
   └ 连接状态 / 重连            ├ SQLite 曲库 / 扫描器 / 歌单
        │                      ├ state.json（唯一写入者）
        │                      ├ FFT 线程
        │                      └ MPRIS2（会话总线）
        └────── unix socket ───┘
          $XDG_RUNTIME_DIR/tmper/socket
          行分隔 JSON，双向
```

- **状态整体推送，不做增量**：daemon 每 tick 推一个完整快照，所以重连不需要补课协议——
  下一个快照就是补课。
- **协议是不对称的**：客户端发 `Request`，daemon 说的一切都是 `Event`，**包括对请求的答复**。
  TUI 的循环在 `select!` 里，绝不阻塞等回包。
- **daemon 绝不因为客户端而等待**：每个连接的邮箱有界，满了丢会过期的快照和频谱帧，
  该断的断——播放永远不被慢客户端拖住。
- **一个 trait，两种投递时机**：测试用的本地句柄同步应答，socket 句柄下一 tick 到货，
  但两者交给同一个 `apply_event`。**时机不同，应用逻辑完全相同**，所以不存在「测试走一条路、
  生产走另一条路」的假绿。
- **一个文件一个写入者**：`state.json`、`library.db`、`playlists.json`、`library.json`
  都只有 daemon 写。

完整架构、不变量和踩过的坑见 [DESIGN.md](DESIGN.md)；当前能力与限制见 [STATUS.md](STATUS.md)。

---

## 项目结构

```
tmper/
├── Cargo.toml                    # Rust 项目配置
├── DESIGN.md                     # 架构与实现约束
├── STATUS.md                     # 当前能力、限制与近期计划
├── README.md / README.en.md      # 本文件 / English
│
├── config/default.toml           # 编译进程序的默认配置
├── themes/                       # 编译进程序的五套主题
├── progress/                     # 历史开发记录，不代表当前实现
│
├── src/                          # 源代码（约 24,750 行 Rust，单 binary）
│   ├── main.rs                   #   入口：按动词分流（daemon / 一次性命令 / TUI）
│   ├── cli.rs                    #   命令行解析
│   ├── client.rs                 #   一次性动词：连上、发一条、打印、退出
│   ├── daemon.rs                 #   播放器进程：socket 监听、空闲退出、MPRIS 装配
│   ├── ipc/                      #   进程间协议
│   │   ├── mod.rs                #     行分隔 JSON 收发
│   │   └── proto.rs              #     Request / Event —— 两侧唯一的契约
│   ├── player/                   #   daemon 内核
│   │   ├── mod.rs                #     引擎 + 队列 + 续播策略
│   │   ├── library.rs            #     曲库索引与扫描任务
│   │   ├── playlists.rs          #     歌单（身份是 id）
│   │   ├── persistence.rs        #     state.json（唯一写入者）
│   │   ├── cover.rs              #     封面缓存（供 artUrl）
│   │   ├── fft.rs                #     频谱线程与订阅
│   │   └── mpris.rs              #     MPRIS2 接口
│   ├── app/                      #   TUI 客户端
│   │   ├── mod.rs                #     App + 事件循环
│   │   ├── handle.rs             #     PlayerHandle：socket / 本地两种实现
│   │   ├── playback.rs           #     选曲与歌词装载
│   │   └── handlers/             #     按键分发
│   ├── audio/                    #   音频引擎（daemon 侧）
│   │   ├── decoder.rs            #     Symphonia 解码
│   │   ├── output.rs             #     Rodio 输出
│   │   └── engine.rs             #     播放 / 暂停 / seek
│   ├── metadata/                 #   元数据（lofty 标签）
│   ├── lyrics/                   #   歌词（LRC 解析 + 同步）
│   ├── visualizer/               #   频谱（FFT + 分桶 + 字符渲染）
│   ├── library/                  #   曲库底层（SQLite / 扫描 / M3U）
│   ├── ui/                       #   界面
│   │   ├── mod.rs                #     UiState + render()（含连接横幅）
│   │   ├── theme.rs              #     13 色槽语义主题
│   │   ├── cover/                #     封面渲染（Kitty / SIXEL / 半块）
│   │   ├── views/                #     7 个视图
│   │   └── widgets/              #     可复用组件
│   └── input/                    #   键盘（键位 / 双键序列 / `:` 命令）
│
└── tests/fixtures/               # 测试音频
    ├── test.wav                  #   440Hz 正弦波
    ├── test.flac                 #   带完整标签的 FLAC
    └── test_notags.wav           #   无标签 WAV（验证回退）
```

---

## 开发

```bash
cargo build                        # 调试编译
cargo build --release              # 发布编译（资源已内嵌的单文件）
cargo test                         # 默认：全部无需音频设备的测试（521 个）
cargo test audio_output_ -- --ignored --test-threads=1  # 需要真实/虚拟设备（6 个）
cargo clippy --all-targets -- -D warnings
cargo fmt --all

# 提交前完整检查
cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test

# 行覆盖率（一次性安装，之后可复用）
rustup component add llvm-tools-preview   # 一次性：llvm-cov 依赖
cargo install cargo-llvm-cov --locked     # 一次性
cargo llvm-cov --all-features --workspace # 输出各模块行覆盖率与总计
```

**默认测试集合是免设备的**：daemon 侧的测试在 headless 引擎上建 `Player`，TUI 测试驱动一个
本地句柄，所以没有声卡也能 `cargo test` 全绿。只有 6 个测试真的碰音频输出，它们叫
`audio_output_*` 并标了 `#[ignore]`。

覆盖率现状 **89.96% 行 / 90.61% 区域 / 88.63% 函数**（2026-10-03 实测，共 528 个 =
521 默认运行 + 7 忽略，其中 6 个是设备门控、1 个是生成上面那张截图的工具）。剩下的未覆盖部分是结构性的，不是遗漏——细节见
[DESIGN.md §10](DESIGN.md)。

### 重新生成 README 里的截图

README 顶部的截图是渲染器的真实输出，不是画的：

```bash
cargo test -- --ignored --nocapture print_the_player_view
```

它会打印一段纯文本代码块，替换 README 里对应的那段即可。**不要**给它加颜色：GitHub
不支持 ```` ```ansi ```` 代码块，它会把每个 ESC 换成 U+FFFD，把 `[0;38;2;…m` 原样留在图里，
整张截图变成两千多个替换字符——这个坑踩过一次。

---

## 技术栈

| 分类 | 库 | 说明 |
|------|----|------|
| 异步 | tokio | 事件循环、定时器、后台任务 |
| TUI | ratatui + crossterm | 终端界面框架 |
| 音频 | symphonia + rodio | 多格式解码 + 音频输出 |
| 标签 | lofty | 元数据（ID3 / Vorbis / APE / MP4） |
| FFT | rustfft | 2048 点频谱分析 |
| 数据库 | rusqlite (bundled) | SQLite + FTS5 曲库索引 |
| 图像 | image + icy_sixel | 封面解码、Lanczos3 缩放、进程内 SIXEL 编码 |
| 桌面集成 | mpris-server (zbus) | MPRIS2 服务端：Plasma 媒体控件、媒体键、playerctl |
| 进程间 | serde_json + unix socket | 行分隔 JSON，不引入额外协议库 |
| 配置 | toml + serde + clap | 配置文件 + 命令行参数 |
| 编码 | encoding_rs | 歌词编码自动检测 |

---

## 许可证

本项目基于 [MIT License](LICENSE) 发布。Copyright (c) 2026 Rhys Zhang
