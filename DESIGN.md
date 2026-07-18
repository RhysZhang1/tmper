# tmper — 项目架构与实现文档

> **项目名称**: tmper — 终端音乐播放器
> **语言**: Rust
> **平台**: Arch Linux + KDE Plasma
> **文档版本**: v3.3（实现文档）
> **最后更新**: 2026-07-18

---

## 目录

1. [项目概览](#1-项目概览)
2. [整体架构](#2-整体架构)
3. [模块详解](#3-模块详解)
4. [数据流](#4-数据流)
5. [UI 系统](#5-ui-系统)
6. [事件系统](#6-事件系统)
7. [配置系统](#7-配置系统)
8. [持久化](#8-持久化)
9. [键盘处理](#9-键盘处理)
10. [测试](#10-测试)
11. [项目结构](#11-项目结构)
12. [依赖清单](#12-依赖清单)

---

## 1. 项目概览

tmper 是一个运行在终端中的全功能音乐播放器。核心特性：

- **多格式解码**: MP3、FLAC、OGG、Opus、WAV、AAC、M4A、WMA、APE、WavPack、AIFF
- **元数据**: ID3v1/v2、Vorbis Comments、APE、MP4 标签，含内嵌封面图
- **LRC 歌词**: 标准/增强 LRC 解析，实时同步，编码自动检测
- **频谱可视化**: 2048 点 FFT，对数分桶，颜色渐变
- **曲库**: SQLite 索引，三栏浏览器，全文搜索
- **7 个视图**: 播放器、曲库、歌词、频谱、歌单管理、文件浏览器、设置
- **5 套主题**: Tokyo Night、Dracula、Nord、Solarized Dark、Catppuccin Mocha
- **Vim 风格操作**: 模态键盘，双键序列（gg、dd），/ 搜索

---

## 2. 整体架构

### 2.1 分层架构

```
┌──────────────────────────────────────────────────────┐
│  main.rs — tracing 初始化 → Config 加载 → App::run() │
└────────────────────────┬─────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────┐
│  App (src/app/) — 事件循环 + 全局状态                 │
│  ├─ mod.rs       — App 结构体 + run() 事件循环        │
│  ├─ handlers/    — 按键分发 + 播放逻辑                 │
│  │  ├─ mod.rs    — handle_event(), 全局/播放器按键     │
│  │  ├─ playlist.rs — 歌单视图按键                      │
│  │  ├─ library.rs  — 曲库视图按键                      │
│  │  ├─ browser.rs  — 文件浏览器视图按键                 │
│  │  └─ settings.rs — 设置视图按键 + write_config()     │
│  └─ persistence.rs — 状态保存/加载                    │
└────────────────────────┬─────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────┐
│  UI (src/ui/) — 渲染层（只读 &AppState）               │
│  ├─ mod.rs       — UiState, ViewMode, render()       │
│  ├─ theme.rs     — 5 套预设主题                       │
│  ├─ views/       — 7 个视图                           │
│  │  ├─ player_view.rs  — 播放器（封面+频谱+歌词+列表） │
│  │  ├─ library_view.rs — 曲库（三栏浏览+搜索）         │
│  │  ├─ lyrics_view.rs  — 全屏歌词（KTV 风格）          │
│  │  ├─ playlist_view.rs— 歌单管理器                    │
│  │  ├─ file_browser_view.rs — 文件浏览器               │
│  │  └─ settings_view.rs — 设置编辑器                   │
│  └─ widgets/     — 可复用组件                          │
│     ├─ help_popup.rs       — 帮助面板（键 0）           │
│     ├─ lyrics_panel.rs     — 播放器内歌词面板           │
│     └─ visualizer_panel.rs — 频谱渲染                   │
└────────────────────────┬─────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────┐
│  业务层                                               │
│  ├─ AudioEngine (src/audio/engine.rs)                 │
│  │  ├─ decoder.rs — Symphonia 解码适配                │
│  │  └─ output.rs  — Rodio Sink 封装                   │
│  ├─ LibraryDb (src/library/database.rs)               │
│  │  ├─ scanner.rs        — 目录扫描                   │
│  │  └─ playlist_manager.rs — M3U 导入/导出             │
│  ├─ LyricEngine (src/lyrics/engine.rs)                │
│  │  ├─ parser.rs — LRC 解析 + 编码检测                │
│  │  └─ types.rs  — LyricLine, LyricTrack              │
│  ├─ Visualizer (src/visualizer/)                       │
│  │  ├─ fft.rs       — FFT 分析（2048 点 Hann 窗）      │
│  │  ├─ processor.rs — 频谱后处理（对数分桶+平滑）       │
│  │  └─ render.rs    — 字符渲染（Block Elements）       │
│  └─ MetadataReader (src/metadata/reader.rs)           │
└──────────────────────────────────────────────────────┘
```

### 2.2 并发模型

```
┌─────────────────────────────────────────────────────┐
│                    主线程 (tokio)                     │
│                                                      │
│  tokio::select! {                                   │
│      keyboard_event = event_stream.next() => {       │
│          KeyHandler 处理 → handle_event(event)        │
│          → 修改 AppState                              │
│      }                                               │
│      _ = tick_interval.tick() => {                   │
│          更新播放位置、FFT 数据、歌词同步              │
│          检测曲目结束 → 自动切歌                       │
│      }                                               │
│  }                                                   │
│  terminal.draw(|f| ui::render(f, &app))  ← 每次事件后 │
└─────────────────────────────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────┐
│               后台线程 (spawn_blocking)                │
│                                                      │
│  FFT 线程:                                           │
│    loop {                                            │
│      从 pcm_buffer 读取 PCM 采样                      │
│      → FftAnalyzer.process()                         │
│      → SpectrumProcessor.process()                   │
│      → 通过 AppEvent::VisualizerData 发送回主线程      │
│      sleep(32ms)  // ~31 FPS                         │
│    }                                                 │
│                                                      │
│  InstrumentedSource (在 rodio 管线中):                │
│    Source::next() 被调用时 → 拷贝采样到 pcm_buffer     │
│    → FFT 线程读取                                     │
└─────────────────────────────────────────────────────┘
```

---

## 3. 模块详解

### 3.1 音频引擎 (src/audio/)

#### decoder.rs — Symphonia 解码适配

```rust
pub struct AudioDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    pub sample_rate: u32,
    pub channels: u8,
    pub total_frames: u64,
}
```

**流程**：`symphonia::default::get_probe()` 探测容器 → 创建解码器 → `read_packet()` 循环输出 `Vec<f32>` PCM。使用 `SampleBuffer` 统一转换为 f32。

**测试**：`test_decode_wav`（440Hz 正弦波，44100Hz 验证）、`test_decode_nonexistent`（错误处理）。

#### output.rs — Rodio Sink 封装

```rust
pub struct AudioOutput {
    sink: Sink,
    stream_handle: OutputStreamHandle,
    _stream: OutputStream,  // 必须保持存活
}
```

**关键实现**：`stop_and_replace()` — 解决 rodio 0.20 的 `sink.stop()` 永久断连问题。创建全新 Sink 替换旧 Sink。

#### engine.rs — 播放引擎

```rust
pub struct AudioEngine {
    output: AudioOutput,
    decoder: Option<AudioDecoder>,
    start_time: Option<Instant>,
    paused_at: Option<Duration>,
    sample_rate: u32,
    total_duration: Option<f64>,
    pcm_buffer: Arc<Mutex<VecDeque<f32>>>,
    position_tx: tokio::sync::watch::Sender<f64>,
}
```

**位置追踪**：使用壁钟时间（`Instant`），而非帧计数。避免了帧计数导致 100% 进度显示的 bug。暂停时记录 `paused_at`，恢复时补偿。

**seek**：`seek_relative(secs)` 从目标位置重新解码整个文件，送入新 Sink。

**InstrumentedSource**：包装 rodio Source，在 `next()` 中拷贝采样到共享 `pcm_buffer: Arc<Mutex<VecDeque<f32>>>`，供 FFT 线程读取。

**测试**：生命周期测试、位置追踪、停止清空位置、排队测试。

---

### 3.2 元数据读取 (src/metadata/reader.rs)

```rust
pub struct TrackInfo {
    pub path: PathBuf,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track_num: Option<u32>,
    pub track_total: Option<u32>,
    pub disc_num: Option<u32>,
    pub duration_s: f64,
    pub bitrate: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub codec: String,
    pub cover_art: Option<Vec<u8>>,
}
```

使用 `lofty::read_from_path` 读取标签。`cover_art` 从 `Tag::pictures()` 提取第一张图片。

**Fallback**：`title` → 文件名（去扩展名）、`artist` → "Unknown Artist"。

**测试**：FLAC（完整标签）、WAV（无标签，验证 fallback）、不存在的文件。

---

### 3.3 歌词系统 (src/lyrics/)

#### types.rs — 数据结构

```rust
pub struct LyricLine {
    pub timestamp: Duration,
    pub text: String,
    pub word_timestamps: Vec<(Duration, String)>,
}

pub struct LyricTrack {
    pub metadata: LyricMetadata,
    pub lines: Vec<LyricLine>,
}

pub struct LyricMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub author: Option<String>,
    pub global_offset_ms: i64,
    pub length: Option<Duration>,
}
```

#### parser.rs — LRC 解析

解析流程：
1. 逐行正则匹配：`\[(\d{2}):(\d{2})\.(\d{2,3})\]` 提取时间标签
2. `\[(ti|ar|al|by|offset|length):(.*?)\]` 提取元数据
3. `<mm:ss.cc>` 内部标签 → 逐字时间戳
4. 多时间标签行 → 为每个时间生成 LyricLine
5. 按 timestamp 排序

**编码检测**：BOM → UTF-8 → GBK → Shift-JIS（使用 `encoding_rs`）

**测试**：7 个测试覆盖标准 LRC、元数据、多时间戳、逐字时间戳、空文件、损坏行、排序。

#### engine.rs — 查找与同步

- `load()` — 按优先级查找 `.lrc` 文件：同名 .lrc → 内嵌歌词 → 统一目录
- `sync()` — 二分查找当前位置对应当前行（从 hint_index 开始搜索）

---

### 3.4 频谱可视化 (src/visualizer/)

#### fft.rs — FFT 分析

```rust
pub struct FftAnalyzer {
    fft: Arc<dyn Fft<f32>>,
    scratch: Vec<Complex<f32>>,
    window: Vec<f32>,  // Hann 窗
    pub size: usize,   // 2048
}
```

**流程**：Hann 窗 → FFT → 幅度 `sqrt(real² + imag²)` → 返回前半部分（Nyquist 以下）

**测试**：440Hz 正弦波峰值检测。

#### processor.rs — 频谱后处理

- **对数分桶**：60Hz–8kHz 分为 N 个桶（默认 64），低频桶窄、高频桶宽
- **双向平滑**：EMA 前后各一次（α=0.22）
- **动态范围归一化**：基于滑动窗口历史最大值

**测试**：桶数量验证、平滑收敛测试。

#### render.rs — 终端渲染

- 8 级 Block Elements：`▁▂▃▄▅▆▇█`
- 绿→黄→红 逐柱颜色渐变
- `CharSet::Blocks` 默认，`Braille`/`Ascii` 预留

**测试**：渲染输出验证、颜色渐变测试。

---

### 3.5 音乐库 (src/library/)

#### database.rs — SQLite 索引

```rust
pub struct LibraryDb { conn: Connection }
pub struct TrackRow { /* 21 个字段，匹配数据库列 */ }
```

**Schema**：`tracks` 表，21 列（id, path, title, artist, album, ..., added_at）。3 个索引（artist, album, genre）。

**操作**：`upsert()`（INSERT OR REPLACE）、`get_by_path()`、`search()`（LIKE 模糊搜索）、`get_artists()`、`get_albums()`、`delete_by_path()`。

**测试**：6 个测试覆盖 upsert、重复更新、搜索、get_artists、get_albums、delete。

#### scanner.rs — 目录扫描

- `scan_directory()` — 递归遍历目录，过滤扩展名，mtime 变化检测
- 支持 `follow_symlinks` 配置

**测试**：扩展名过滤验证。

#### playlist_manager.rs — M3U 导入导出

- `import_m3u()` — 解析 EXTINF 标签，相对路径解析
- `export_m3u()` — 写入扩展 M3U 格式

**测试**：往返测试、相对路径测试。

---

### 3.6 CLI (src/cli.rs)

```rust
#[derive(Parser)]
#[command(name = "tmper")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    Play { files: Vec<PathBuf> },
}
```

支持：`tmper`（交互模式）、`tmper play <path>`（播放文件/目录）。

---

### 3.7 错误处理 (src/error.rs)

```rust
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Audio: {0}")]      Audio(String),
    #[error("Metadata: {0}")]   Metadata(String),
    #[error("Config: {0}")]     Config(String),
    #[error("Lyrics: {0}")]     Lyrics(String),
    #[error("IO: {0}")]         Io(#[from] std::io::Error),
}
pub type AppResult<T> = anyhow::Result<T>;
```

---

## 4. 数据流

### 4.1 播放一首歌的完整流程

```
1. 用户按 Enter 选歌
      │
2. handle_key_event() → Enter → play_selected(path)
      │
3. AudioEngine::load_file(path):
      │
      ├─ AudioDecoder::open(path)
      │    └─ Symphonia: 探测容器 → 选择音轨 → 创建解码器
      │
      ├─ 循环 read_packet() 解码全部 PCM 采样
      │    └─ 送入 InstrumentedSource → Rodio Sink
      │         └─ InstrumentedSource::next() 拷贝采样到 pcm_buffer
      │
      └─ 记录 start_time, total_duration
      │
4. 主循环 Tick (每 ~33ms):
      │
      ├─ engine.position_secs() → 更新 UI 进度条
      ├─ 读取 pcm_buffer → 通过 FFT 线程 → VisualizerData 事件 → 更新频谱
      ├─ LyricEngine::sync(position) → 更新当前歌词行
      └─ 检测 is_playing() && sink.empty() → on_track_ended()
            └─ 根据 RepeatMode 自动切歌 / 停止
      │
5. UI 渲染 (每次事件后):
      └─ terminal.draw(|f| ui::render(f, &app))
           └─ 根据 active_view 分发到对应 view_render 函数
```

### 4.2 状态管理原则

- **单一写入者**：AppState 只在 `handle_event()` 中修改
- **只读渲染**：UI 渲染函数接收 `&AppState`，不做修改
- **通道通信**：FFT 线程通过 `tokio::sync::watch` 发送数据
- **不可变更新**：遵循"创建新值，不修改旧值"原则

---

## 5. UI 系统

### 5.1 视图系统

| 键 | 视图 | 文件 | 描述 |
|----|------|------|------|
| `1` | Player | `views/player_view.rs` | 两栏布局：封面+歌单 / 歌词+频谱+控制栏 |
| `2` | Library | `views/library_view.rs` | 三栏：艺术家→专辑→歌曲，/ 搜索 |
| `3` | Lyrics | `views/lyrics_view.rs` | 全屏 KTV 风格歌词，居中大字体 |
| `4` | Visualizer | `widgets/visualizer_panel.rs` | 全屏频谱（鱼缸模式） |
| `5` | Playlists | `views/playlist_view.rs` | 歌单管理：左侧曲库，右侧歌单 |
| `6` | Browser | `views/file_browser_view.rs` | 双栏文件管理器 |
| `7` | Settings | `views/settings_view.rs` | 设置编辑器（主题、频谱、音量等） |

### 5.2 状态结构

```rust
pub struct UiState {
    pub active_view: ViewMode,
    pub show_help: bool,
    pub tracks: Vec<TrackDisplay>,
    pub playing_index: Option<usize>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub volume: f32,
    pub repeat_mode: RepeatMode,
    pub lyrics_offset_ms: i64,
    pub current_lyric_index: usize,
    pub lyric_track: Option<LyricTrack>,
    pub search_query: String,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub fft_bars: Vec<f32>,
    pub notification: Option<Notification>,
    pub settings_state: SettingsState,
    pub library_state: LibraryUiState,
    pub playlist_state: PlaylistUiState,
    pub file_browser_state: FileBrowserState,
}
```

### 5.3 主题系统

5 套预设主题（`src/ui/theme.rs`），每套定义 12 种颜色：

| 颜色键 | 用途 |
|--------|------|
| `bg` / `fg` | 背景/前景 |
| `accent` | 标题、选中项高亮 |
| `highlight` | 当前播放行、进度条 |
| `dimmed` | 非焦点文字 |
| `success` | 播放状态指示 |
| `warning` | 音量警告 |
| `error` | 错误信息 |
| `bar_low/mid/high` | 频谱柱低/中/高频颜色 |

---

## 6. 事件系统

### 6.1 事件类型

```rust
pub enum AppEvent {
    Key(KeyEvent),             // 键盘输入
    Tick,                      // 定时触发（~30 FPS）
    Quit,                      // 退出
    JumpTop,                   // gg 跳到顶部
    JumpBottom,                // G 跳到底部
    RemoveSelected,            // dd 删除选中
    VisualizerData(Vec<f32>),  // FFT 频谱数据
}
```

### 6.2 事件循环

```rust
pub async fn run(&mut self, cli: Cli) -> AppResult<()> {
    // 1. 启用 raw mode + alternate screen
    // 2. 加载 CLI 指定的文件
    // 3. 启动 FFT 线程
    // 4. 事件循环：
    loop {
        tokio::select! {
            Some(Ok(event)) = reader.next() => {
                // crossterm EventStream → KeyHandler → handle_key_event()
                // 搜索模式/插入模式绕过 KeyHandler 双键延迟
            }
            _ = tick_interval.tick() => {
                self.handle_event(AppEvent::Tick);
            }
        }
        terminal.draw(|f| ui::render(f, &self))?;
        if self.should_quit { break; }
    }
    // 5. 停止 FFT、恢复终端
}
```

---

## 7. 配置系统

### 7.1 文件位置

所有配置文件自包含在项目 `config/` 目录中（非 XDG 路径，便携设计）。

| 文件 | 用途 |
|------|------|
| `config/config.toml` | 主配置（设置视图可在线编辑和保存） |
| `config/keybindings.toml` | 自定义快捷键（预留） |
| `data/state.json` | 退出时保存的状态 |
| `data/library.db` | 曲库 SQLite 数据库 |
| `data/tmper.log` | 运行日志 |

### 7.2 配置结构

```rust
pub struct Config {
    pub library: LibraryConfig,
    pub playback: PlaybackConfig,
    pub visualizer: VisualizerConfig,
    pub lyrics: LyricsConfig,
    pub ui: UiConfig,
}
```

**加载**：`Config::load_or_default()` — 读取 TOML → `toml::from_str` → 失败则用 `Default::default()`

**保存**：`write_config()` — `toml::to_string_pretty(&config)` → 写入文件（通过设置视图自动触发）

### 7.3 配置优先级

```
设置视图在线修改 > config/config.toml > 硬编码默认值
```

---

## 8. 持久化

### 8.1 状态保存

退出时（`AppEvent::Quit` 或 `should_quit`）调用 `save_state()`：

```json
{
  "volume": 0.8,
  "repeat_mode": "Sequential",
  "lyrics_offset_ms": 0,
  "last_track_path": "/home/user/Music/song.flac"
}
```

### 8.2 播放列表持久化

`save_playlists()` / `load_playlists()` — 歌单名称 + 歌曲路径数组，JSON 格式。

### 8.3 曲库路径持久化

`save_library_paths()` / `load_library_paths()` — 文件浏览器中的库路径列表。

---

## 9. 键盘处理

### 9.1 处理流程

```
crossterm KeyEvent → KeyHandler (双键序列检测: gg, dd)
    → handle_key_event() → 分发:
        ├─ 视图切换键 (1-7, 0)
        ├─ Esc (清除搜索/帮助)
        ├─ 搜索模式 (累积字符)
        ├─ 视图专用按键 (Playlists/Browser/Library/Settings)
        └─ 全局按键 (Space, n/p, j/k, Enter, ...)
```

### 9.2 双键序列

`KeyHandler` 记录上一个按键 + 时间戳。200ms 内收到第二个键 → 匹配序列（`gg` → JumpTop, `dd` → RemoveSelected）。超时 → 丢弃第一个键。

### 9.3 全局快捷键

| 键 | 事件 | 说明 |
|----|------|------|
| `Space` | 播放/暂停 | `engine.pause()` / `engine.resume()` |
| `n` / `p` | 下一首/上一首 | `next_track()` / `prev_track()` |
| `-` / `=` | 音量 | `volume ± 0.05` |
| `←` / `→` | Seek | `seek_relative(-5)` / `seek_relative(5)` |
| `j` / `k` | 移动选择 | ±1，边界 clamp |
| `gg` / `G` | 跳首/尾 | JumpTop / JumpBottom |
| `Ctrl+d/u` | 翻半页 | 10 行 |
| `dd` | 删除选中 | RemoveSelected |
| `r` | 循环模式 | Sequential → Shuffle → SingleTrack |
| `/` | 搜索 | 进入搜索模式 |
| `[` `]` `{` `}` | 歌词偏移 | ±500ms / ±2000ms |
| `Ctrl+r` | 重置偏移 | 0ms |
| `q` | 退出 | Quit |

---

## 10. 测试

### 10.1 测试覆盖

| 模块 | 测试数 | 覆盖内容 |
|------|--------|----------|
| audio/decoder.rs | 2 | 解码 WAV、不存在的文件 |
| audio/engine.rs | 4 | 生命周期、位置追踪、停止、排队 |
| lyrics/parser.rs | 7 | 标准 LRC、元数据、多时间戳、逐字、空文件、损坏行、排序 |
| visualizer/fft.rs | 1 | 440Hz 峰值检测 |
| visualizer/processor.rs | 2 | 桶数量、平滑收敛 |
| visualizer/render.rs | 2 | 渲染输出、颜色渐变 |
| library/database.rs | 6 | upsert、重复更新、搜索、artists、albums、delete |
| library/scanner.rs | 1 | 扩展名过滤 |
| library/playlist_manager.rs | 2 | M3U 往返、相对路径 |
| metadata/reader.rs | 3 | FLAC、WAV（无标签）、不存在的文件 |
| input/command.rs | 4 | quit、theme、volume、unknown |
| playlist.rs | 9 | push、next/prev、remove、shuffle、insert、边界条件 |
| **总计** | **41** | **全部通过** |

### 10.2 运行测试

```bash
cargo test                     # 全部测试
cargo test <test_name>         # 单个测试
cargo test -- --nocapture      # 显示输出
```

---

## 11. 项目结构

```
tmper/
├── Cargo.toml                  # 包名 tmper，Rust 2021 edition
├── CLAUDE.md                   # AI 助手指引
├── DESIGN.md                   # 本文件 — 架构与实现文档
├── README.md                   # 用户手册
├── tmper                       # 符号链接 → target/release/tmper
│
├── config/                     # 配置文件（自包含，非 XDG）
│   ├── config.toml             #   主配置（在线编辑，自动保存）
│   └── keybindings.toml        #   自定义快捷键（预留）
│
├── data/                       # 运行时数据（自动生成）
│   ├── tmper.log               #   运行日志
│   ├── state.json              #   退出时保存的状态
│   ├── playlists.json          #   歌单数据
│   ├── library.json            #   文件浏览器库路径
│   └── library.db              #   SQLite 曲库索引
│
├── themes/                     # 主题色板
│   ├── tokyo-night.toml
│   ├── dracula.toml
│   ├── nord.toml
│   ├── solarized-dark.toml
│   └── catppuccin-mocha.toml
│
├── progress/                   # 开发日志
│   ├── progress.md
│   ├── phase0-2026-07-12.md
│   ├── ...
│   ├── 2026-07-13-rewrite-and-fixes.md
│   └── 2026-07-13-cleanup.md
│
├── src/                        # 源代码
│   ├── main.rs                 #   入口：tracing init、config load、App::run()
│   ├── cli.rs                  #   clap CLI 参数解析
│   ├── config.rs               #   Config 加载/默认值
│   ├── constants.rs            #   运行时调优常量（集中管理 magic numbers）
│   ├── error.rs                #   AppError + AppResult<T>
│   ├── event.rs                #   AppEvent 枚举
│   ├── playlist.rs             #   TrackEntry + Playlist 数据结构
│   ├── paths.rs                #   项目内路径工具（config_dir, data_dir）
│   │
│   ├── app/                    #   应用核心
│   │   ├── mod.rs              #     App struct + run() 事件循环
│   │   ├── playback.rs         #     播放控制：move_selection, next_track, FFT 线程
│   │   ├── persistence.rs      #     状态/歌单/路径 保存和加载
│   │   └── handlers/           #     按键事件处理器
│   │       ├── mod.rs          #       handle_event、switch_view、全局按键
│   │       ├── playlist.rs     #       歌单视图按键
│   │       ├── library.rs      #       曲库视图按键
│   │       ├── browser.rs      #       文件浏览器按键
│   │       └── settings.rs     #       设置视图按键 + write_config
│   │
│   ├── audio/                  #   音频引擎
│   │   ├── decoder.rs          #     Symphonia 解码适配
│   │   ├── output.rs           #     Rodio Sink 封装
│   │   └── engine.rs           #     AudioEngine: 播放/暂停/seek/位置
│   │
│   ├── metadata/               #   元数据
│   │   └── reader.rs           #     lofty 标签读取 → TrackInfo
│   │
│   ├── lyrics/                 #   歌词系统
│   │   ├── types.rs            #     LyricLine、LyricTrack、LyricMetadata
│   │   ├── parser.rs           #     LRC 解析 + 编码检测
│   │   └── engine.rs           #     歌词查找 + 同步
│   │
│   ├── visualizer/             #   频谱可视化
│   │   ├── fft.rs              #     FFT 分析 (2048 点 Hann 窗)
│   │   ├── processor.rs        #     对数分桶 + 平滑 + 归一化
│   │   └── render.rs           #     Block Elements 字符渲染
│   │
│   ├── library/                #   音乐库
│   │   ├── database.rs         #     SQLite CRUD + 搜索
│   │   ├── scanner.rs          #     目录扫描 + mtime 检测
│   │   └── playlist_manager.rs #     M3U 导入/导出
│   │
│   ├── ui/                     #   用户界面
│   │   ├── mod.rs              #     UiState、ViewMode、render() 入口
│   │   ├── theme.rs            #     5 套预设主题
│   │   ├── cover/              #     封面图渲染（终端协议直接输出）
│   │   │   └── mod.rs          #       CoverRenderer: Kitty / SIXEL 协议
│   │   ├── views/              #     视图
│   │   │   ├── player_view.rs  #       播放器主视图
│   │   │   ├── library_view.rs #       曲库浏览器
│   │   │   ├── lyrics_view.rs  #       全屏歌词
│   │   │   ├── playlist_view.rs#       歌单管理器
│   │   │   ├── file_browser_view.rs #  文件浏览器
│   │   │   └── settings_view.rs#       设置编辑器
│   │   └── widgets/            #     可复用组件
│   │       ├── help_popup.rs   #       帮助面板
│   │       ├── lyrics_panel.rs #       播放器内歌词
│   │       └── visualizer_panel.rs #   频谱渲染组件
│   │
│   └── input/                  #   键盘输入
│       ├── handler.rs          #     KeyHandler（双键序列）
│       ├── keymap.rs           #     KeyBindings 配置（预留）
│       └── command.rs          #     : 命令解析器（预留）
│
└── tests/
    └── fixtures/               #   测试数据
        └── test.wav            #     440Hz 正弦波 (2s, 44100Hz, stereo)
```

---

## 12. 依赖清单

### 核心依赖

| 类别 | 库 | 版本 | 用途 |
|------|-----|------|------|
| 异步 | tokio | 1 | 事件循环、定时器、通道 |
| TUI | ratatui | 0.29 | Widget 系统、布局引擎 |
| 终端 | crossterm | 0.28 | 原始模式、键盘事件、颜色 |
| 音频输出 | rodio | 0.20 | 跨平台音频 Sink（基于 cpal） |
| 音频解码 | symphonia | 0.5 | 多格式解码器（全部特性启用） |
| 元数据 | lofty | 0.22 | 音频标签解析 |
| FFT | rustfft | 6 | 快速傅里叶变换（纯 Rust + SIMD） |
| 配置 | toml | 0.8 | TOML 序列化/反序列化 |
| 序列化 | serde + serde_json | 1 | 配置/状态 JSON |
| CLI | clap | 4 | 命令行参数解析 |
| 数据库 | rusqlite | 0.32 | SQLite（bundled） |
| 目录遍历 | walkdir | 2 | 递归目录扫描 |
| 正则 | regex | 1 | LRC 解析正则 |
| 编码 | encoding_rs | 0.8 | GBK/Shift-JIS 歌词编码检测 |
| 日志 | tracing + tracing-subscriber | 0.1/0.3 | 结构化日志 |
| 错误 | thiserror + anyhow | 2/1 | 错误类型 + 传播 |
| 终端流 | futures-util | 0.3 | crossterm EventStream |

---

## 13. 版本历史

| 版本 | 日期 | 变更 |
|------|------|------|
| v3.0 | 2026-07-12 | 初始实现文档 |
| v3.1 | 2026-07-17 | 重构：提取 constants.rs、覆盖渲染模块化、统一视图切换 |
| v3.2 | 2026-07-18 | 代码质量改进：PCM 缓冲增大、消除 clippy allow、render() 改 match、Config 默认值去重、RUST_LOG 支持、KeyHandler 控制字符过滤 |
| v3.3 | 2026-07-18 | Session A–D：UiState 视图参数抽取 + 状态分组 (PlayerCore/LyricsState/ViewState)、play_file 异步化解码、stdout 防护增强 (4 层防御) |

### 已知技术债（v3.3 记录）

> 详细计划见 `progress/2026-07-18-deferred-issues-plan.md`

| 问题 | 严重度 | 说明 | 状态 |
|------|--------|------|------|
| UiState 上帝结构体 | 🔴 | 30+ 字段 → 13 分组 + 3 Cell | ✅ v3.3 完成 |
| stdout 直接写入 | 🔴 | 4 层防御：suppress + drain + control-char filter + cooldown | ✅ v3.3 完成 |
| play_file 同步解码 | 🟡 | spawn_blocking + 独立 Sink，>50MB 自动异步 | ✅ v3.3 完成 |
| 无集成测试 | 🟡 | 42 → 54（+12 集成测试） | ✅ v3.3 完成 |
