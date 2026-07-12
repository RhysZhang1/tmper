# 终端音乐播放器 — 详细设计方案

> **目标平台**: Arch Linux + KDE Plasma + Bash
> **设计目标**: 终端原生音乐播放器，支持多格式音频解码、元数据显示、LRC 歌词同步、频谱律动可视化、纯键盘 Vim 风格操作
> **文档版本**: v2.0
> **最后更新**: 2026-07-11

> **⚠️ 版本说明**：本文中引用的 crate 版本号为撰写时的最新版本。实际开发时应运行 `cargo update` 获取最新稳定版，并查阅各 crate 的 CHANGELOG 确认 API 无 breaking changes。

---

## 目录

1. [语言选择](#1-语言选择)
2. [整体架构](#2-整体架构)
3. [模块设计](#3-模块设计)
4. [UI 布局设计](#4-ui-布局设计)
5. [键盘操作设计](#5-键盘操作设计)
6. [数据流设计](#6-数据流设计)
7. [依赖库选型](#7-依赖库选型)
8. [音频格式兼容方案](#8-音频格式兼容方案)
9. [歌词系统设计](#9-歌词系统补充细节)
10. [频谱可视化设计](#10-频谱可视化补充细节)
11. [播放列表与音乐库](#11-播放列表与音乐库补充)
12. [配置系统](#12-配置系统)
13. [实施路线图](#13-实施路线图)
14. [附录：关键数据结构与项目结构](#14-附录关键数据结构与项目结构)

---

## 1. 语言选择

### 推荐：Rust

| 维度 | Rust | Go | Python | C/C++ |
|------|------|-----|--------|-------|
| 性能 | ★★★★★ | ★★★★☆ | ★★☆☆☆ | ★★★★★ |
| 内存安全 | ★★★★★ | ★★★★☆ | ★★★★★ | ★★☆☆☆ |
| TUI 生态 | ★★★★★ (ratatui) | ★★★★☆ (bubbletea) | ★★★☆☆ (textual) | ★★★☆☆ (FTXUI) |
| 音频生态 | ★★★★☆ (rodio/symphonia) | ★★★☆☆ (beep/oto) | ★★★☆☆ (pygame/pydub) | ★★★★★ (ffmpeg/libav) |
| 打包部署 | ★★★★★ (单二进制) | ★★★★★ (单二进制) | ★★☆☆☆ | ★★★☆☆ |
| 开发效率 | ★★★☆☆ | ★★★★☆ | ★★★★★ | ★★☆☆☆ |

**选择 Rust 的核心理由**：

1. **ratatui** 是目前最成熟的终端 TUI 框架，支持复杂布局、异步渲染、组件化开发，且维护活跃
2. **rodio + symphonia** 组合是 Rust 生态中最成熟的纯 Rust 音频方案。symphonia 是纯 Rust 实现的多格式解码器，无需依赖系统 ffmpeg，真正做到单二进制分发
3. **零成本抽象** — 音频解码和 FFT 计算对实时性敏感，Rust 的无 GC 保证是关键优势
4. **单二进制分发** — `cargo build --release` 产出一个可执行文件，用户不需要安装任何运行时
5. **长期可维护性** — 强类型系统让重构安全，Cargo 依赖管理生态成熟

### 备选：Go

如果团队更熟悉 Go，`bubbletea` + `bubbles` + `lipgloss` 的 TUI 组合体验不错。但 Go 的音频解码生态较弱，通常需要 CGo 绑定 ffmpeg，会失去单二进制和跨平台编译的便利性。频谱计算（FFT）在 Go 中也有成熟库（如 `gonum`），但整体生态不如 Rust 完善。

### 不推荐 Python/C++

- **Python**: TUI 框架（textual/rich）体验尚可，但音频处理和实时 FFT 的性能不足，打包分发依赖 Python 运行时。可使用 `numpy` + `pyaudio` + `pydub` 实现原型，但生产级体验差距明显。
- **C++**: 能力最强大（直接调 ffmpeg/libav），但内存管理、构建系统（CMake）、依赖管理复杂度高，不适合个人项目长期维护。

---

## 2. 整体架构

### 架构全景图

```
┌─────────────────────────────────────────────────────────────┐
│                        main.rs                              │
│                  (初始化日志/配置/启动)                        │
└────────────┬───────────────────────────────┬────────────────┘
             │                               │
    ┌────────▼────────┐              ┌───────▼────────┐
    │   Event Loop     │◄─────────────│   TUI / UI      │
    │  (tokio async)   │              │  (ratatui)      │
    │  事件分发中心      │──────────────►  渲染 + 输入     │
    └──┬────┬────┬─────┘              └────────────────┘
       │    │    │
  ┌────▼┐ ┌▼──┐ ┌▼──────┐
  │Audio│ │LRC│ │Library│
  │Engine│ │Sync│ │Manager│
  └────┬┘ └───┘ └───────┘
       │
  ┌────▼────────┐
  │FFT Analyzer │
  │(频谱分析)     │
  └─────────────┘
```

### 架构分层（自上而下）

```
┌──────────────────────────────────────────┐
│  UI 层 (ratatui + crossterm)              │  ← 渲染管线、输入捕获、布局管理
├──────────────────────────────────────────┤
│  应用层 (AppState + Event Loop)           │  ← 全局状态管理、事件路由、生命周期
├──────────────────────────────────────────┤
│  业务层                                   │
│  ├─ PlayerCore      (播放状态机)          │
│  ├─ LibraryManager  (曲库索引与查询)       │
│  ├─ LyricEngine     (歌词解析与同步)       │
│  └─ VisualizerEngine(频谱计算与后处理)     │
├──────────────────────────────────────────┤
│  基础设施层                                │
│  ├─ AudioDecoder    (Symphonia 适配层)    │
│  ├─ AudioOutput     (rodio + cpal 后端)   │
│  ├─ FileScanner     (walkdir 遍历)        │
│  ├─ MetadataReader  (lofty 标签解析)       │
│  └─ ConfigManager   (toml/serde 配置)     │
└──────────────────────────────────────────┘
```

### 并发模型

使用 **tokio** 作为异步运行时。各模块通过 **多生产者单消费者通道（tokio::mpsc）** 与事件循环通信：

```
                      ┌──────────────┐
  ┌──────────────────►│              │
  │  AudioEngine      │   Event      │
  │  (PositionUpdate, │   Loop       │   ┌─────────────┐
  │   TrackEnd...)    │   (主线程)     │──►│  UI Render   │
  ├──────────────────►│              │   └─────────────┘
  │  VisualizerEngine │  统一收集     │
  │  (BarData)         │  所有事件     │   ┌─────────────┐
  ├──────────────────►│  更新 AppState│──►│  State       │
  │  InputHandler     │              │   │  Mutation    │
  │  (KeyEvent)        │              │   └─────────────┘
  └──────────────────►│              │
     LyricEngine       └──────────────┘
     (Tick for sync)
```

**关键设计决策**：
- 音频解码和 FFT 计算在**独立线程**中运行（通过 `tokio::task::spawn_blocking`），不阻塞事件循环
- UI 渲染在**主线程**执行（ratatui 要求），以固定帧率（约 30-60 FPS）驱动
- 状态更新集中在事件循环中完成，UI 渲染只读 AppState，避免数据竞争

### Symphonia + Rodio FFT 采样要点

从正在播放的音频流中提取 FFT 采样数据需要特殊处理。`rodio::Sink::append()` 会消耗 Source（获取所有权），因此不能简单地在解码后分叉。推荐方案：

1. **自定义 Source Wrapper**：实现一个 `InstrumentedSource<I>`，包裹 symphonia 解码器输出，在 `Source::sample()` 的每次调用中将采样值拷贝到共享的环形缓冲区（`Arc<Mutex<RingBuffer>>`），FFT 线程从该缓冲区读取
2. **双 Sink 不适合 FFT 采集**：文档中提到的双 Sink 方案仅用于 Gapless Playback，与 FFT 采集无关
3. 环形缓冲区大小建议为 `FFT_SIZE * 4`（约 8192 个 f32），确保 FFT 线程有足够的窗口读取而不丢帧

---

## 3. 模块设计

### 3.1 Audio Engine（音频引擎）

**职责**：音频文件的加载、解码、播放控制、音量管理

#### 核心流程

```
音频文件 → Symphonia 探测容器格式 → 选择解码器 → 输出 PCM f32 采样流
                                                    │
                               ┌────────────────────┼────────────────────┐
                               ▼                    ▼                    ▼
                         Rodio Sink           FFT Analyzer         Position Tracker
                         (音频输出到声卡)       (频谱数据)              (播放进度)
```

#### 状态机

```
                  ┌──────────┐
     load()       │  Stopped  │  stop()
    ─────────────►│           │◄─────────────
                  └─────┬─────┘
                        │ play()
                        ▼
                  ┌──────────┐
           ┌──────│ Playing   │──────┐
           │      │           │      │
      pause()    └──────────┘  seek()
           │           ▲           │
           ▼           │           ▼
     ┌──────────┐      │     ┌──────────┐
     │  Paused   │──────┘     │ Seeking   │
     │           │ resume()   │ (瞬态)     │
     └──────────┘             └─────┬─────┘
                                    │ seek 完成
                                    ▼
                               Playing (新位置)
```

#### 关键实现要点

- `symphonia` 提供统一的 `FormatReader` + `Decoder` 接口，运行时自动探测容器和编码
- `rodio::Sink` 作为音频终点，内部通过 `cpal` 与 PulseAudio/PipeWire/ALSA 通信
- 播放位置以**采样帧数**精确追踪（`current_frame: u64`），帧数 / 采样率 = 秒数，用于歌词同步和进度显示
- 音量控制由 `rodio::Sink::set_volume()` 实现（0.0 ~ 1.0，推荐使用对数映射以匹配人耳感知）
- Gapless Playback：在上一曲结束前约 2 秒预加载下一曲到另一个 `Sink`，旧 Sink fade out 同时新 Sink fade in。注意：两个 Sink 之间的精确同步在 rodio 中有一定局限性，可能需要降级为近似 gapless（间隔 < 50ms）

---

### 3.2 Metadata Reader（元数据读取）

**职责**：从音频文件中提取结构化标签信息

使用 **lofty** 库，它支持以下标签格式的读写：

| 标签格式 | 常见容器 |
|----------|----------|
| ID3v1 / ID3v2 | MP3 |
| Vorbis Comments | FLAC, OGG, Opus |
| APE tags | APE, Musepack, WavPack |
| MP4 / iTunes atoms | AAC, ALAC (M4A, MP4) |
| RIFF chunks | WAV |

**提取字段清单**：

| 字段 | 必选 | Fallback |
|------|------|----------|
| `title` | 是 | 文件名（去除扩展名） |
| `artist` | 否 | "Unknown Artist" |
| `album` | 否 | "Unknown Album" |
| `album_artist` | 否 | — |
| `track_number` / `track_total` | 否 | — |
| `disc_number` / `disc_total` | 否 | — |
| `genre` | 否 | — |
| `year` / `date` | 否 | — |
| `duration` | 是 | 从音频流实际解码获取 |
| `cover_art` | 否 | — (Picture 帧) |
| `bitrate` / `sample_rate` / `channels` / `codec` | 是 | 从音频流属性获取 |

---

### 3.3 Lyric Engine（歌词引擎）

**职责**：LRC 文件解析、内嵌歌词提取、实时同步显示

#### LRC 格式规范

```lrc
[ti:歌曲标题]
[ar:艺术家]
[al:专辑]
[by:LRC 制作者]
[offset:+500]           ← 全局时间偏移，单位毫秒，+ 表示歌词整体延后
[length:03:45]          ← 歌曲总时长

[00:12.34]第一句歌词     ← 标准格式：[分:秒.百分之一秒]
[00:15.67]第二句歌词
[00:18.90]<00:18.90>这<00:19.20>是<00:19.50>逐<00:19.80>字<00:20.10>歌<00:20.40>词   ← 增强 LRC
[03:45.00][02:30.00]同一句出现在两个时间点   ← 多时间标签（重复段落）
```

#### LRC 文件查找策略（按优先级）

1. 音频文件同目录下的**同名 `.lrc` 文件**（如 `song.flac` → `song.lrc`）
2. 音频文件**内嵌的歌词标签**（ID3v2 `USLT` / `SYLT` 帧，Vorbis `LYRICS` 字段）— 通过 `lofty` 读取
3. 用户配置中指定的**统一歌词目录**（如 `~/Music/Lyrics/`）
4. （后期可选）在线歌词 API 搜索（网易云、QQ 音乐等）

#### 同步显示算法

```
每个渲染帧（或每 50ms 定时 tick）：
  1. 获取当前播放位置 current_pos（已应用全局 + 用户偏移）
  2. 在 lyrics_lines 中二分查找：
     找到最后一个 timestamp <= current_pos 的行索引
  3. 设为 "当前行"，应用高亮样式
  4. 滚动视口使当前行保持在画面中间偏上位置
  5. 渲染：当前行 → 高亮色 + 加粗
            前 N 行 → 逐步变暗（渐变衰减）
            后 N 行 → 使用 "未来" 样式（淡色）
```

**关键细节**：
- 二分查找从**上次索引附近**开始（不用每次都从头搜），因为时间单调递增
- 用户可微调偏移：`[` 键提前 0.5s，`]` 键延后 0.5s，偏移值持久化到配置
- 无歌词时显示占位信息："No lyrics found"

#### LRC 数据结构

```rust
struct LyricLine {
    timestamp: Duration,                          // 起始时间
    text: String,                                 // 歌词文本（不含时间标签）
    word_timestamps: Vec<(Duration, String)>,     // 逐字时间戳（可选）
}

struct LyricTrack {
    metadata: LyricMetadata,                      // ti, ar, al, offset 等
    lines: Vec<LyricLine>,                        // 按时间排序的歌词行
}

struct LyricMetadata {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    author: Option<String>,
    global_offset_ms: i64,                        // [offset:...] 的值
    length: Option<Duration>,
}
```

---

### 3.4 Visualizer Engine（频谱可视化）

**职责**：从音频 PCM 数据实时计算频谱，输出适合终端渲染的柱状数据

#### 处理流程

```
PCM f32 采样流 (单声道或混合后单声道)
    │
    ▼
┌─ 环形缓冲区 (ring buffer) ─┐
│  持续写入采样，攒够 2048 个  │
│  采样后取出一帧              │
└────────────────────────────┘
    │
    ▼
应用 Hann 窗口函数 (减少频谱泄漏)
    │
    ▼
FFT 变换 (rustfft，O(N log N)，N=2048)
    │
    ▼
频域幅度计算 (real² + imag² 的平方根)
    │
    ▼
对数尺度分桶 (20Hz–16kHz → 32 或 64 个桶)
    │
    ▼
指数移动平均平滑 (EMA，α ≈ 0.3–0.5)
    │
    ▼
动态范围归一化 (基于滑动窗口的历史最大值)
    │
    ▼
输出 → [f32; N_BARS]  → 通过 channel 发送给 UI 层
```

#### 关键参数推荐

| 参数 | 推荐值 | 说明 |
|------|--------|------|
| FFT 窗口大小 | 2048 | 频率分辨率 ≈ 21.5Hz @ 44100Hz 采样率 |
| 频谱柱数量 | 32 (窄终端) / 64 (宽终端) | 太少没细节，太多终端放不下 |
| 刷新率 | 30 FPS | 与 UI 的 visualizer tick 频率一致 |
| 平滑系数 α | 0.35 | EMA: `smooth[i] = α × raw[i] + (1-α) × smooth[i]` |
| 最小/最大频率 | 20Hz / 16000Hz | 覆盖人耳敏感范围，滤除听不到的超低频和高频 |
| 窗口函数 | Hann | 最常用的选择，旁瓣抑制好 |

#### 对数分桶策略

将 20Hz–16kHz 按**对数刻度**（模仿人耳对音高的感知）分为 N 个桶。低频区桶窄（分辨率高，能看清贝斯和鼓），高频区桶宽。

每个桶计算方式：找到落在该频率范围内的所有 FFT bin，取它们的**幅度最大值**或**RMS**。

#### 性能考量

- FFT 计算在独立线程执行，与 UI 渲染完全解耦
- `rustfft` 支持预计算 plan（`FftPlanner`），复用避免每帧分配
- 2048 点 FFT 约需 10–50μs（取决于 CPU），对实时性无影响
- 从 rodio 的 PCM 流通过 `InstrumentedSource` 采集采样数据，不拷贝整段音频

---

### 3.5 Library Manager（曲库管理）

**职责**：扫描音乐目录、构建可搜索索引、管理播放列表

#### 功能清单

- 递归扫描用户配置的音乐目录，发现所有支持的音频文件
- 增量更新：比较文件修改时间 `mtime`，仅重新读取新增或变化的文件
- 索引持久化到本地 SQLite（加速启动、减少重复扫描）
- 多维排序与过滤（按艺术家 → 专辑 → 曲序的层级浏览）
- 全局文本搜索（文件名、标题、艺术家、专辑）
- 播放列表 CRUD：创建、重命名、删除、导入/导出 M3U

#### 数据库 Schema

```sql
CREATE TABLE tracks (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    path        TEXT    NOT NULL UNIQUE,
    title       TEXT,
    artist      TEXT,
    album       TEXT,
    album_artist TEXT,
    track_num   INTEGER,
    disc_num    INTEGER,
    genre       TEXT,
    year        INTEGER,
    duration_s  REAL,
    bitrate     INTEGER,
    sample_rate INTEGER,
    channels    INTEGER,
    codec       TEXT,
    file_size   INTEGER,
    file_mtime  INTEGER,    -- 文件修改时间戳，用于增量扫描
    added_at    INTEGER     -- 首次加入曲库的时间戳
);

CREATE INDEX idx_artist ON tracks(artist);
CREATE INDEX idx_album  ON tracks(album);
CREATE INDEX idx_genre  ON tracks(genre);
```

#### 增量扫描算法

```
对每个配置的监控目录：
  for each audio_file in walkdir(monitor_dir):
      if audio_file.extension NOT IN supported_extensions:
          skip
      if audio_file.path exists in DB:
          if audio_file.mtime == DB.file_mtime:
              skip                              // 未变化
          else:
              re-read metadata → UPDATE          // 文件已修改
      else:
          read metadata → INSERT                 // 新文件

  // 清理：DB 中有但文件系统不存在的记录
  for each record in DB WHERE path like 'monitor_dir%':
      if !Path::exists(record.path):
          mark as stale / DELETE
```

---

### 3.6 Config Manager（配置管理）

**职责**：读写配置、管理运行时状态持久化

#### 文件位置（XDG Base Directory 规范）

| 路径 | 用途 |
|------|------|
| `~/.config/termusic/config.toml` | 主配置文件 |
| `~/.config/termusic/keybindings.toml` | 自定义快捷键 |
| `~/.local/share/termusic/library.db` | 曲库 SQLite 索引 |
| `~/.local/share/termusic/state.json` | 退出时保存的运行时状态 |
| `~/.cache/termusic/` | 封面图片缓存等 |

#### 主配置结构

```toml
[library]
music_dirs = ["~/Music", "/mnt/media/music"]
extensions = ["mp3", "flac", "ogg", "opus", "wav", "aac", "m4a", "ape", "wv", "aiff", "wma"]
scan_on_startup = false            # 是否启动时全量扫描
follow_symlinks = true

[playback]
default_volume = 0.8               # 0.0 – 1.0
gapless = true
crossfade_seconds = 2              # 0 表示关闭
resume_on_startup = true           # 启动时恢复上一次播放位置
seek_step_small_secs = 5           # 方向键快进/退秒数
seek_step_large_secs = 30          # Shift+方向键快进/退秒数

[visualizer]
enabled = true
num_bars = 32                      # 频谱柱数（32/48/64）
frame_rate = 30                    # 最高刷新率
smoothing = 0.35                   # EMA 平滑系数
char_set = "blocks"                # "blocks" | "braille" | "ascii"
color_scheme = "gradient"          # "gradient" | "solid" | "mono"
show_on_idle = true                # 暂停时是否继续显示（静默跳动）

[lyrics]
auto_load = true
encoding_fallbacks = ["utf-8", "gbk", "shift-jis"]   # 尝试的解码顺序
display_lines_before = 4           # 当前行上方显示的行数
lrc_search_embedded = true         # 是否读取内嵌歌词

[ui]
theme = "tokyo-night"             # 预设主题名或自定义主题路径
show_progress_bar = true
show_cover_art = true
cover_art_max_width = 25           # 字符宽度
default_view = "player"            # 启动时的默认视图
```

#### 配置加载优先级

```
命令行参数 > 环境变量 > 用户配置文件 > 硬编码默认值
```

---

## 4. UI 布局设计

### 4.1 默认布局（Player View）

```
┌──────────────────────────────────────────────────────────────┐
│  ▶  Bohemian Rhapsody — Queen                     03:12 / 05:55 │  ← 顶部信息栏
│  ████████████████░░░░░░░░░░░░░░░░░░░░░░░  Vol: 80%  🔁 🎲   │  ← 进度条 + 状态
│  ─────────────────────────────────────────────────────────── │
│                                                              │
│   ▁▂▃▄▅▆▇█▇▆▅▄▃▂▁ ▁▂▃▄▅▆▇█▇▆▅▄▃▂▁ ▁▂▃▄▅▆▇█▇▆▅▄▃▂▁    │  ← 频谱区 (~5 行)
│   ▂▃▄▅▆▇█▇▆▅▄▃▂▁ ▂▃▄▅▆▇█▇▆▅▄▃▂▁ ▂▃▄▅▆▇█▇▆▅▄▃▂▁    │
│   ▃▄▅▆▇████▇▆▅▄▃ ▃▄▅▆▇█▇▆▅▄▃▂▁ ▃▄▅▆▇████▇▆▅▄▃    │
│   ▂▃▄▅▆▇█▇▆▅▄▃▂ ▂▃▄▅▆▇█▇▆▅▄▃▂ ▃▄▅▆▇█▇▆▅▄▃▂     │
│   ▁▂▃▄▅▆▇█▇▆▅▄▃▂ ▁▂▃▄▅▆▇█▇▆▅▄▃ ▁▂▃▄▅▆▇█▇▆▅▄▃▂     │
│                                                              │
│  ─────────────────────────────────────────────────────────── │
│       Is this the real life? Is this just fantasy?           │  ← 上一行 (dim)  │
│       Caught in a landslide, no escape from reality          │  ← 更前一行      │
│  ▶    Open your eyes, look up to the skies and see           │  ← 当前行 (高亮)  │
│       I'm just a poor boy, I need no sympathy               │  ← 下一行 (淡色)  │
│       Because I'm easy come, easy go, little high, little low│  ← 下两行         │
│                                                              │
│  ─────────────────────────────────────────────────────────── │
│    1. Bohemian Rhapsody — Queen                    05:55    │  ← 播放列表区域
│  ▶ 2. Another One Bites the Dust — Queen           03:36    │     (占据剩余行)
│    3. Don't Stop Me Now — Queen                    03:29    │
│    4. We Will Rock You — Queen                     02:01    │
│    5. We Are the Champions — Queen                 02:59    │
│    ...                                                       │
│  ─────────────────────────────────────────────────────────── │
│  Playlist: Queen's Best  │ 42 tracks │ 3h 28m │ 🔁 list 🔀  │  ← 底部状态栏
└──────────────────────────────────────────────────────────────┘
```

### 4.2 多视图系统

应用支持多个视图，通过数字键或命令切换。每个视图复用相同的全局状态（当前歌曲、播放状态等），但布局和聚焦数据不同。

| 视图 | 键 | 描述 | 主面板内容 |
|------|-----|------|-----------|
| **Player** | `1` | 默认视图 | 频谱 + 歌词 + 当前播放列表 |
| **Library** | `2` | 按艺术家/专辑/流派浏览 | 三栏浏览器（左：艺术家，中：专辑，右：曲目） |
| **Lyrics** | `3` | 全屏歌词（KTV 风格） | 居中大字体当前行 + 前后各 3 行 |
| **Visualizer** | `4` | 全屏频谱（鱼缸模式） | 放大版频谱，铺满终端 |
| **Playlists** | `5` | 播放列表管理器 | 左侧：播放列表列表，右侧：选中列表的曲目 |
| **Browser** | `6` | 文件系统浏览器 | 目录树 + 文件列表 + 预览 |

### 4.3 主题系统

主题定义为 TOML 文件（`~/.config/termusic/themes/custom.toml`），基于 ratatui 的 `Style` 系统：

```toml
[colors]
bg           = "#1a1b26"    # Tokyo Night 风格示例
fg           = "#c0caf5"
accent       = "#7aa2f7"
highlight    = "#bb9af7"
dimmed       = "#565f89"
success      = "#9ece6a"
warning      = "#e0af68"
error        = "#f7768e"

[visualizer]
bar_low      = "#9ece6a"    # 20–200 Hz  低音 → 绿
bar_mid      = "#e0af68"    # 200–2k Hz  中音 → 黄
bar_high     = "#f7768e"    # 2k–16k Hz  高音 → 红

[lyrics]
current_line = { fg = "#7aa2f7", bold = true }
past_line    = { fg = "#565f89" }
future_line  = { fg = "#9aa5ce" }

[progress]
filled = { fg = "#bb9af7" }
empty  = { fg = "#3b4261" }

[playlist]
playing_marker = { fg = "#9ece6a", bold = true }
selected       = { fg = "#7aa2f7", bg = "#292e42" }
```

预设主题：`tokyo-night`、`dracula`、`nord`、`solarized-dark`、`catppuccin-mocha`

---

## 5. 键盘操作设计

### 5.1 Vim 模态操作

借鉴 Vim 的模态编辑理念，将操作分为四种模式：

| 模式 | 进入方式 | 用途 |
|------|----------|------|
| **Normal** | 默认 / `Esc` | 导航、播放控制、常规操作 |
| **Insert** | `i` / `a` / `/` | 文本输入（搜索、过滤、重命名） |
| **Command** | `:` | 执行命令（如 `:playlist save xxx`） |
| **Visual** | `v` | 多选曲目，用于批量操作 |

### 5.2 Normal Mode 完整键位表

#### 播放控制

| 键 | 功能 | 说明 |
|----|------|------|
| `Space` | 播放 / 暂停 | 切换播放状态 |
| `s` | 停止 | 停止播放并重置位置 |
| `n` | 下一首 | 跳到播放列表下一项 |
| `p` | 上一首 | 跳到播放列表上一项 |
| `←` | 快退 5 秒 | 可配置步长 |
| `→` | 快进 5 秒 | 可配置步长 |
| `Shift+←` | 快退 30 秒 | 大步后退 |
| `Shift+→` | 快进 30 秒 | 大步前进 |
| `-` | 降低音量 5% | |
| `=` | 增加音量 5% | |
| `m` | 静音切换 | toggle mute |

#### 导航（Vim 风格）

| 键 | 功能 |
|----|------|
| `j` / `↓` | 向下移动光标 |
| `k` / `↑` | 向上移动光标 |
| `h` / `←` | 在分栏视图中向左移动 |
| `l` / `→` | 在分栏视图中向右移动 |
| `g` `g` | 跳到列表顶部 |
| `G` | 跳到列表底部 |
| `Ctrl+d` | 向下翻半页 |
| `Ctrl+u` | 向上翻半页 |
| `Ctrl+f` | 向下翻整页 |
| `Ctrl+b` | 向上翻整页 |
| `z` `z` | 将当前选中项滚动到屏幕中央 |
| `z` `t` | 将当前选中项滚动到屏幕顶部 |

#### 搜索

| 键 | 功能 |
|----|------|
| `/` | 进入搜索模式（正向搜索，输入时实时过滤） |
| `?` | 反向搜索 |
| `n` | 跳到下一个匹配项 |
| `N` | 跳到上一个匹配项 |
| `Esc` | 退出搜索 / 清除过滤 |

#### 播放列表操作

| 键 | 功能 |
|----|------|
| `Enter` | 播放当前选中的歌曲 |
| `a` | 将所选歌曲/目录添加到播放列表末尾 |
| `A` | 将所选内容插入到当前歌曲之后（插队播放） |
| `d` `d` | 从播放列表中移除当前项 |
| `y` `y` | 复制当前歌曲信息到剪贴板 |
| `o` | 打开文件浏览器 |
| `r` | 切换重复模式（off → track → playlist） |
| `R` | 切换随机播放 |
| `x` | 清除播放队列（临时排队列表） |

#### 视图切换

| 键 | 功能 |
|----|------|
| `1` | 切换到播放器视图 |
| `2` | 切换到曲库浏览器 |
| `3` | 切换到全屏歌词 |
| `4` | 切换到全屏频谱 |
| `5` | 切换到播放列表管理 |
| `6` | 切换到文件浏览器 |
| `Tab` | 在多面板布局中切换焦点面板 |
| `Shift+Tab`| 反向切换焦点面板 |

#### 歌词控制

| 键 | 功能 |
|----|------|
| `[` | 歌词提前 0.5 秒（offset -500ms） |
| `]` | 歌词延后 0.5 秒（offset +500ms） |
| `{` | 歌词大幅提前 2 秒 |
| `}` | 歌词大幅延后 2 秒 |
| `Ctrl+r` | 重置歌词偏移为 0 |

#### 其他

| 键 | 功能 |
|----|------|
| `q` | 退出应用 |
| `Ctrl+l` | 强制刷新屏幕（类似 Vim 的 `Ctrl+l`） |
| `?` | 显示帮助面板（键位速查） |

### 5.3 Command Mode 命令参考

在 Normal Mode 下按 `:` 进入：

```text
# 播放列表
:playlist load <name>        加载已保存的播放列表
:playlist save <name>        保存当前播放列表
:playlist delete <name>      删除播放列表
:playlist clear              清空当前播放列表
:playlist shuffle            随机打乱
:playlist sort title|artist|album|date  排序

# 曲库
:library scan                重新扫描所有音乐目录
:library add <path>          添加新目录到监控列表
:library stats               显示曲库统计信息

# 播放
:seek <time>                 跳转到指定时间 (如 :seek 1:30)
:seek <percentage>%          跳转到百分比位置 (如 :seek 50%)
:volume <0-100>              设置音量
:repeat off|track|playlist   设置循环模式
:shuffle on|off              切换随机播放

# 界面
:theme <name>                切换主题
:view <name>                 切换视图 (player/library/lyrics/visualizer/playlists/browser)
:columns <n>                 设置频谱柱数

# 系统
:q / :quit / :wq             退出
:help                        打开帮助页面
:version                     显示版本信息
```

### 5.4 自定义快捷键

`~/.config/termusic/keybindings.toml`：

```toml
[normal]
play_pause       = " "
stop             = "s"
next_track       = "n"
prev_track       = "p"
seek_forward     = "Right"
seek_backward    = "Left"
seek_forward_big = "Shift+Right"
seek_backward_big = "Shift+Left"
vol_down         = "-"
vol_up           = "="
toggle_mute      = "m"
quit             = "q"

[navigation]
down             = "j"
up               = "k"
page_down        = "Ctrl+d"
page_up          = "Ctrl+u"
top              = "gg"
bottom           = "G"
search           = "/"

# 支持组合键、多键序列（如 gg）、功能键
# 底层使用 crossterm 的 KeyEvent 匹配
```

---

## 6. 数据流设计

### 6.1 播放一首歌的完整数据流

```
用户在播放列表中按 Enter 选歌
    │
    ▼
InputHandler 捕获 Enter → 发送 KeyEvent::Enter 到事件循环
    │
    ▼
Event Loop 匹配当前上下文（焦点在播放列表）
    → 取出当前选中项的 track_id
    → 发送 PlayEvent(track_id)
    │
    ▼
PlayerCore::play(track_id):
    │
    ├─► LibraryManager 根据 track_id 查询完整路径
    │
    ├─► 创建 AudioEngine::load(path):
    │     ├── Symphonia 打开文件 → 探测容器 (MP3/FLAC/OGG/...) → 创建解码器
    │     ├── 解码器产生 PCM f32 采样流
    │     ├── 采样流分叉为两路：
    │     │   ├── Rodio Sink → 声卡 (用户听到声音)
    │     │   └── 环形缓冲区 → FFT 分析器读取
    │     └── 启动后台线程持续喂数据
    │
    ├─► MetadataReader::read(path) → TrackInfo
    │
    └─► LyricEngine::load(path):
          ├── 查找同名 .lrc 文件
          ├── 未找到 → 读取音频内嵌歌词 (lofty USLT/SYLT)
          └── 解析 LRC 文本 → LyricTrack
    │
    ▼
事件循环进入 "Playing" 状态，启动定期 tick (每 50ms)：
    │
    ├── tick: 查询 AudioEngine 当前播放位置
    │     ├── 更新进度条
    │     └── 调用 LyricEngine::sync(position) → 更新当前歌词行
    │
    ├── visualizer_event: 接收 FFT 分析结果
    │     └── 更新 visualizer_data: Vec<f32>
    │
    └── track_end_event: 当前曲目播放完毕
          └── 根据循环模式：播放下一首 / 停止 / 单曲循环
    │
    ▼
每次状态变更后 → terminal.draw(render_fn) → 用户看到更新后的 UI
```

### 6.2 事件类型定义

```rust
enum AppEvent {
    // ── 输入 ──
    Key(crossterm::event::KeyEvent),
    Mouse(crossterm::event::MouseEvent),
    Resize(u16, u16),

    // ── 音频引擎 ──
    TrackStarted(TrackInfo),
    PositionUpdated(Duration),         // 播放位置（用于进度条和歌词）
    TrackEnded,
    PlaybackError(String),

    // ── 可视化 ──
    VisualizerData(Vec<f32>),          // 频谱柱数据，长度 = config.visualizer.num_bars

    // ── 定时器 ──
    Tick,                               // 定期触发（UI 刷新、歌词同步）

    // ── 系统 ──
    Quit,
}
```

### 6.3 状态变更原则

- **单一写入者**：AppState 的所有变更集中在事件循环的一个 `handle_event()` 函数中完成
- **只读渲染**：UI 渲染函数只读取 `&AppState`，不做任何修改
- **通道通信**：子系统的结果通过 `tokio::mpsc::unbounded_channel()` 发送给事件循环，不直接操作状态
- **不可变更新**：遵循 `update(old, changes) → new` 的不可变模式（通过 `std::mem::take` 或直接替换字段）

---

## 7. 依赖库选型

### 核心依赖（Cargo.toml 骨架）

| 类别 | 库 | 用途 | 备注 |
|------|-----|------|------|
| 异步运行时 | `tokio = { version = "1", features = ["full"] }` | 事件循环、定时器、通道 | 全特性启用 |
| TUI 框架 | `ratatui = "0.29"` | Widget 系统、布局引擎 | 查最新稳定版 |
| 终端控制 | `crossterm = "0.28"` | 原始模式、键盘事件、颜色、光标 | ratatui 的默认后端 |
| 音频输出 | `rodio = "0.20"` | 跨平台音频 Sink | 基于 cpal |
| 音频解码 | `symphonia = { version = "0.5", features = ["all"] }` | 多格式解码器 | 启用所有容器和编码格式 |
| 元数据读取 | `lofty = "0.22"` | 音频标签解析 | 支持所有主要标签格式 |
| FFT 计算 | `rustfft = "6"` | 快速傅里叶变换 | 纯 Rust，支持 SIMD |
| 配置解析 | `toml = "0.8"` | TOML 配置文件 | |
| 序列化 | `serde = { version = "1", features = ["derive"] }` | 配置/状态的序列化 | |
| 命令行参数 | `clap = { version = "4", features = ["derive"] }` | 命令行接口 | |

### 辅助依赖

| 库 | 用途 |
|----|------|
| `walkdir = "2"` | 递归遍历目录 |
| `regex = "1"` | LRC 解析正则、用户搜索 |
| `encoding_rs = "0.8"` | 处理非 UTF-8 编码的歌词文件（GBK、Shift-JIS 等） |
| `tracing = "0.1"` | 结构化日志框架 |
| `tracing-subscriber = "0.3"` | 日志输出（终端/文件） |
| `thiserror = "2"` | 自定义错误类型派生 |
| `anyhow = "1"` | 应用级错误传播（main 和顶层函数） |
| `rusqlite = { version = "0.32", features = ["bundled"] }` | 曲库索引存储 |
| `unicode-width = "0.2"` | Unicode 字符宽度计算（CJK 歌词对齐） |
| `dirs = "6"` | XDG 目录路径（跨平台） |

### 可选依赖（后期引入）

| 库 | 用途 |
|----|------|
| `viuer = "0.9"` | 在支持的终端中渲染封面图片（Kitty/iTerm2 协议） |
| `notify-rust = "4"` | 切歌时发送桌面通知（通过 D-Bus） |
| `reqwest = "0.12"` | HTTP 客户端（在线歌词搜索、封面下载） |
| `serde_json = "1"` | JSON 处理（运行时状态持久化、在线 API） |
| `mpdris2` (自实现或社区) | MPRIS2 D-Bus 接口（媒体键、KDE 整合） |

---

## 8. 音频格式兼容方案

### 8.1 Symphonia 原生支持矩阵

| 容器格式 | 扩展名 | 常见编码 | 支持状态 |
|----------|--------|----------|----------|
| MPEG Audio (MP3) | `.mp3` | MPEG Layer I/II/III | ✅ 稳定 |
| FLAC | `.flac` | FLAC | ✅ 稳定 |
| OGG | `.ogg` | Vorbis | ✅ 稳定 |
| OGG | `.opus` | Opus | ✅ 稳定 |
| RIFF WAVE | `.wav` | PCM, ADPCM | ✅ 稳定 |
| ADTS | `.aac` | AAC (LC, HE, HEv2) | ✅ 稳定 |
| MP4 / ISOBMFF | `.m4a`, `.mp4` | AAC, ALAC | ⚠️ 部分支持（取决于具体编码和 symphonia 版本） |
| Matroska/WebM | `.mka`, `.webm` | 多种 | ⚠️ 实验性 |
| WavPack | `.wv` | WavPack | ⚠️ 部分支持 |
| Monkey's Audio | `.ape` | APE | ❌ 暂不支持 |

> **注意**：symphonia 的 MP4/ISOBMFF 支持在持续改进。开发前应查阅 symphonia 最新 changelog 确认当前支持范围。

### 8.2 处理不支持的格式

对于 Symphonia 不直接支持的格式，提供**分级降级策略**：

1. **首选**：Symphonia 解码（零依赖、最快）
2. **降级方案**：调用系统 `ffmpeg` 命令行工具解码为原始 PCM 后播放（需要用户安装 ffmpeg）。实现方式：`std::process::Command` 调用 `ffmpeg -i input.ape -f f32le -acodec pcm_f32le -ar 44100 -ac 2 pipe:1`，读取 stdout 送入 rodio
3. **用户提示**：遇到无法播放的格式时给出明确提示，建议转换或安装 ffmpeg

### 8.3 音频属性兼容

- **采样率**：Symphonia 支持 8kHz – 384kHz。rodio/cpal 会自动处理采样率转换（通过其内部的混音器），但建议在送入 Sink 前用 symphonia 的 `SampleBuffer` 统一重采样到 44100Hz，简化 FFT 逻辑
- **声道**：Mono / Stereo / 5.1 / 7.1，FFT 分析时将多声道混合为 Mono（取各声道均值）
- **位深**：Symphonia 解码为 f32 / i32 / i16 等，统一标准化到 f32 [-1.0, 1.0] 范围（symphonia 已内置此转换）

---

## 9. 歌词系统补充细节

### 9.1 LRC 状态机解析器

```
初始状态: Idle
    │
    ├── 行首匹配 [ti:...] / [ar:...] / [al:...] / [by:...]
    │     → 提取为 LyricMetadata 字段
    │
    ├── 行首匹配 [offset:±n]
    │     → LyricMetadata.global_offset_ms = n
    │
    ├── 行首匹配 [mm:ss.cc]
    │     ├── 收集该行所有时间标签（可能多个）
    │     ├── 剩余文本作为歌词内容
    │     ├── 如果内容包含 <mm:ss.cc> 标签 → 解析为逐字时间戳
    │     └── 为每个时间标签创建一个 LyricLine（同一行内容可对应多个时间）
    │
    ├── 纯时间标签行（无文本）→ 跳过（可能是乐器段落标记）
    │
    └── 纯文本行（无时间标签）→ 忽略或记录为注释
```

### 9.2 编码探测

中文歌词（尤其从网络下载的 LRC 文件）常见编码问题：

1. 首先尝试 UTF-8 解码
2. 失败时依次尝试：GBK → GB2312 → Shift-JIS → Latin-1
3. 使用 `encoding_rs` 库的 `Encoding::for_bom()` 检测 BOM 标记
4. 用户可在配置中显式指定编码

### 9.3 逐字高亮（增强 LRC）

如果 `LyricLine.word_timestamps` 非空，渲染时：
- 已唱过的字 → 已完成样式（如主题色）
- 当前正在唱的字 → 高亮 + 闪烁或下划线
- 尚未唱到的字 → 默认前景色

在普通 LRC 中（无逐字信息），整行作为一个整体高亮。

---

## 10. 频谱可视化补充细节

### 10.1 字符集对比

| 字符集 | 分辨率 | 示例 | 兼容性 | 推荐场景 |
|--------|--------|------|--------|----------|
| Block Elements (▁▂▃▄▅▆▇█) | 8 级 | `▃▅▇█▇▅▃` | ★★★★★ | **默认推荐** |
| Braille Dots (⣀⣤⣶⣿) | 8 级（垂直） | 更高密度 | ★★★★☆ | 精细频谱 |
| Half Blocks (▄█) + 颜色 | 2 级 + 色彩 | `█▄▄█` | ★★★★★ | 彩色方案 |
| 纯 ASCII | 可变 | `.-~*#@` | ★★★★★ | 最大兼容 |

推荐 **Block Elements + 颜色渐变**：宽度上每个柱状占 1-2 字符，高度由 8 级 block 字符表示，颜色从绿→黄→红表示低频→高频。

### 10.2 渲染优化

- 频谱数据通过无锁 channel（`tokio::mpsc` 足够，因为 FFT 帧率远低于 UI 帧率）发送，不阻塞音频线程
- UI 帧率通过 `tokio::time::interval` 控制，在渲染函数中检查距上次刷新是否 ≥ `1000/fps` ms
- 在暂停或无音频时显示"静默律动"（逐渐衰减至零）或保持最后一帧

### 10.3 动态范围压缩

原始 FFT 的幅度范围很大（安静段落 vs 高潮段落），不做处理会让视觉体验差：

```
对每一帧:
  1. 当前帧最大值 = max(bars)
  2. 滑动窗口最大值 = EMA(历史帧最大值, α=0.1)
  3. 将 bars 线性映射到 0..1，除以 max(当前帧最大值, 滑动窗口最大值 × 0.3)
  4. 最终归一化到 0..=7 的字符级别
```

这样做的好处：安静段落不会被压成一条直线，高潮段落也不会一直顶满。

---

## 11. 播放列表与音乐库补充

### 11.1 M3U 格式兼容

支持标准 M3U 和扩展 M3U（`#EXTM3U`）：

```m3u
#EXTM3U
#EXTINF:355,Bohemian Rhapsody - Queen
/home/user/Music/Queen/Bohemian_Rhapsody.flac
#EXTINF:216,Another One Bites the Dust - Queen
/home/user/Music/Queen/Another_One_Bites_the_Dust.mp3
```

- 导入时解析 `#EXTINF` 获取时长和标题（作为 fallback）
- 导出时写入扩展 M3U 格式（被大多数播放器支持）

### 11.2 播放队列模型

```
播放顺序优先级（从高到低）：
  1. 临时队列 (Queue)        ← 用户按 'A' 插入队列
  2. 播放列表当前位置         ← 正常顺序播放
  3. 随机模式：在播放列表中随机选择
```

临时队列播放完毕后自动回到播放列表的正常顺序。

---

## 12. 配置系统

配置系统已在 [3.6 Config Manager](#36-config-manager配置管理) 中详述。此处补充配置文件的实际存放路径约定和首次运行引导流程：

### 首次运行引导

```
termusic 首次启动:
  1. 检查 ~/.config/termusic/config.toml 是否存在
  2. 若不存在：
     a. 创建 ~/.config/termusic/ 目录
     b. 将内嵌的 default.toml 复制到 ~/.config/termusic/config.toml
     c. 创建 ~/.local/share/termusic/ 目录（用于 library.db）
     d. 创建 ~/.cache/termusic/ 目录（用于封面缓存）
     e. 显示首次运行欢迎信息，引导用户设置 music_dirs
  3. 加载配置并启动
```

### 配置热重载

- 监听配置文件变更（通过 `notify` crate 的 `inotify` 后端）
- 变更时自动重新加载并应用（主题切换立即生效，不需重启）
- 部分配置（如 `music_dirs`）需要手动触发 `:library scan` 生效

---

## 13. 实施路线图（AI 开发优化版）

> **设计理念**：本路线图专为 AI 辅助开发优化。每个 Session 是一个**自包含的 AI 编码任务**：
> - 包含该 Session 所需的全部上下文（不必回翻文档）
> - 明确输入文件、输出文件、函数签名、测试策略
> - 完成后可独立编译和测试
> - 标注了可并行的 Session（同一 Phase 内无依赖的 Session 可同时派发）
>
> **使用方式**：将每个 Session 的完整内容（包括上下文块）直接作为 AI 编码助手的 prompt，逐 Session 推进。

---

### 全局 AI 开发约定

以下约定适用于所有 Session，每个 Session 中不再重复：

1. **错误处理**：使用 `anyhow::Result<T>` 作为公开 API 返回值，使用 `thiserror` 派生自定义错误类型 `#[derive(Error, Debug)] pub enum AppError { ... }`（集中定义在 `src/error.rs`）
2. **日志**：关键路径使用 `tracing::info!` / `tracing::debug!` / `tracing::error!` 记录
3. **测试**：每个包含逻辑的模块都要有 `#[cfg(test)] mod tests { ... }`，覆盖正常路径 + 边界条件
4. **不可变性**：遵循"创建新值、不修改旧值"的原则
5. **提交**：每个 Session 完成后 `cargo fmt --all && cargo clippy -- -D warnings && cargo test` 全部通过后再 commit
6. **依赖添加**：Session 中如需要新 crate，统一加到 `Cargo.toml`，并注明版本（先查 `cargo search <name>` 确认最新版）
7. **版本管理**：在本地用git做版本管理，完成一部分提交一部分，全部留在本地，不用推送任何在线网站。
---

### 开发阶段总览

```
Phase 0 ──► Phase 1 ──► Phase 2 ──► Phase 3 ──► Phase 5
  (1 session) (4 sessions) (6 sessions) (4 sessions) (7 sessions)
                                    │
                                    └──► Phase 4 ──┘
                                      (4 sessions, 与 P3 并行)

Phase 6: 独立扩展（各 1 session，无依赖）
```

| Phase | 内容 | Session 数 | 并行机会 |
|-------|------|-----------|----------|
| P0 | 项目脚手架 | 1 | — |
| P1 | 核心播放 MVP | 4 | S1.2 和 S1.3 可并行 |
| P2 | 播放列表 + 元数据 | 6 | S2.1/S2.3/S2.5 可并行 |
| P3 | 歌词系统 | 4 | S3.1 和 S3.2 可并行 |
| P4 | 频谱可视化 | 4 | S4.2 独立 |
| P5 | 曲库 + 高级 UI | 7 | S5.2/S5.3/S5.4 可并行 |
| P6 | 扩展 | 9 | 全部独立并行 |

---

### Phase 0：项目脚手架（1 Session）

---

#### Session 0.1 — 初始化项目骨架

| 属性 | 值 |
|------|-----|
| **依赖** | 无 |
| **产出文件** | `Cargo.toml`, `src/main.rs`, `src/error.rs`, 全部 `src/**/mod.rs` |

**任务描述**：

1. 运行 `cargo init termusic --name termusic` 创建项目
2. 编辑 `Cargo.toml`：
   - `[package]`：`name = "termusic"`, `version = "0.1.0"`, `edition = "2021"`, `license = "MIT"`
   - `[profile.release]`：`opt-level = 3`, `lto = true`, `codegen-units = 1`, `strip = true`
   - `[dependencies]` 添加以下 crate（版本号写 `cargo search` 查到的最新稳定版）：
     ```toml
     tokio = { version = "1", features = ["full"] }
     ratatui = "0.29"
     crossterm = "0.28"
     rodio = "0.20"
     symphonia = { version = "0.5", features = ["all"] }
     lofty = "0.22"
     clap = { version = "4", features = ["derive"] }
     toml = "0.8"
     serde = { version = "1", features = ["derive"] }
     tracing = "0.1"
     tracing-subscriber = "0.3"
     anyhow = "1"
     thiserror = "2"
     dirs = "6"
     ```
   - **暂时不加** `rustfft`, `rusqlite`, `regex`, `encoding_rs`, `walkdir`, `unicode-width` — 这些在后续 Session 用到时再加

3. 创建 `src/error.rs`，定义全局错误类型：
   ```rust
   use thiserror::Error;
   #[derive(Error, Debug)]
   pub enum AppError {
       #[error("Audio error: {0}")]
       Audio(String),
       #[error("Metadata error: {0}")]
       Metadata(String),
       #[error("IO error: {0}")]
       Io(#[from] std::io::Error),
       #[error("Config error: {0}")]
       Config(String),
       #[error("Lyrics error: {0}")]
       Lyrics(String),
   }
   pub type AppResult<T> = anyhow::Result<T>;
   ```

4. 按附录 B 的目录结构创建所有 `mod.rs` 文件（每个模块声明空子模块），确保 `cargo check` 通过

5. 在 `src/main.rs` 中初始化 tracing：
   ```rust
   fn main() {
       tracing_subscriber::fmt().with_writer(std::io::stderr).init();
       tracing::info!("termusic starting...");
   }
   ```

6. 创建 `config/default.toml`（从文档第 3.6 节复制默认配置模板）

7. 创建 `.github/workflows/ci.yml`：`cargo check` + `cargo test` + `cargo clippy` + `cargo fmt --check`

**验收条件**：
- [ ] `cargo build` 零 warning
- [ ] `cargo test` 通过
- [ ] `cargo clippy` 零 warning
- [ ] `cargo run` 输出 "termusic starting..."

---

### Phase 1：核心播放 MVP（4 Sessions）

**目标**：播放单个音频文件，终端显示标题和进度条，Space 暂停/恢复，q 退出

---

#### Session 1.1 — Symphonia 音频解码器

| 属性 | 值 |
|------|-----|
| **依赖** | Phase 0 完成 |
| **输入** | `src/audio/decoder.rs`（空模块） |
| **输出** | `src/audio/decoder.rs`, `tests/fixtures/` |

**任务描述**：

实现 `src/audio/decoder.rs`，一个从音频文件解码出 f32 PCM 采样的适配层：

```rust
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::Decoder;
use symphonia::core::formats::FormatReader;
use symphonia::core::io::MediaSourceStream;

pub struct AudioDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    pub sample_rate: u32,
    pub channels: u8,
    pub total_frames: u64,
}

impl AudioDecoder {
    /// 打开音频文件，自动探测容器格式和编码
    pub fn open(path: &std::path::Path) -> crate::error::AppResult<Self> { ... }

    /// 解码下一个 packet，返回 f32 PCM 采样（-1.0..1.0）
    /// 返回 None 表示流结束
    pub fn read_packet(&mut self) -> crate::error::AppResult<Option<Vec<f32>>> { ... }

    /// 总时长（秒）
    pub fn duration_secs(&self) -> f64 { self.total_frames as f64 / self.sample_rate as f64 }
}
```

关键实现要点：
- 使用 `symphonia::default::get_probe()` 探测格式
- 选择默认 track（通常只有一条音轨）
- 使用 `SampleBuffer::new()` 将解码输出统一转换为 f32
- 处理 `DecodeError` 和 `IoError`，转为 `AppError::Audio`

**测试要求**：
- 在 `tests/fixtures/` 放一个短的测试音频文件（WAV 或 FLAC，几秒即可，可以用 `ffmpeg` 生成：`ffmpeg -f lavfi -i "sine=frequency=440:duration=2" -ar 44100 -ac 2 tests/fixtures/test.wav`）
- 写 `#[test] fn test_decode_wav()`：验证 `open()` 成功、`read_packet()` 返回 > 0 个采样、`sample_rate` = 44100

**验收条件**：
- [ ] `cargo test` 通过（解码测试）
- [ ] `cargo build` 零 warning

---

#### Session 1.2 — Rodio 音频输出管理

| 属性 | 值 |
|------|-----|
| **依赖** | Phase 0 完成（可与 S1.1 并行） |
| **输入** | `src/audio/output.rs`（空模块） |
| **输出** | `src/audio/output.rs` |

**任务描述**：

实现 `src/audio/output.rs`，封装 rodio 的音频输出：

```rust
pub struct AudioOutput {
    sink: rodio::Sink,
    _stream: rodio::OutputStream,   // 必须保持存活！
}

impl AudioOutput {
    pub fn new() -> crate::error::AppResult<Self> {
        // 创建 OutputStream，获取 handle
        // 用 handle 创建 Sink
    }

    /// 将 PCM 采样送入 Sink 播放
    pub fn play_raw(&self, samples: Vec<f32>, sample_rate: u32, channels: u8) {
        let source = rodio::buffer::SamplesBuffer::new(channels as u16, sample_rate, samples);
        self.sink.append(source);
    }

    pub fn pause(&self) { self.sink.pause(); }
    pub fn play(&self) { self.sink.play(); }
    pub fn stop(&self) { self.sink.stop(); self.sink.clear(); }
    pub fn set_volume(&self, vol: f32) { self.sink.set_volume(vol); }
    pub fn is_paused(&self) -> bool { self.sink.is_paused() }
    pub fn empty(&self) -> bool { self.sink.empty() }
    pub fn len(&self) -> usize { self.sink.len() }
}
```

关键点：`OutputStream` 必须在结构体中保持存活（用 `_stream` 命名抑制 unused warning），否则音频输出会立即中断。

**验收条件**：
- [ ] `cargo build` 零 warning
- [ ] 无需单独测试（在 S1.3 中集成测试）

---

#### Session 1.3 — 串联解码与输出（AudioEngine）

| 属性 | 值 |
|------|-----|
| **依赖** | S1.1 + S1.2 完成 |
| **输入** | `src/audio/engine.rs`, `src/audio/decoder.rs`, `src/audio/output.rs` |
| **输出** | `src/audio/engine.rs` |

**任务描述**：

实现 `src/audio/engine.rs`，将解码器和输出串联为完整的播放引擎：

```rust
use std::sync::{Arc, Mutex};

pub struct AudioEngine {
    output: AudioOutput,
    decoder: Option<AudioDecoder>,
    total_frames: Arc<Mutex<u64>>,       // 已消费的采样帧数（用于计算播放位置）
    sample_rate: u32,
}

impl AudioEngine {
    pub fn new() -> crate::error::AppResult<Self> { ... }

    /// 加载并播放一个音频文件（全量解码 + 送入 Sink）
    pub fn play_file(&mut self, path: &std::path::Path) -> crate::error::AppResult<()> { ... }

    pub fn pause(&self) { ... }
    pub fn resume(&self) { ... }
    pub fn stop(&mut self) { ... }
    pub fn set_volume(&self, vol: f32) { ... }
    pub fn is_playing(&self) -> bool { ... }

    /// 当前播放位置（秒）
    pub fn position_secs(&self) -> f64 {
        let frames = *self.total_frames.lock().unwrap();
        frames as f64 / self.sample_rate as f64
    }

    /// 当前曲目总时长（秒）
    pub fn duration_secs(&self) -> Option<f64> { ... }
}
```

`play_file()` 实现要点：
1. 创建 `AudioDecoder::open(path)`
2. 循环调用 `decoder.read_packet()`，每次得到的 `Vec<f32>` 通过 `output.play_raw()` 送入 Sink
3. 累计 `total_frames += samples.len() / channels`
4. 注意：`play_raw` 后 sink 在后台播放，不需要阻塞等待

**测试**：写 `#[test] fn test_engine_lifecycle()` — 播放 fixtures/test.wav → 断言 is_playing → pause → 断言 is_paused → resume → stop → 断言 !is_playing

**验收条件**：
- [ ] 测试通过
- [ ] `cargo build` 零 warning

---

#### Session 1.4 — TUI 骨架 + 事件循环 + 可跑的 MVP

| 属性 | 值 |
|------|-----|
| **依赖** | S1.3 完成 |
| **输入** | `src/main.rs`, `src/app.rs`, `src/event.rs`, `src/ui/mod.rs`, `src/cli.rs`, `src/input/handler.rs`, `src/config.rs` |
| **输出** | 完整可运行 MVP |

**任务描述**：

这是 Phase 1 的集成 Session，把解码、输出、UI 全部串起来。需要创建/修改以下文件：

**a) `src/config.rs`** — 最小化 Config：
```rust
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Config {
    #[serde(default = "default_volume")]
    pub default_volume: f32,
    #[serde(default = "default_seek_step")]
    pub seek_step_small_secs: u32,
}
fn default_volume() -> f32 { 0.8 }
fn default_seek_step() -> u32 { 5 }
impl Config {
    pub fn load_or_default() -> Self { /* XDG 路径查找，不存在则用默认值 */ }
}
```

**b) `src/cli.rs`** — 命令行参数：
```rust
use clap::{Parser, Subcommand};
#[derive(Parser)]
#[command(name = "termusic")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}
#[derive(Subcommand)]
pub enum Command {
    Play { file: std::path::PathBuf },
}
```

**c) `src/event.rs`** — 事件类型：
```rust
pub enum AppEvent {
    Key(crossterm::event::KeyEvent),
    Tick,
    Quit,
}
```

**d) `src/ui/mod.rs`** — 最简渲染（用 ratatui 渲染两行：标题 + 进度条）

**e) `src/input/handler.rs`** — 键盘映射（Space→暂停, q→退出, -→音量-5%, =→音量+5%）

**f) `src/app.rs`** — 主事件循环（见文档第 6 节架构）：
```rust
pub struct App {
    pub state: AppState,
    engine: AudioEngine,
}
impl App {
    pub async fn run(&mut self, cli: Cli) -> AppResult<()> {
        // 1. 启用 crossterm raw mode + alternate screen
        // 2. 如果 cli.command == Play { file }，调用 engine.play_file(&file)
        // 3. 事件循环：tokio::select! {
        //      Some(event) = crossterm_event_stream.next() => handle_input(event),
        //      _ = tokio::time::interval(100ms).tick() => handle_tick(),
        //    }
        // 4. 每次事件后 terminal.draw(|f| ui::render(f, &self.state))
        // 5. Quit 时恢复终端
    }
}
```

**g) `src/main.rs`** — 入口：
```rust
#[tokio::main]
async fn main() -> AppResult<()> {
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();
    let cli = Cli::parse();
    let config = Config::load_or_default()?;
    let mut app = App::new(config)?;
    app.run(cli).await
}
```

**h) `src/audio/mod.rs`** — 确保 `pub mod decoder; pub mod output; pub mod engine;`

**验收条件**：
- [ ] `cargo run -- play tests/fixtures/test.wav` → 听到 440Hz 正弦波
- [ ] 终端显示 `▶ test.wav  00:01 / 00:02` 和实时进度条
- [ ] Space 暂停/恢复、q 退出、- = 调节音量
- [ ] `cargo test && cargo clippy` 全通过

---

### Phase 2：播放列表 + 元数据（6 Sessions）

**目标**：浏览目录、播放列表管理、Vim 风格键位、完整元数据显示

---

#### Session 2.1 — 元数据读取（lofty 集成）

| 属性 | 值 |
|------|-----|
| **依赖** | Phase 1 完成 |
| **可并行** | ✅ 与 S2.2, S2.3 并行 |
| **输入** | `src/metadata/reader.rs` |
| **输出** | `src/metadata/reader.rs` |

**任务描述**：

实现 `pub fn read_metadata(path: &Path) -> AppResult<TrackInfo>`，使用 `lofty::read_from_path` 提取所有标签字段。`TrackInfo` 结构体定义见文档第 14 节附录 A。

关键处理逻辑：
- `lofty::read_from_path` 返回 `TaggedFile`，从中提取 `tag.primary_tag()` 或 `tag.first_tag()`
- 将 lofty 的各字段映射到 `TrackInfo`（注意 `Option` 处理）
- 时长优先从 `properties.duration()` 获取（更准确）
- Fallback：title 为空 → `path.file_stem().and_then(|s| s.to_str()).unwrap_or("Unknown")`；artist 为空 → `"Unknown Artist".to_string()`

**测试**：准备两个文件 — 一个标签完整的 FLAC 和一个故意清空标签的 WAV。验证 FLAC 字段全部正确填充、WAV 触发 fallback。

**验收条件**：
- [ ] `cargo test` 通过
- [ ] 元数据测试覆盖 normal + fallback 路径

---

#### Session 2.2 — 播放列表数据结构

| 属性 | 值 |
|------|-----|
| **依赖** | Phase 1 完成 |
| **可并行** | ✅ 与 S2.1, S2.3 并行 |
| **输入** | `src/playlist.rs`（新文件） |
| **输出** | `src/playlist.rs` |

**任务描述**：

创建 `src/playlist.rs`，定义播放列表数据结构和操作：

```rust
pub struct TrackEntry {
    pub path: std::path::PathBuf,
    pub title: String,
    pub artist: String,
    pub duration_secs: f64,
}

pub struct Playlist {
    pub name: String,
    pub tracks: Vec<TrackEntry>,
    pub current_index: Option<usize>,
}

impl Playlist {
    pub fn new(name: &str) -> Self { ... }

    /// 追加曲目到列表末尾
    pub fn push(&mut self, entry: TrackEntry) { ... }

    /// 插队到当前曲目之后
    pub fn insert_after_current(&mut self, entry: TrackEntry) { ... }

    /// 移除指定索引的曲目，自动修正 current_index
    pub fn remove(&mut self, index: usize) { ... }

    /// 移动到下一首，返回新索引（或 None 表示列表结束）
    pub fn next(&mut self) -> Option<usize> { ... }

    /// 移动到上一首
    pub fn prev(&mut self) -> Option<usize> { ... }

    /// Fisher-Yates 随机打乱，保持当前曲目在新位置
    pub fn shuffle(&mut self) { ... }

    /// 返回所有曲目标题（用于 UI 列表渲染）
    pub fn titles(&self) -> Vec<&str> { ... }

    /// 对 tracks 进行稳定排序
    pub fn sort_by(&mut self, key: SortKey) { ... }
}

pub enum SortKey { Title, Artist, Album, Duration }
```

**测试**：覆盖 add/remove/next/prev/shuffle 的边界行为（空列表、单曲、多曲、首尾越界）

**验收条件**：
- [ ] `cargo test` 通过（播放列表测试）
- [ ] 边界条件全部有断言

---

#### Session 2.3 — 目录扫描器

| 属性 | 值 |
|------|-----|
| **依赖** | Phase 1 完成 |
| **可并行** | ✅ 与 S2.1, S2.2 并行 |
| **输入** | `src/library/scanner.rs` |
| **输出** | `src/library/scanner.rs` |

**任务描述**：

在 `Cargo.toml` 添加 `walkdir = "2"`，然后实现：

```rust
/// 递归扫描目录，返回所有支持的音频文件路径
pub fn scan_directory(dir: &std::path::Path, extensions: &[String]) -> Vec<std::path::PathBuf> {
    walkdir::WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path().extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| extensions.iter().any(|allowed| allowed == ext.to_lowercase()))
                .unwrap_or(false)
        })
        .map(|e| e.path().to_path_buf())
        .collect()
}
```

**测试**：创建临时目录 → 放入 .flac + .txt → 验证只返回 .flac

**验收条件**：
- [ ] 测试通过

---

#### Session 2.4 — 完整 Player View UI

| 属性 | 值 |
|------|-----|
| **依赖** | S2.1 + S2.2 + S2.3 完成 |
| **输入** | `src/ui/views/player_view.rs`, `src/ui/widgets/` |
| **输出** | Player View + 信息栏 + 进度条 + 状态栏 |

**任务描述**：

实现文档第 4.1 节的完整 Player View 布局：

```
顶部信息栏（1行）：▶ {title} — {artist}    {position} / {duration}
进度条（1行）    ：████████░░░░░░░░  Vol: {vol}%  {repeat_icon} {shuffle_icon}
分隔线
播放列表区域（剩余行）：List widget，当前曲目左侧有 ▶ 标记
分隔线
底部状态栏（1行）：Playlist: {name} | {n} tracks | {total_duration} | {repeat} {shuffle}
```

使用 ratatui 的 `Layout::Vertical` + `Constraint` 分配空间。播放列表用 `List` widget，让出足够空间给后续 Phase 的频谱和歌词面板。

**验收条件**：
- [ ] `cargo run` 进入交互模式 → 显示完整布局
- [ ] 添加几首歌后列表正确显示，当前播放曲目有 ▶ 标记
- [ ] `cargo clippy` 零 warning

---

#### Session 2.5 — Vim 风格键盘导航

| 属性 | 值 |
|------|-----|
| **依赖** | S2.4 完成 |
| **可并行** | 否（依赖 UI 布局确定后的焦点模型） |
| **输入** | `src/input/keymap.rs`, `src/input/handler.rs` |
| **输出** | 完整 Normal Mode 键位 |

**任务描述**：

实现 Normal Mode 键盘处理：

1. **移动**：`j`/`k` → 移动播放列表选中索引；`gg` → index 0（需实现双键序列检测：200ms 时间窗口）；`G` → index = len-1；`Ctrl+d`/`Ctrl+u` → 翻半页

2. **播放**：`Enter` → 播放选中项（调用 `engine.play_file(entry.path)`；先通过 `read_metadata` 构建 `TrackEntry`）；`Space` → 暂停/恢复；`n`/`p` → 下一首/上一首

3. **模式**：`r` → 循环切换 `RepeatMode::Off → Track → Playlist → Off`；`R` → toggle shuffle

4. **删除**：`dd` → 从播放列表移除当前项（双键序列，与 `gg` 同理）

5. **`/` → 进入搜索模式**（见 S2.6）

双键序列实现：
```rust
struct KeySequence {
    last_key: Option<(crossterm::event::KeyEvent, std::time::Instant)>,
    timeout_ms: u64,
}
// 在 handler 中：收到第一个 g 后记录时间戳，200ms 内收到第二个 g → 执行 gg 动作
// 超时或收到其他键 → 清除状态，将第一个 g 作为未识别键忽略
```

**验收条件**：
- [ ] `cargo run` → 所有 Vim 键位按预期工作
- [ ] `gg` / `G` / `dd` 双键序列正常工作
- [ ] 翻页不影响选中状态
- [ ] 边界测试：空列表不崩、最后一项 next 正确处理

---

#### Session 2.6 — 搜索过滤 + Config 加载 + 播放模式

| 属性 | 值 |
|------|-----|
| **依赖** | S2.5 完成 |
| **输入** | `src/ui/widgets/search_bar.rs`, `src/config.rs`（已有）, `src/app.rs` |
| **输出** | 搜索功能 + Config 完整加载 + Repeat/Shuffle 逻辑 |

**任务描述**：

**a) 搜索过滤**：

`/` 进入 Insert 模式，底部出现搜索输入框。`InputHandler` 在 Insert 模式下将按键作为字符累积到 `AppState.search_query`。UI 渲染时过滤播放列表：
```rust
fn matches_query(entry: &TrackEntry, query: &str) -> bool {
    let q = query.to_lowercase();
    entry.title.to_lowercase().contains(&q)
        || entry.artist.to_lowercase().contains(&q)
}
```
`Esc` 清除查询并回到 Normal 模式。

**b) Config 完整加载**：

扩展现有的 `Config`，支持文档 3.6 节中所有字段（`library`, `playback`, `visualizer`, `lyrics`, `ui` 段）。配置文件不存在时使用硬编码默认值。

**c) 播放模式**：

在事件循环的 `Tick` 处理中检测 `engine.is_playing()` 变为 false 且 `sink.empty()` → 触发 `TrackEnded` 逻辑：
- `RepeatMode::Track` → `engine.seek_to(0)` + `engine.resume()`
- `RepeatMode::Playlist` → `playlist.next()` → 加载下一首
- `RepeatMode::Off` → 不做任何事，停在那里

Shuffle 模式：维护 `shuffled_indices: Vec<usize>`，next/prev 在这个列表上移动。

**验收条件**：
- [ ] `/` 搜索实时过滤，大小写不敏感
- [ ] `Esc` 清除后恢复完整列表
- [ ] 单曲循环 / 列表循环 / 随机播放全部正确
- [ ] `cargo test` 通过（含 Config 解析测试）

---

### Phase 3：歌词系统（4 Sessions）

---

#### Session 3.1 — LRC 解析器（纯逻辑，无 UI）

| 属性 | 值 |
|------|-----|
| **依赖** | Phase 2 完成 |
| **可并行** | ✅ 与 S3.2 并行 |
| **输入** | `src/lyrics/types.rs`, `src/lyrics/parser.rs` |
| **输出** | 完整的 LRC 解析 + 编码检测 |

**任务描述**：

添加依赖 `regex = "1"`, `encoding_rs = "0.8"`, `unicode-width = "0.2"` 到 `Cargo.toml`。

**a) `src/lyrics/types.rs`** — 定义 `LyricLine`, `LyricTrack`, `LyricMetadata`（见文档 3.3 节数据结构）

**b) `src/lyrics/parser.rs`** — 实现 `pub fn parse_lrc(content: &str) -> AppResult<LyricTrack>`

核心解析逻辑：
1. 逐行遍历，对每一行：
   - 用正则 `\[(\d{2}):(\d{2})\.(\d{2,3})\]` 提取所有时间标签
   - 用正则 `\[(ti|ar|al|by|offset|length):(.*?)\]` 提取元数据标签
   - 剩余文本作为歌词内容
   - 如果内容中有 `<mm:ss.cc>` 标签 → 解析为逐字时间戳 `word_timestamps`
2. 所有时间标签行收集完毕后，按 timestamp 排序
3. `[offset:...]` 的值存入 `LyricMetadata.global_offset_ms`

**编码回退**（在 parser 中或独立的 `decode_with_fallback` 函数）：
```rust
fn decode_with_fallback(bytes: &[u8], fallbacks: &[&str]) -> AppResult<String> {
    // 1. 先检测 BOM
    if let Some((encoding, _)) = encoding_rs::Encoding::for_bom(bytes) {
        return Ok(encoding.decode(bytes).0.into_owned());
    }
    // 2. 尝试 UTF-8
    if let Ok(s) = std::str::from_utf8(bytes) { return Ok(s.to_string()); }
    // 3. 按 fallbacks 列表依次尝试
    for label in fallbacks {
        if let Some(encoding) = encoding_rs::Encoding::for_label(label.as_bytes()) {
            let (decoded, _, had_errors) = encoding.decode(bytes);
            return Ok(decoded.into_owned());  // 接受有替换字符的结果
        }
    }
    Err(AppError::Lyrics("All encodings failed".into()).into())
}
```

**测试**（重点！）：
1. 标准 LRC：`[00:12.34]Hello\n[00:15.67]World` → 验证 2 行、时间戳正确
2. 元数据：`[ti:Song]\n[offset:+500]` → 验证 metadata 字段和 offset
3. 多时间标签（重复段落）：同一行对应多个 `[mm:ss.cc]` → 验证生成多个 `LyricLine`
4. 逐字标签：`[00:18.90]<00:18.90>A<00:19.20>B` → 验证 `word_timestamps`
5. 空文件 → 返回空 `LyricTrack`
6. 损坏行（乱码）→ 不应 panic，跳过坏行
7. 用真实 LRC 文件（`tests/fixtures/test.lrc`）验证完整流程

**验收条件**：
- [ ] `cargo test` 全部通过（至少 7 个测试用例）
- [ ] GBK 编码的 LRC 测试文件正确解码

---

#### Session 3.2 — 歌词查找引擎 + 同步显示

| 属性 | 值 |
|------|-----|
| **依赖** | Phase 2 完成 |
| **可并行** | ✅ 与 S3.1 并行 |
| **输入** | `src/lyrics/engine.rs`, `src/ui/widgets/lyrics_panel.rs` |
| **输出** | 歌词查找 + 同步渲染组件 |

**任务描述**：

**a) `src/lyrics/engine.rs`**：

```rust
pub struct LyricEngine;

impl LyricEngine {
    /// 按优先级查找歌词文件
    pub fn find_lyrics(audio_path: &Path, config: &LyricsConfig) -> Option<PathBuf> { ... }

    /// 加载并解析歌词（含编码检测）
    pub fn load(audio_path: &Path, config: &LyricsConfig) -> AppResult<Option<LyricTrack>> { ... }

    /// 根据当前播放位置查找应高亮的歌词行索引
    /// 使用二分查找，从 hint_index 开始（因为时间单调递增）
    pub fn sync(track: &LyricTrack, position_secs: f64, hint_index: usize) -> usize { ... }
}
```

查找优先级：同名 .lrc → 内嵌歌词（lofty `USLT`/`SYLT` 帧）→ 统一歌词目录

**b) `src/ui/widgets/lyrics_panel.rs`**：

```rust
pub fn render_lyrics(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    track: &LyricTrack,
    current_line_index: usize,
    theme: &LyricsTheme,
    config: &LyricsConfig,
) { ... }
```

渲染效果：
- 当前行：bold + `theme.lyrics.current_line` 前景色
- 前 N 行：`theme.lyrics.past_line`（逐渐变暗）
- 后 N 行：`theme.lyrics.future_line`（淡色）
- 如果有 `word_timestamps`，用逐字高亮
- 视口滚动确保当前行在中间偏上

**验收条件**：
- [ ] 测试：有同名 .lrc 文件 → `find_lyrics()` 返回 Some
- [ ] 测试：无歌词文件 → `load()` 返回 Ok(None)
- [ ] 渲染可视化验证：`cargo run` 播放带 LRC 的音频，歌词随进度滚动

---

#### Session 3.3 — 歌词偏移微调 + 全屏歌词视图

| 属性 | 值 |
|------|-----|
| **依赖** | S3.2 完成 |
| **输入** | `src/input/handler.rs`（扩展）, `src/ui/views/lyrics_view.rs` |
| **输出** | 偏移微调 + 全屏 KTV 视图 |

**任务描述**：

**a)** 在输入处理中添加歌词偏移按键（`[` / `]` / `{` / `}` / `Ctrl+r`），修改 `AppState.player.lyrics_offset_ms`

**b)** 实现 `src/ui/views/lyrics_view.rs`：全屏居中布局，当前行使用较大视觉空间（上下留白，前后行缩小/淡化），KTV 风格

**验收条件**：
- [ ] `[` 提前 500ms、`]` 延后 500ms 立即生效
- [ ] `Ctrl+r` 重置偏移
- [ ] 按 `3` 切换到全屏歌词视图
- [ ] 无歌词时显示 "No lyrics found"

---

#### Session 3.4 — 歌词集成到 Player View

| 属性 | 值 |
|------|-----|
| **依赖** | S3.3 完成 |
| **输入** | `src/ui/views/player_view.rs`（修改）, `src/app.rs`（修改） |
| **输出** | 歌词在 Player View 中正常显示 |

**任务描述**：

1. 修改 Player View 布局，在进度条下方分配 5–6 行给歌词面板
2. 在 `App::run()` 的 Tick 处理中调用 `LyricEngine::sync()` 更新 `current_lyric_index`
3. 在切歌时自动调用 `LyricEngine::load()` 加载新歌词

**验收条件**：
- [ ] Player View 中看到歌词随播放进度自动滚动
- [ ] 切歌时歌词自动切换

---

### Phase 4：频谱可视化（4 Sessions）

> Phase 4 和 Phase 3 完全独立，可并行开发。

---

#### Session 4.1 — InstrumentedSource + 环形缓冲区

| 属性 | 值 |
|------|-----|
| **依赖** | Phase 2 完成 |
| **输入** | `src/audio/engine.rs`（修改） |
| **输出** | PCM 采样数据可被外部读取 |

**任务描述**：

实现一个 `InstrumentedSource<I: rodio::Source<Item = f32>>` 包装器：
```rust
pub struct InstrumentedSource<I> {
    inner: I,
    buffer: Arc<std::sync::RwLock<VecDeque<f32>>>,
}

impl<I: rodio::Source<Item = f32>> rodio::Source for InstrumentedSource<I> { ... }
impl<I: rodio::Source<Item = f32>> Iterator for InstrumentedSource<I> { ... }
```

在 `next()` 中：从 inner 取采样 → push 到共享 buffer → 返回采样值。

修改 `AudioEngine::play_file()`，在解码后的 samples 送入 Sink 之前包装为 `InstrumentedSource`。

环形缓冲区存储在 `AudioEngine` 中：`pub pcm_buffer: Arc<RwLock<VecDeque<f32>>>`，容量上限 = `2048 * 4`（8192），满时 pop_front 腾空间。

**验收条件**：
- [ ] 播放音频时 buffer 持续有数据写入
- [ ] `cargo test` 通过（检查 buffer 非空）

---

#### Session 4.2 — FFT 计算 + 频谱后处理（纯逻辑，无 UI）

| 属性 | 值 |
|------|-----|
| **依赖** | Phase 1 完成（不依赖 P2/P3） |
| **可并行** | ✅ 与 S4.1 并行，与 Phase 3 完全独立 |
| **输入** | `src/visualizer/fft.rs`, `src/visualizer/processor.rs` |
| **输出** | FFT 分析器 + 频谱处理器（纯算法，独立可测） |

**任务描述**：

添加依赖 `rustfft = "6"` 到 `Cargo.toml`。

**a) `src/visualizer/fft.rs`**：

```rust
use rustfft::{FftPlanner, num_complex::Complex};

pub struct FftAnalyzer {
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
    scratch: Vec<Complex<f32>>,
    window: Vec<f32>,
    pub size: usize,
}

impl FftAnalyzer {
    pub fn new(fft_size: usize) -> Self {
        // 预计算 Hann 窗口系数：w[i] = 0.5 * (1 - cos(2π * i / (N-1)))
        // 预分配 FftPlanner 的 forward FFT
    }

    pub fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        // 1. 将 samples 拷贝到 scratch（as Complex<f32>），同时乘以 window
        // 2. self.fft.process(&mut self.scratch)
        // 3. 计算每个 bin 的幅度：sqrt(real² + imag²)
        // 4. 只返回前半部分（Nyquist 频率以下）
    }
}
```

**b) `src/visualizer/processor.rs`**：

```rust
pub struct SpectrumProcessor {
    num_bars: usize,
    smoothing: Vec<f32>,           // EMA 状态
    peak_window: VecDeque<f32>,    // 滑动窗口历史最大值
    alpha: f32,
}

impl SpectrumProcessor {
    pub fn new(num_bars: usize, alpha: f32) -> Self { ... }

    pub fn process(&mut self, magnitudes: &[f32], sample_rate: u32) -> Vec<f32> {
        // 1. 对数分桶：将 freq_bins 映射到 num_bars 个桶
        //    start_freq = 20.0, end_freq = 16000.0
        //    每个桶边界 = start_freq * (end_freq / start_freq)^(i / num_bars)
        // 2. 每个桶取 max(magnitudes) 或 RMS
        // 3. EMA 平滑：smooth[i] = alpha * raw[i] + (1-alpha) * smooth[i]
        // 4. 动态归一化（见文档 10.3 节）
        //    max_val = max(bars)
        //    peak = EMA(historical_peak, 0.1)
        //    bars = bars / max(max_val, peak * 0.3)
        //    bars = clamp(bars, 0.0, 1.0)
    }
}
```

**测试**：
- `#[test] fn test_fft_440hz()`：生成 `sin(2π * 440 * t / 44100)` 的 2048 个采样 → FFT → 验证 index ≈ 440 * 2048 / 44100 ≈ 20 处幅度最大
- `#[test] fn test_log_buckets()`：模拟 FFT 输出 → 验证分桶数量 = num_bars
- `#[test] fn test_smoothing()`：连续输入相同值 → 验证平滑后趋于稳定

**验收条件**：
- [ ] 所有测试通过
- [ ] FFT 440Hz 测试的峰值误差在 ±2 bin 以内

---

#### Session 4.3 — 频谱字符渲染 + UI 面板

| 属性 | 值 |
|------|-----|
| **依赖** | S4.2 完成 |
| **输入** | `src/visualizer/render.rs`, `src/ui/widgets/visualizer_panel.rs` |
| **输出** | 频谱字符渲染 + UI 组件 |

**任务描述**：

**a) `src/visualizer/render.rs`**：

```rust
pub enum CharSet { Blocks, Braille, Ascii }

pub fn render_bars(
    bars: &[f32],           // 0.0..1.0 的归一化幅度
    width: u16,             // 可用字符宽度
    height: u16,            // 可用行数
    char_set: &CharSet,
    colors: &[ratatui::style::Color],  // 渐变颜色映射
) -> Vec<(String, ratatui::style::Color)> {
    // 每个 bar 对应 1-2 列、height 行
    // 值 0.0..1.0 映射到 0..7 → 选择 ▁▂▃▄▅▆▇█ + 对应行的颜色
}
```

Block Elements 映射：`const BLOCKS: &[char] = &[' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];`

颜色映射：根据 bar 索引在 `bar_low`, `bar_mid`, `bar_high` 之间线性插值（R, G, B 各自 lerp）。

**b) `src/ui/widgets/visualizer_panel.rs`**：

```rust
pub fn render_visualizer(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    bars: &[f32],
    theme: &VisualizerTheme,
    char_set: &CharSet,
) { ... }
```

将 rendered bars 渲染到 `area` 内（`Paragraph` + 逐行拼接）。

**验收条件**：
- [ ] 单元测试：`render_bars()` 输入 [0.0, 0.5, 1.0] 输出 3 列 bar，包含 ' ', '▄', '█'
- [ ] 颜色在低频（绿）和高频（红）之间有视觉差异

---

#### Session 4.4 — 启动 FFT 线程 + 集成到 Player View

| 属性 | 值 |
|------|-----|
| **依赖** | S4.1 + S4.3 完成 |
| **输入** | `src/app.rs`（修改）, `src/ui/views/player_view.rs`（修改） |
| **输出** | 播放时看到频谱跳动 |

**任务描述**：

1. 在 `AudioEngine` 中添加 `pub fn start_fft_loop(&self, tx: tokio::sync::mpsc::UnboundedSender<AppEvent>, config: &VisualizerConfig)` 方法，内部 `tokio::task::spawn_blocking` 启动 FFT 线程：
   ```
   loop {
       从 pcm_buffer 取 FFT_SIZE 个采样
       → fft_analyzer.process()
       → spectrum_processor.process()
       → tx.send(AppEvent::VisualizerData(bars))
       → sleep(1000 / config.frame_rate ms)
   }
   ```

2. 在 `App::run()` 中接收 `VisualizerData` 事件 → 更新 `AppState.visualizer_data`

3. 修改 Player View：将布局调整为「顶部信息栏 + 进度条 + 频谱区（5行）+ 歌词区（5行）+ 播放列表（剩余）+ 状态栏」

4. 全屏频谱视图（按 `4`）：`src/ui/views/visualizer_view.rs`

5. 暂停时频谱逐渐衰减：在 `processor.rs` 中，当 `process()` 收到全零或极低的 bar 值时，smoothing 让值自然衰减

**验收条件**：
- [ ] 播放音乐时频谱柱随节奏跳动
- [ ] 颜色从低频→高频渐变
- [ ] 暂停时频谱逐渐趋于平坦
- [ ] 全屏频谱模式正常
- [ ] `cargo run --release` 下 CPU < 10%

---

### Phase 5：曲库索引 + 高级 UI（7 Sessions）

---

#### Session 5.1 — SQLite 曲库索引

| 属性 | 值 |
|------|-----|
| **依赖** | Phase 2 完成 |
| **输入** | `src/library/database.rs` |
| **输出** | 完整的曲库索引存储层 |

**任务描述**：

添加 `rusqlite = { version = "0.32", features = ["bundled"] }` 到 `Cargo.toml`。

```rust
pub struct LibraryDb {
    conn: rusqlite::Connection,
}

impl LibraryDb {
    pub fn open(path: &std::path::Path) -> AppResult<Self> {
        // 打开/创建数据库，执行 CREATE TABLE IF NOT EXISTS (schema 见文档 3.5 节)
    }

    pub fn upsert(&self, track: &TrackInfo, file_mtime: u64) -> AppResult<u64> { ... }
    pub fn get_by_path(&self, path: &str) -> AppResult<Option<TrackInfo>> { ... }
    pub fn query_all(&self) -> AppResult<Vec<TrackInfo>> { ... }

    pub fn search(&self, query: &str) -> AppResult<Vec<TrackInfo>> {
        // SELECT * FROM tracks WHERE title LIKE '%q%' OR artist LIKE '%q%' OR album LIKE '%q%'
    }

    pub fn get_artists(&self) -> AppResult<Vec<String>> { ... }
    pub fn get_albums_by_artist(&self, artist: &str) -> AppResult<Vec<String>> { ... }
    pub fn get_tracks_by_album(&self, artist: &str, album: &str) -> AppResult<Vec<TrackInfo>> { ... }
}
```

**测试**：使用 `Connection::open_in_memory()` 做 CRUD 测试：
- insert → query → 字段一致
- upsert 重复 path → 更新而非插入
- search → 匹配 title/artist/album

**验收条件**：
- [ ] 全部 CRUD 测试通过

---

#### Session 5.2 — 增量扫描引擎

| 属性 | 值 |
|------|-----|
| **依赖** | S5.1 + S2.3 完成 |
| **可并行** | ✅ 与 S5.3, S5.4 并行 |
| **输入** | `src/library/scanner.rs`（重写） |
| **输出** | 增量扫描 + 进度反馈 |

**任务描述**：

重写 `src/library/scanner.rs`，实现文档 3.5 节的完整增量扫描算法：

```rust
pub enum ScanEvent {
    Progress { found: usize, total: usize },
    NewTrack { path: String, title: String },
    Done { added: usize, updated: usize, removed: usize },
}

/// 增量扫描：比较 mtime，新/改 → upsert，删除不存在 → DELETE
pub async fn scan_library(
    music_dirs: &[PathBuf],
    extensions: &[String],
    db: &LibraryDb,
    progress_tx: tokio::sync::mpsc::UnboundedSender<ScanEvent>,
) -> AppResult<()> { ... }
```

在 `tokio::task::spawn_blocking` 中执行扫描（walkdir 是阻塞的），通过 channel 发进度事件。

**验收条件**：
- [ ] 首次扫描 → 所有文件入库
- [ ] 再次扫描（文件无变化）→ 0 新增/更新
- [ ] 修改文件 mtime → 再次扫描 → 1 更新
- [ ] 删除文件 → 再次扫描 → 1 移除

---

#### Session 5.3 — 三栏 Library 浏览器

| 属性 | 值 |
|------|-----|
| **依赖** | S5.1 完成 |
| **可并行** | ✅ 与 S5.2, S5.4 并行 |
| **输入** | `src/ui/views/library_view.rs` |
| **输出** | 三栏 Library 视图 |

**任务描述**：

实现 `LibraryView`：
```
┌──────────┬──────────────┬──────────────────────────┐
│ Artists  │ Albums       │ Tracks                   │
│ (20%)    │ (25%)        │ (55%)                    │
└──────────┴──────────────┴──────────────────────────┘
```

交互逻辑：
- 默认焦点在 Artists 栏
- `j`/`k` 在焦点栏移动，`h`/`l` 切换焦点栏
- Artists 选中 → 查询并更新 Albums 栏
- Albums 选中 → 查询并更新 Tracks 栏
- Tracks 选中 + `Enter` → 构建临时播放列表并开始播放
- `a` → 将选中专辑/曲目追加到当前播放列表

**验收条件**：
- [ ] 三栏联动正确（选 Artist → Album 变，选 Album → Tracks 变）
- [ ] `h`/`l` 切换焦点顺畅
- [ ] 空曲库不 panic

---

#### Session 5.4 — M3U 导入/导出

| 属性 | 值 |
|------|-----|
| **依赖** | S2.2 完成 |
| **可并行** | ✅ 与 S5.2, S5.3 并行 |
| **输入** | `src/library/playlist_manager.rs` |
| **输出** | M3U/M3U8 导入/导出 |

**任务描述**：

```rust
/// 从 M3U/M3U8 文件导入，返回 Playlist
pub fn import_m3u(path: &Path) -> AppResult<Playlist> {
    // 逐行解析：
    // - #EXTM3U → 标记为扩展格式（同时兼容两种）
    // - #EXTINF:seconds,display_title → 记录时长和标题
    // - 非注释行的路径 → 创建 TrackEntry
    // - 相对路径 → 相对于 M3U 文件所在目录解析
}

/// 导出为扩展 M3U 格式
pub fn export_m3u(playlist: &Playlist, path: &Path) -> AppResult<()> {
    // 写入 #EXTM3U\n#EXTINF:{duration},{artist} - {title}\n{path}\n...
}
```

**测试**：创建含绝对路径和相对路径的 M3U → import → export → 验证导出版本与原始版本逻辑等价

**验收条件**：
- [ ] 导入/导出与 VLC、foobar2000 生成的 M3U 兼容
- [ ] 相对路径正确解析

---

#### Session 5.5 — 主题系统 + 命令模式

| 属性 | 值 |
|------|-----|
| **依赖** | Phase 2 完成 |
| **输入** | `src/ui/theme.rs`, `src/input/command.rs`, `themes/*.toml` |
| **输出** | 主题切换 + Command Mode |

**任务描述**：

**a) `src/ui/theme.rs`**：定义 `Theme` 结构体，实现内嵌 5 套预设主题。`Theme::load(name)` 优先从文件加载，不存在则从内嵌预设匹配。

**b) `src/input/command.rs`**：`:` 进入 Command Mode → 底部输入栏 → `Enter` 执行 / `Esc` 取消。实现文档 5.3 节的关键命令（`:q`, `:theme <name>`, `:seek`, `:volume`, `:repeat`, `:shuffle`, `:help`）。

**c)** 创建 `themes/` 目录，放入 5 个预设 TOML 主题文件（从文档 4.3 节的设计生成）。

**验收条件**：
- [ ] `:theme dracula` 立即切换所有颜色
- [ ] 未识别的命令显示 "Unknown command: xxx"
- [ ] `:help` 显示命令列表 popup

---

#### Session 5.6 — 自定义快捷键 + 运行时状态持久化

| 属性 | 值 |
|------|-----|
| **依赖** | S5.5 完成 |
| **输入** | `src/input/keymap.rs`（扩展）, `src/app.rs`（修改） |
| **输出** | 可自定义键位 + 启动续播 |

**任务描述**：

**a)** 扩展 `KeyBindings` 支持从 `~/.config/termusic/keybindings.toml` 加载。实现键名字符串 → `crossterm::event::KeyEvent` 的解析函数（如 `"Ctrl+d"` → `KeyEvent { code: Char('d'), modifiers: KeyModifiers::CONTROL }`）

**b)** 在 `App` 中添加：
```rust
fn save_state(&self) -> AppResult<()> { ... }  // 序列化为 state.json
fn load_state(&self) -> AppResult<()> { ... }  // 从 state.json 恢复
```
保存字段：`last_track_path`, `last_position_secs`, `volume`, `repeat_mode`, `shuffle`, `lyrics_offset_ms`, `playlist_name` + `tracks`

在 `Quit` 事件时自动调用 `save_state()`；`App::new()` 时自动调用 `load_state()`（如果 `config.playback.resume_on_startup` 为 true）。

**验收条件**：
- [ ] 修改 `keybindings.toml` 后重启，自定义键位生效
- [ ] 播放中途退出再启动 → 续播相同位置
- [ ] 配置文件缺失的键位回退到默认值

---

#### Session 5.7 — 最终集成与打磨

| 属性 | 值 |
|------|-----|
| **依赖** | S5.1–S5.6 全部完成 |
| **输入** | 全项目 |
| **输出** | 完整的生产级体验 |

**任务描述**：

这是 Phase 5 的收尾 Session，不做新功能，专注集成打磨：

1. **连通性测试**：完整走一遍启动 → 扫描 → 浏览 → 播放 → 歌词 → 频谱 → 切歌 → 退出 → 续播
2. **错误处理硬化**：所有 `unwrap()` / `expect()` 替换为 `?` 或 `unwrap_or_*()`；所有 `panic!` 路径加保护
3. **边界条件测试**：
   - 空音乐目录
   - 损坏的音频文件（`play_file` 在 catch 到错误后优雅跳过）
   - 终端 resize 后布局自动适应
   - CJK 文件名和歌词对齐（用 `unicode-width` 的 `UnicodeWidthStr::width()`）
4. **内存/资源**：确认 `AudioEngine::stop()` 正确释放 decoder；确认 Sink 在 stop 时被 clear
5. **README.md**：写安装说明、使用指南、键位速查表

**验收条件**：
- [ ] 完整用户流程无 panic 无错误
- [ ] `cargo test && cargo clippy && cargo fmt --check` 全绿
- [ ] 损坏音频文件不崩溃，仅显示 "Playback error"

---

### Phase 6：扩展（9 个独立 Session，无依赖顺序）

每个 Phase 6 Session 是独立的功能扩展，可随时按需求启动。

| Session | 功能 | 依赖 | 关键 crate |
|---------|------|------|-----------|
| S6.1 | 终端封面图显示 | P5 | `viuer` |
| S6.2 | 桌面通知 | P1 | `notify-rust` |
| S6.3 | MPRIS2 D-Bus | P1 | `zbus` |
| S6.4 | 在线歌词搜索 | P3 | `reqwest` + `serde_json` |
| S6.5 | 10段EQ均衡器 | P1 | 手动 IIR 或 `biquad` |
| S6.6 | ReplayGain | P2 | 通过 `lofty` 读取已有标签 |
| S6.7 | Last.fm 集成 | P1 | `reqwest` |
| S6.8 | 打包分发 | P5 | `cargo-release` + GitHub Actions |
| S6.9 | macOS 适配 | P5 | 验证 cpal CoreAudio |

每个扩展 Session 的结构（模板）：
- **上下文**：在哪个模块插入代码、读取哪些现有结构体
- **新依赖**：要加的 crate
- **实现步骤**：3–5 步具体修改
- **验收方式**：手动测试还是自动测试

---

### AI 开发最佳实践

1. **一个 Session = 一个 AI 对话**：每个 Session 设计为约 100–400 行的净变更，可在一个 AI 对话中完成
2. **先测试、后实现**：每个 Session 标注了测试要求，让 AI 先写测试（TDD）
3. **编译即检查点**：每个 Session 结束时 `cargo build` 必须通过
4. **并行扩展**：标注了「可并行」的 Session 可以同时开多个 AI 对话推进（需用 `git worktree` 隔离，合并时可能有冲突需手动解决）
5. **迭代式修复**：AI 生成的代码大概率需要 1–2 轮修复（编译错误、测试失败），在估算时预留修复轮次

---

## 14. 附录：关键数据结构与项目结构

### 附录 A：关键数据结构

```rust
// ── 音频信息 ──
struct TrackInfo {
    path: PathBuf,
    title: String,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    track_number: Option<u32>,
    track_total: Option<u32>,
    disc_number: Option<u32>,
    genre: Option<String>,
    year: Option<u32>,
    duration: Duration,
    cover_art: Option<Vec<u8>>,
    // 音频流属性
    bitrate: u32,
    sample_rate: u32,
    channels: u8,
    codec: String,
}

// ── 播放状态 ──
struct PlayerState {
    current_track: Option<TrackInfo>,
    position: Duration,               // 当前播放位置
    duration: Duration,               // 当前曲目总时长
    volume: f32,                      // 0.0 – 1.0
    is_playing: bool,
    repeat_mode: RepeatMode,          // Off / Track / Playlist
    shuffle: bool,
    is_muted: bool,
    lyrics_offset_ms: i64,            // 用户手动偏移
}

enum RepeatMode { Off, Track, Playlist }

// ── 播放列表 ──
struct Playlist {
    name: String,
    tracks: Vec<TrackEntry>,
    current_index: Option<usize>,      // 当前播放位置的索引
}

struct TrackEntry {
    track_id: u64,                     // 关联 LibraryManager 中的 ID
    title: String,
    artist: String,
    duration: Duration,
}

// ── 全局应用状态 ──
struct AppState {
    player: PlayerState,
    playlist: Playlist,
    queue: Vec<TrackEntry>,             // 临时播放队列
    library: LibraryState,
    lyrics: Option<LyricTrack>,
    current_lyric_index: Option<usize>,
    visualizer_data: Vec<f32>,         // 最新频谱柱数据
    active_view: ViewMode,
    focused_panel: PanelId,
    search_query: String,
    config: Config,
    should_quit: bool,
}

// ── 事件类型 ──
enum AppEvent {
    Key(crossterm::event::KeyEvent),
    Mouse(crossterm::event::MouseEvent),
    Resize(u16, u16),
    TrackStarted(TrackInfo),
    PositionUpdated(Duration),
    TrackEnded,
    PlaybackError(String),
    VisualizerData(Vec<f32>),
    Tick,
    Quit,
}

// ── 视图模式 ──
enum ViewMode {
    Player,
    Library,
    Lyrics,
    Visualizer,
    Playlists,
    Browser,
}
```

### 附录 B：项目目录结构

```
termusic/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── DESIGN.md                          ← 本文件
├── LICENSE
├── src/
│   ├── main.rs                        # 入口：初始化 tracing、加载配置、启动 App
│   ├── app.rs                         # AppState + 主事件循环
│   ├── event.rs                       # AppEvent 枚举 + 通道类型别名
│   ├── config.rs                      # Config 结构体 + 加载/保存
│   ├── cli.rs                         # clap 命令行参数解析
│   ├── playlist.rs                    # Playlist + TrackEntry 数据结构与方法
│   │
│   ├── audio/
│   │   ├── mod.rs
│   │   ├── engine.rs                  # AudioEngine：加载、播放、暂停、seek、音量
│   │   ├── decoder.rs                 # Symphonia 适配：格式探测 → 解码器
│   │   └── output.rs                  # Rodio 输出管理：Sink 创建、多 Sink 切换
│   │
│   ├── metadata/
│   │   ├── mod.rs
│   │   └── reader.rs                  # Lofty 封装：读取所有标签字段
│   │
│   ├── lyrics/
│   │   ├── mod.rs
│   │   ├── parser.rs                  # LRC 状态机解析器
│   │   ├── engine.rs                  # 歌词查找、加载、同步
│   │   └── types.rs                   # LyricTrack, LyricLine, LyricMetadata
│   │
│   ├── visualizer/
│   │   ├── mod.rs
│   │   ├── fft.rs                     # FFT 计算（rustfft 封装）
│   │   ├── processor.rs               # 分桶、平滑、归一化
│   │   └── render.rs                  # 字符/颜色渲染
│   │
│   ├── library/
│   │   ├── mod.rs
│   │   ├── scanner.rs                 # 目录遍历 + 文件发现
│   │   ├── database.rs                # SQLite 读写（曲库索引）
│   │   └── playlist_manager.rs        # M3U 导入/导出、播放列表持久化
│   │
│   ├── ui/
│   │   ├── mod.rs                     # UI 入口：根据 active_view 选择渲染函数
│   │   ├── theme.rs                   # 主题加载 + 样式应用
│   │   ├── widgets/
│   │   │   ├── mod.rs
│   │   │   ├── progress_bar.rs        # 播放进度条
│   │   │   ├── track_list.rs          # 通用曲目列表（复用）
│   │   │   ├── lyrics_panel.rs        # 歌词显示面板
│   │   │   ├── visualizer_panel.rs    # 频谱面板
│   │   │   ├── status_bar.rs          # 底部状态栏
│   │   │   ├── file_browser.rs        # 文件系统浏览器
│   │   │   ├── search_bar.rs          # 搜索/过滤输入框
│   │   │   └── help_popup.rs          # 帮助弹窗（键位速查）
│   │   └── views/
│   │       ├── mod.rs
│   │       ├── player_view.rs         # 默认：频谱 + 歌词 + 播放列表
│   │       ├── library_view.rs        # 三栏：艺术家 / 专辑 / 曲目
│   │       ├── lyrics_view.rs         # 全屏歌词
│   │       ├── visualizer_view.rs     # 全屏频谱
│   │       └── browser_view.rs        # 文件浏览器
│   │
│   └── input/
│       ├── mod.rs
│       ├── keymap.rs                  # 默认键位定义 + 用户自定义加载
│       ├── handler.rs                 # 按键处理（模态逻辑 + 快捷键匹配）
│       └── command.rs                 # Command Mode 命令解析器
│
├── config/
│   └── default.toml                   # 默认配置模板（首次运行时复制到 ~/.config/）
│
├── themes/
│   ├── tokyo-night.toml
│   ├── dracula.toml
│   ├── nord.toml
│   ├── solarized-dark.toml
│   └── catppuccin-mocha.toml
│
└── tests/
    ├── fixtures/                      # 测试用音频文件和 LRC 文件
    │   ├── test.flac
    │   ├── test.lrc
    │   ├── test_utf8.lrc
    │   └── test_gbk.lrc
    ├── lrc_parser_test.rs
    ├── visualizer_fft_test.rs
    ├── playlist_test.rs
    └── config_test.rs
```

---

> **文档版本**: v3.0
> **最后更新**: 2026-07-11
> **变更摘要（v3.0）**：
> - **实施路线图完全重写为「AI 开发优化版」**：
>   - 将每个 Phase 拆解为自包含的 AI Session（共 35 个 Session）
>   - 每个 Session 包含 AI 所需的全部上下文：输入/输出文件、函数签名、数据结构、测试策略
>   - 标注了上下文大小（🟢🟡🔴）帮助选择合适的模型
>   - 标注了可并行开发的 Session，支持多 agent 同时推进
>   - 每个 Session 可独立编译和测试（checkpoint 式开发）
>   - 增加了全局 AI 开发约定和最佳实践
>
> **变更摘要（v2.0）**：
> - 修复了"Vue 风格键盘导航"→"Vim 风格键盘导航"的笔误
> - 修复了目录与实际章节编号不一致的问题
> - 新增 Symphonia + Rodio FFT 采样的技术要点说明
> - 添加版本号时效性警告
