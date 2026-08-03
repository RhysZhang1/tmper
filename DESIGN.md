# tmper — 项目架构与实现文档

> **项目名称**: tmper — 终端音乐播放器
> **语言**: Rust
> **平台**: Arch Linux + KDE Plasma
> **文档版本**: v3.6（实现文档）
> **最后更新**: 2026-08-02

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
│  ├─ theme.rs     — 13 色槽语义主题（themes/*.toml）  │
│  ├─ views/       — 7 个视图                           │
│  │  ├─ player_view.rs  — 播放器（封面+频谱+歌词+列表） │
│  │  ├─ library_view.rs — 曲库（三栏浏览+搜索）         │
│  │  ├─ lyrics_view.rs  — 全屏歌词（KTV 风格）          │
│  │  ├─ playlist_view.rs— 歌单管理器                    │
│  │  ├─ file_browser_view.rs — 文件浏览器               │
│  │  └─ settings_view.rs — 设置编辑器                   │
│  └─ widgets/     — 可复用组件                          │
│     ├─ help_popup.rs       — 帮助面板（键 8）           │
│     └─ visualizer_panel.rs — 频谱渲染                   │
└────────────────────────┬─────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────┐
│  业务层                                               │
│  ├─ AudioEngine (src/audio/engine.rs)                 │
│  │  ├─ decoder.rs — Symphonia 解码适配                │
│  │  └─ output.rs  — Rodio Sink 封装                   │
│  ├─ LibraryDb (src/library/database.rs)               │
│  │  ├─ scanner.rs        — 测试辅助（仅 cfg(test)）   │
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
│      batch = event_rx.recv() => {                    │
│          for event in batch {                        │
│              KeyHandler 处理 → handle_event(event)    │
│              → 修改 AppState                          │
│          }                                           │
│      }                                               │
│      _ = tick_interval.tick() => {                   │
│          更新播放位置、FFT 数据、歌词同步              │
│          检测曲目结束 → 自动切歌                       │
│      }                                               │
│  }                                                   │
│  terminal.draw(...)  ← 每批/节流 tick 至多绘制一次     │
└─────────────────────────────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────┐
│         输入线程 (spawn_blocking，burst 模式)           │
│                                                      │
│  loop {                                              │
│    poll(80ms) 等待首个事件                            │
│    → 批量 read() 排空 PTY 缓冲（终端 auto-repeat）     │
│    → send(Vec<CrosstermEvent>)                       │
│  }                                                   │
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
│      → 写入共享 fft_data（Arc<Mutex<Vec<f32>>>），主循环 tick 读取      │
│      sleep(32ms)  // ~31 FPS                         │
│    }                                                 │
│                                                      │
│  InstrumentedSource (在 rodio 管线中):                │
│    Source::next() 被调用时 → 拷贝采样到 pcm_buffer     │
│    → FFT 线程读取                                     │
└─────────────────────────────────────────────────────┘
```

**输入模型（burst 模式，commit e122084）**：不再使用 crossterm `EventStream`（一个事件 → 一次绘制）。改为后台线程 `crossterm::event::poll(80ms)` 等待首个事件，然后批量 `read()` 一次性排空 PTY 缓冲（长按按键时终端 auto-repeat 约 33ms 一个事件，每批可收集 2–3 个），经 `mpsc::unbounded_channel<Vec<CrosstermEvent>>` 发给主循环。主循环 `event_rx.recv()` 拿到整批事件后统一处理、绘制一次，消除了「松开按键仍持续滚动」的卡顿。

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

**seek**：`seek_relative(secs)` 从目标位置重新解码整个文件，送入新 Sink。解码在后台线程进行（镜像 `play_file_async` 的 `Arc<Sink>` 模式），位置立即跳到目标，不阻塞事件循环。

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

#### scanner.rs — 测试辅助（非生产模块）

> 该文件整体 `#[cfg(test)]`，仅作为**测试辅助**存在；项目当前**没有**生产目录扫描器，
> 也没有基于 mtime 的增量扫描。`follow_symlinks` 配置键已移除。

**测试**：扩展名过滤验证（1 个）。

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
    Play { file: PathBuf },
}
```

支持：`tmper`（交互模式）、`tmper play <path>`（播放**单个文件**；目录播放暂未实现）。

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
      ├─ 读取共享 fft_data（Arc<Mutex>）→ 更新频谱
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
- **共享内存**：FFT 数据通过 `Arc<Mutex<Vec<f32>>>`（`fft_data`）写入 `UiState.visualizer_data`；播放位置经 `tokio::sync::watch` 推送
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
    pub theme: Theme,                    // 当前主题（UI 颜色统一来自 UiState.theme，不硬编码）
    pub player: PlayerCore,              // 播放核心：曲目信息、position/duration、tracks、选中、滚动
    pub volume: f32,
    pub repeat_mode: RepeatMode,
    pub lyrics: LyricsState,             // 歌词：lyric_track、current_lyric_index、lyrics_offset_ms
    pub visualizer_data: Vec<f32>,       // FFT 共享数据（后台线程写 Arc<Mutex<Vec<f32>>>，主循环读取）
    pub view: ViewState,                 // 当前视图 + 帮助标志（active_view、show_help）
    pub playlist_state: PlaylistManagerState,
    pub file_browser_state: FileBrowserState,
    pub library_state: LibraryState,
    pub settings_state: SettingsState,
    pub active_playlist: Option<usize>,  // 跨视图播放上下文
    pub active_playlist_song: Option<usize>,
    pub playlist_name: String,
    pub command_mode: bool,
    pub command_buffer: String,
    pub search_mode: bool,               // 全局 / 搜索（播放器队列实时过滤）
    pub search_query: String,
    pub notification: Option<(String, std::time::Instant)>,
    pub visible_rows: Cell<usize>,
    pub cover_rect: Cell<(u16, u16, u16, u16)>,
}
```

### 5.3 主题系统

`src/ui/theme.rs` 定义了一个 **13 色槽语义化 `Theme` 结构体**（另含 `name` 字段），
从 `themes/<name>.toml` 加载（5 套真实配色：Tokyo Night、Dracula、Nord、Solarized Dark、Catppuccin Mocha）。
UI 颜色统一取自 `UiState.theme`，不再硬编码；`ui.theme` 配置键、`:theme <名称>` 命令与设置视图均可实时切换。

| 颜色键 | 用途 |
|--------|------|
| `primary` | 边框、面板标题、当前歌词、选中项 |
| `text` | 加粗标题 |
| `secondary` | 艺术家、次要文字 |
| `muted` | 边框、标签、提示 |
| `success` | 播放指示、音量、通知、确认 |
| `warning` | 命令模式、设置边框 |
| `control` | 控制栏 |
| `accent` | 活动歌单名 |
| `album` | 专辑 |
| `genre` | 流派 |
| `year` | 年份 |
| `codec` | 编码格式 |
| `bg` | 终端背景 |

---

## 6. 事件系统

### 6.1 事件类型

```rust
pub enum AppEvent {
    Key(KeyEvent),             // 键盘输入
    Tick,                      // 定时触发（~30 FPS）
    Quit,                      // 退出
    JumpTop,                   // gg 跳到顶部
    RemoveSelected,            // dd 删除选中
}
```

### 6.2 事件循环

```rust
pub async fn run(&mut self, cli: Cli) -> AppResult<()> {
    // 1. 启用 raw mode + alternate screen
    // 2. 加载 CLI 指定的文件
    // 3. 启动 FFT 线程 + burst 输入线程（poll/read → unbounded_channel）
    // 4. 事件循环：
    loop {
        tokio::select! {
            Some(batch) = event_rx.recv() => {
                // burst 模式：整批 Vec<CrosstermEvent> 统一处理
                // 逐事件 → KeyHandler → handle_key_event()
                // 搜索/插入模式绕过 KeyHandler 双键延迟
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
| `config/default.toml` | 默认配置模板（随仓库分发） |
| `config/config.toml` | 主配置（首次运行由 `default.toml` 自动复制生成；设置视图可在线编辑和保存） |
| `config/keybindings.toml` | 自定义快捷键（预留） |
| `data/state.json` | 退出时保存的状态 |
| `data/library.db` | 曲库 SQLite 数据库 |
| `data/tmper.log` | 运行日志 |

### 7.2 配置结构

```rust
pub struct Config {
    pub playback: PlaybackConfig,     // default_volume, seek_step_small_secs
    pub visualizer: VisualizerConfig, // num_bars, frame_rate, smoothing
    pub ui: UiConfig,                 // theme, show_cover_art
}
```

> `[library]` / `[lyrics]` 配置段及 `gapless`、`resume_on_startup`、`color_scheme` 等键均已移除，仅保留以上 7 个键。

**加载**：首次运行 `Config::ensure_config_file()` 将 `config/default.toml` 复制为 `config/config.toml`；随后 `Config::load_or_default()` — 读取 `config/config.toml` → `toml::from_str` → 失败则用 `Default::default()`

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

> 启动时 `load_state()` 仅恢复 **音量、循环模式、歌词偏移**；`last_track_path` 仅作记录，**不会**自动恢复播放。

### 8.2 播放列表持久化

`save_playlists()` / `load_playlists()` — 歌单名称 + 歌曲路径数组，JSON 格式。

### 8.3 曲库路径持久化

`save_library_paths()` / `load_library_paths()` — 文件浏览器中的库路径列表。

---

## 9. 键盘处理

### 9.1 处理流程

```
crossterm KeyEvent → KeyHandler (双键序列检测: gg, dd)
    → handle_key_event():
        ├─ 视图切换键 (1-7, 8)
        ├─ Esc (清除搜索/帮助)
        ├─ 搜索模式 (累积字符)
        ├─ 视图专用按键 (Playlists/Browser/Library/Settings)
        └─ 全局按键 (Space, n/p, j/k, Enter, ...)
```

### 9.2 双键序列

`KeyHandler` 记录上一个按键 + 时间戳。200ms 内收到第二个键 → 匹配序列（`gg` → JumpTop, `dd` → RemoveSelected）。超时 → 丢弃第一个键。

### 9.3 Cover 渲染与输入隔离

SIXEL/Kitty 封面数据直接写入 stdout（绕过 ratatui 差分缓冲），Konsole 等终端可能把转义字节误读为 stdin 产生虚假按键。2026-08-03 起的设计（详见 `progress/2026-08-03-cover-refactor.md`）：

1. **一次性发送**：SIXEL/Kitty 是持久图形层——发送后不随 ratatui 重绘消失。封面只在变化时发送一次（换歌、改渲染区域、切回播放器视图），不再每帧重发。**每帧重发是伪按键与卡顿的历史根因**（见 `progress/2026-08-01-cover-rollback.md`）。
2. **协议互斥**：Kitty 与 chafa SIXEL 按终端环境检测二选一，避免两者同时写入争抢同一区域。
3. **输入零防御**：不再需要 guard / 帧抑制 / 控制字符过滤等防御层。`handle_key_event` 不拦截任何按键。
4. **清理**：离开播放器视图、隐藏封面、或新曲目无封面时，置 `clear_pending` → 事件循环 `terminal.clear()` 覆盖 SIXEL 残留（Konsole 对 ED 清 SIXEL 的 workaround）。
5. 渲染器经注入式 writer + encoder 可测试，状态机由 6 个单元测试锁定（`src/ui/cover/mod.rs`）。

### 9.4 全局快捷键

| 键 | 事件 | 说明 |
|----|------|------|
| `Space` | 播放/暂停 | `engine.pause()` / `engine.resume()` |
| `n` / `p` | 下一首/上一首 | `next_track()` / `prev_track()` |
| `-` / `=` | 音量 | `volume ± 0.05` |
| `←` / `→` | Seek | `seek_relative(-5)` / `seek_relative(5)` |
| `j` / `k` | 移动选择 | ±1，边界 clamp |
| `gg` / `G` | 跳首/尾 | JumpTop / 直接滚动到末尾（无独立事件） |
| `Ctrl+d/u` | 翻半页 | 10 行 |
| `dd` | 删除选中 | RemoveSelected |
| `r` | 循环模式 | Sequential → Shuffle → SingleTrack |
| `/` | 搜索 | 播放器队列实时过滤（j/k 选结果，Enter 播放并退出） |
| `[` `]` `{` `}` | 歌词偏移 | ±500ms / ±2000ms |
| `Ctrl+r` | 重置偏移 | 0ms |
| `q` | 退出 | Quit |

---

## 10. 测试

### 10.1 测试覆盖

| 模块 | 测试数 | 覆盖内容 |
|------|--------|----------|
| audio/decoder.rs | 3 | 解码 WAV、不存在的文件、seek |
| audio/engine.rs | 5 | 生命周期、位置追踪、停止、排队（含 1 个 tokio 集成式） |
| lyrics/parser.rs | 7 | 标准 LRC、元数据、多时间戳、逐字、空文件、损坏行、排序 |
| visualizer/fft.rs | 1 | 440Hz 峰值检测 |
| visualizer/processor.rs | 2 | 桶数量、平滑收敛 |
| visualizer/render.rs | 2 | 渲染输出、颜色渐变 |
| library/database.rs | 6 | upsert、重复更新、搜索、artists、albums、delete |
| library/scanner.rs | 1 | 扩展名过滤（测试辅助） |
| library/playlist_manager.rs | 2 | M3U 往返、相对路径 |
| metadata/reader.rs | 3 | FLAC、WAV（无标签）、不存在的文件 |
| ui/theme.rs | 4 | hex 颜色解析（有效/无效回退）、缺失主题回退默认、真实主题 13 色槽加载 |
| input/command.rs | 4 | quit、theme、volume、unknown |
| playlist.rs | — | （v3.5 后仅存 `PlaylistData` 数据模型，逻辑并入 playlist_view） |
| app/mod.rs | 13 | 视图切换、音量、循环、加载播放、停止、命令模式、搜索（含 4 个 tokio 集成式） |
| ui/cover/mod.rs | 6 | 一次性发送不变量、区域重发、视图切换/隐藏/无封面清除、chafa 失败不重试（注入式 writer/encoder） |
| **总计** | **59** | **54 单元 + 5 集成** |

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
│   ├── default.toml            #   默认配置模板
│   ├── config.toml             #   主配置（首次运行由 default.toml 自动复制生成）
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
│   ├── playlist.rs             #   PlaylistData 数据结构（唯一歌单模型）
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
│   │   ├── scanner.rs          #     测试辅助（仅 #[cfg(test)]，非生产扫描器）
│   │   └── playlist_manager.rs #     M3U 导入/导出
│   │
│   ├── ui/                     #   用户界面
│   │   ├── mod.rs              #     UiState、ViewMode、render() 入口
│   │   ├── theme.rs            #     13 色槽语义主题（themes/*.toml 加载）
│   │   ├── cover/              #     封面图渲染（终端协议直接输出）
│   │   │   └── mod.rs          #       CoverRenderer: Kitty/SIXEL 互斥、一次性发送、6 测试
│   │   ├── views/              #     视图
│   │   │   ├── player_view.rs  #       播放器主视图
│   │   │   ├── library_view.rs #       曲库浏览器
│   │   │   ├── lyrics_view.rs  #       全屏歌词
│   │   │   ├── playlist_view.rs#       歌单管理器
│   │   │   ├── file_browser_view.rs #  文件浏览器
│   │   │   └── settings_view.rs#       设置编辑器
│   │   └── widgets/            #     可复用组件
│   │       ├── help_popup.rs   #       帮助面板
│   │       └── visualizer_panel.rs #   频谱渲染组件
│   │
│   └── input/                  #   键盘输入
│       ├── handler.rs          #     KeyHandler（双键序列）
│       ├── keymap.rs           #     KeyBindings 配置（预留）
│       └── command.rs          #     : 命令解析器（预留）
│
└── tests/
    └── fixtures/               #   测试数据
        ├── test.wav            #     440Hz 正弦波 (2s, 44100Hz, stereo)
        ├── test.flac           #     带完整标签的 FLAC
        └── test_notags.wav     #     无标签 WAV（验证 fallback）
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

---

## 13. 版本历史

| 版本 | 日期 | 变更 |
|------|------|------|
| v3.0 | 2026-07-12 | 初始实现文档 |
| v3.1 | 2026-07-17 | 重构：提取 constants.rs、覆盖渲染模块化、统一视图切换 |
| v3.2 | 2026-07-18 | 代码质量改进：PCM 缓冲增大、消除 clippy allow、render() 改 match、Config 默认值去重、RUST_LOG 支持、KeyHandler 控制字符过滤 |
| v3.3 | 2026-07-18 | Session A–D：UiState 视图参数抽取 + 状态分组 (PlayerCore/LyricsState/ViewState)、play_file 异步化解码、stdout 防护增强 (4 层防御) |
| v3.4 | 2026-07-19 | 回滚长按快进快退；事件循环绘制节流（~20fps）修复播放时滚动卡顿；键1迷你歌单独立滚动状态修复末行消失 |
| v3.5 | 2026-07-19 | seek_relative 改为后台线程异步解码（镜像 play_file_async）；git 卫生（分支 rename main、清理 tar.gz、补 gitignore）；文档同步（帮助键 8、测试数 53、事件模型） |
| v3.6 | 2026-08-02 | 清理死配置键（`[library]`/`[lyrics]` 段、gapless、crossfade_seconds、resume_on_startup、color_scheme、char_set、show_on_idle、scan_on_startup、follow_symlinks、show_progress_bar、cover_art_max_width、default_view 全部移除）；首运行自动生成 `config/config.toml`；真实主题系统（13 色槽 `Theme`，从 `themes/*.toml` 加载）；输入改为 burst 模式（poll/read 批量读取）；启动恢复音量/循环模式/歌词偏移 |
| v3.7 | 2026-08-03 | 封面收敛：SIXEL 每次变化只发一次（替代每帧重发）；删除 cover-escape guard / 帧抑制 / `COVER_SUPPRESS_FRAMES`；Kitty 与 SIXEL 协议互斥；无封面曲目清除残留 SIXEL；chafa 空输出 sticky 失败标记；移除裸 `\x1b[?25l`；CoverRenderer 注入式 writer/encoder + 6 测试 |

### 已知技术债（v3.4 更新）

| 问题 | 严重度 | 说明 | 状态 |
|------|--------|------|------|
| UiState 上帝结构体 | 🔴 | 30+ 字段 → 13 分组 + 3 Cell | ✅ v3.3 完成 |
| stdout 直接写入 | 🟡 | 已收敛为"每次变化一次发送 + 协议互斥"，无输入防御层；仍绕过 ratatui 差分缓冲（架构约束，非 bug） | 🟡 大幅缓解（v3.7） |
| play_file 同步解码 | 🟡 | 异步路径对 >50MB 启用；seek_relative 仍同步 | 🟡 部分 |
| 无集成测试 | 🟡 | 59 测试（54 单元 + 5 集成） | ✅ v3.3 完成 |
| 长按快进快退 | 🟡 | 已回滚（crossterm 无按键释放检测） | ✅ v3.4 回滚 |
| 播放时滚动卡顿 | 🟡 | 事件循环绘制节流至 ~20fps | ✅ v3.4 修复 |
| 键1 迷你歌单末行消失 | 🟡 | 侧边栏独立滚动状态 | ✅ v3.4 修复 |
| seek_relative 同步解码 | 🟡 | 跳转时阻塞事件循环 | ✅ v3.5 后台线程异步解码 |
