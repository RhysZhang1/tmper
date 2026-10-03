# tmper — 项目架构与实现文档

> **项目名称**: tmper — 终端音乐播放器
> **语言**: Rust
> **平台**: Linux（主要在 Arch Linux + KDE Plasma 验证）
> **文档状态**: 当前实现
> **最后更新**: 2026-10-03

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
- **曲库**: 后台目录扫描、SQLite 增量索引、FTS5 全文搜索
- **7 个视图**: 播放器、曲库、歌词、频谱、歌单管理、文件浏览器、设置
- **5 套主题**: Tokyo Night、Dracula、Nord、Solarized Dark、Catppuccin Mocha
- **Vim 风格操作**: 模态键盘，双键序列（gg、dd），/ 搜索
- **播放与界面分进程**: 关掉 TUI 音乐继续；重新打开接回同一首同一进度
- **MPRIS2**: 桌面媒体控件、媒体键与 `playerctl` 驱动的是 TUI 看到的那份状态

---

## 2. 整体架构

### 2.1 两个进程，一个 binary

```
                     ┌────────────────────────────┐
                     │  tmper（TUI 客户端）        │
                     │  App + UiState + ratatui   │
                     │  曲库/歌单/扫描只是镜像      │
                     └─────────────┬──────────────┘
                                   │  NDJSON over
                     unix socket   │  $XDG_RUNTIME_DIR/tmper/socket
                                   │
                     ┌─────────────▼──────────────┐        ┌──────────────┐
                     │  tmper daemon              │◄──────►│ 会话总线      │
                     │  Player：engine + 队列 +    │  zbus  │ org.mpris…   │
                     │  策略 + 曲库 + 歌单 + 扫描   │        └──────────────┘
                     └─────────────┬──────────────┘
                                   │
                       ┌───────────▼───────────┐
                       │ rodio → 声卡           │
                       │ FFT 线程 → 频谱帧       │
                       │ state.json / *.db     │
                       └───────────────────────┘
```

`main.rs` 按动词分流，三种身份：

| 参数 | 走的路 | 说明 |
|---|---|---|
| `daemon` | `daemon::run()` | 前台跑播放器（正常由 TUI 按需分离启动） |
| `pause` / `next` / `status` / `quit` /… | `client::run_control()` | 连上、发一条、打印、退出；**绝不启动 daemon** |
| 无参数 / `play <文件>` | TUI | 连不上就拉起 daemon |

判定收在 `cli::Command::controls_a_running_player()`。`play` 是唯一两种身份都有的动词：带文件是「打开界面播放它」，不带文件是「接着放」——也就是媒体键发的那个意思。

**客户端分层**（`src/app/`，就是客户端，没有改名）：

```
┌──────────────────────────────────────────────────────┐
│  App (src/app/mod.rs) — 事件循环 + UiState            │
│  ├─ handle.rs    — PlayerHandle trait                │
│  │                 ├─ DaemonHandle（socket，生产）     │
│  │                 └─ LocalHandle（进程内，仅测试）     │
│  ├─ playback.rs   — 选曲、歌词装载                     │
│  └─ handlers/     — 按键分发（mod/playlist/library/    │
│                     browser/settings）                │
└────────────────────────┬─────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────┐
│  UI (src/ui/) — 渲染层（只读 &UiState）                │
│  ├─ mod.rs       — UiState, ViewMode, render()       │
│  ├─ theme.rs     — 13 色槽语义主题（themes/*.toml）    │
│  ├─ views/       — 7 个视图                           │
│  └─ widgets/     — 帮助面板、频谱面板                   │
└──────────────────────────────────────────────────────┘
```

**daemon 分层**（`src/player/` + `src/daemon.rs`）：

```
┌──────────────────────────────────────────────────────┐
│  daemon.rs — 监听、每客户端一个任务、空闲退出、MPRIS 装配 │
└────────────────────────┬─────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────┐
│  Player (src/player/mod.rs) — 引擎 + 队列 + 续播策略    │
│  ├─ library.rs      — LibraryDb + 扫描任务登记         │
│  ├─ playlists.rs    — 歌单 store（身份是 id）          │
│  ├─ persistence.rs  — state.json（唯一写入者）          │
│  ├─ cover.rs        — 封面缓存（供 mpris:artUrl）       │
│  ├─ fft.rs          — 频谱线程 + 订阅                  │
│  └─ mpris.rs        — MPRIS2 接口（zbus）              │
└────────────────────────┬─────────────────────────────┘
                         │
┌────────────────────────▼─────────────────────────────┐
│  audio/（engine + decoder + output）、library/（SQLite  │
│  + scanner + M3U）、visualizer/{fft,processor}         │
└──────────────────────────────────────────────────────┘
```

### 2.2 并发模型

**daemon 的主循环**（`daemon::run`，单线程持有 `Player`）：

```
loop {
  tokio::select! {
    接受新连接        → spawn(serve_client)：握手、收请求、写事件
    客户端消息到达     → client_joined / handle(request) / client_left
    MPRIS 命令到达    → handle_mpris(request)
    ticker（1/帧率）  → daemon.tick()：推进引擎、路由事件、推快照
  }
  if daemon.should_exit(now) { break }
}
```

`Player` **是 `!Send`**（内部 cpal 流只能在建它的线程上驱动），所以循环直接持有它，客户端只能通过 channel 够到它——音频栈强加的单写者规则，恰好和项目其余部分一致。

**daemon 永不等待客户端。** 每个连接一个**有界信箱**（`DAEMON_CLIENT_QUEUE`），投递用 `try_send`：满了就丢**快照与频谱帧**（下一帧就取代它们，代价是一次闪烁），而丢**队列变化或通知**意味着该客户端再也收不到的事实——那种情况下把客户端丢掉，播放不受影响。

**客户端的主循环**（`App::run`）：

```
┌─────────────────────────────────────────────────────┐
│                    主线程 (tokio)                     │
│  tokio::select! {                                   │
│      batch = event_rx.recv() => { 逐事件 → KeyHandler │
│                                  → handle_event }    │
│      _ = tick_interval.tick() => {                   │
│          handle_tick：poll+socket 事件 → apply_event │
│          歌词同步、连接状态读取、必要时重绘            │
│      }                                               │
│  }                                                   │
│  terminal.draw(...)  ← 每批/节流 tick 至多绘制一次     │
└─────────────────────────────────────────────────────┘
```

位置不再由客户端推进：daemon 每 tick 推一份 `StateSnapshot`，客户端只用它显示。歌词是位置的纯函数，仍在客户端逐 tick 计算（不需要过网）。

```
┌────────────────────────▼─────────────────────────────┐
│         输入线程 (spawn_blocking，burst 模式)           │
│  loop { poll(80ms) → 批量 read() 排空 PTY 缓冲         │
│         → send(Vec<CrosstermEvent>) }                │
└──────────────────────────────────────────────────────┘
┌────────────────────────▼─────────────────────────────┐
│  daemon 侧后台线程                                     │
│  FFT 线程: 读 pcm_buffer → FftAnalyzer → Processor     │
│            → Event::Visualizer（仅在有订阅者时）        │
│  解码线程: spawn_blocking，按 ~2s 高水位背压            │
│  InstrumentedSource: Source::next() 拷采样到 pcm_buffer│
└──────────────────────────────────────────────────────┘
```

**输入模型（burst 模式）**：后台线程通过 crossterm 等待首个事件，然后批量排空 PTY 缓冲，经 channel 发送事件批次。主循环处理整批事件后只绘制一次，避免终端按键自动重复造成绘制堆积。

### 2.3 MPRIS2（只在 daemon 里）

`mpris-server` 0.10（基于 zbus，复用 tokio 运行时），在会话总线上占名 `org.mpris.MediaPlayer2.tmper`。**桌面看到的状态和客户端看到的状态是同一份状态、播报一次**：`Daemon::route` 在把 `Event::Snapshot` 发给客户端的同时把它交给 MPRIS。

| MPRIS | tmper |
|---|---|
| `PlaybackStatus` | Playing / Paused / Stopped（Loading、Seeking 归入 Playing） |
| `LoopStatus` + `Shuffle` | Sequential→`None`；SingleTrack→`Track`；Shuffle→`Playlist` 且 `Shuffle(true)` |
| `Volume` | 0.0–1.0 直通（取三位小数） |
| `Metadata` | `mpris:trackid` 是对象路径 `/tmper/track/{:016x}`；`mpris:length` 是**微秒**；`xesam:title/artist/album`；`mpris:artUrl` 是封面缓存的 `file://` URL |
| `CanRaise=false` / `Raise` | no-op：它是个 TUI，没有窗口可抬 |
| `CanQuit=true` / `Quit` | 退出 daemon |

三处细节是**承重**的：

1. **只播报动了的属性。** 快照每秒来三十来个，绝大多数只差 `position_secs`，而位置不是 MPRIS 属性（客户端要位置时会问 `Position`）。`State::apply` 把自己和上次播报过的镜像做 diff。不 diff 就是每秒三十条 `PropertiesChanged` 和一块永远在重绘的控件。
2. **音量要收窄。** `f32` 直接加宽到 `f64` 会得到 `0.800000011920929`——同一条 80% 挂了十一位噪声，而且不再往返（设 0.8、读回 0.8 却发现变了，diff 于是永远发现同一个「变化」）。`to_volume` 取三位小数。
3. **setter 要先把值记进镜像。** zbus 对每个属性 `Set` 会**自己**发一条 `PropertiesChanged`，内容在 setter 返回那一刻从 getter 读——而 tmper 的 setter 是 fire-and-forget，返回时 daemon 还没应用。不记的话，每次调音量都被播报成「被替换掉的那个音量」，一毫秒后才是真的：滑条动一格先弹回去、再弹过来。记录的规则是**只记客户端点名的那一个 flag**：另一个若真被这次 `Set` 带着动了（`Shuffle=true` 就是 `Playlist`），随后那条快照会播报它，而那是控件**需要**知道的事实。同理 `set_shuffle(false)` 不是「停止循环」——它要回读循环轴再决定请求 `Sequential` 还是 `SingleTrack`（控件每次刷新都会重报自己的值）。

seek 是唯一「结果在所有属性里都看不见」的命令，所以拿到 MPRIS 为此专设的那条信号：状态之后跟一条 `Seeked`。

### 2.4 生命周期一览

| 事件 | 结果 |
|---|---|
| `q` / `:quit` | 只关 TUI；daemon 与音乐继续 |
| `:quit!` / `tmper quit` / MPRIS `Quit` | daemon 存盘、推 `Bye`、停播、退出；客户端收到 `Bye` 也退出 |
| 空闲 5 分钟（无人连着且未出声） | daemon 存盘后自己退出 |
| daemon 意外死亡，TUI 还开着 | 顶部横幅 + 后台重连；按键排队，恢复后按序送达 |
| daemon 死亡，客户端是一次性动词 | 报告失败（`the player closed the connection`），**不**顺手拉起一个 |
| socket 文件是死的 | 启动时删掉重建 |

---

## 3. 模块详解

### 3.1 IPC 与协议 (src/ipc/)

`$XDG_RUNTIME_DIR/tmper/socket`（`cfg(test)` 下落到测试根），行分隔 JSON：一行一个对象，UTF-8，`\n` 结尾。选它是因为 `serde_json` 本来就是依赖、不引入新依赖，而且能用 `nc -U` 手工查。

**协议是不对称的。** 客户端发 `Request`；daemon 说的一切都是 `Event`，**包括对请求的答复**（`LibraryArtists` 请求 → `Event::LibraryArtists`；失败 → `Event::Notice`）。这样客户端（一个 TUI）永远不必阻塞等回包。

```rust
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Request { Hello { proto, version }, Play { path }, Toggle, Pause, Resume,
                   Stop, Next, Prev, SeekRelative { secs }, SetVolume { volume },
                   VolumeStep { delta }, SetRepeat { mode }, QueuePush { path },
                   QueueRemove { path }, SetActivePlaylist { id }, …, Shutdown }

#[serde(tag = "t", rename_all = "snake_case")]
pub enum Event { Welcome { proto, version, pid }, Snapshot(Box<StateSnapshot>),
                 Queue { rev, tracks }, Visualizer { bars }, LibraryArtists { … },
                 Playlists { … }, Notice { level, message }, Bye, … }
```

**状态整体推送，不做增量。** 每 tick 一份 `StateSnapshot`（位置、时长、状态、音量、循环、元数据、封面缓存路径、`queue_rev`、`active_playlist`）。快照是全量的，因此**重连不需要补课协议**——下一个快照就是补课。队列、歌单、曲库路径、事件都只在变化时推，各带一个版本号或键。

**两条 framing 实现之间存在第二份代码，靠测试钉住。** `BufRead` 与 `AsyncBufRead` 没有共同父 trait，所以 `read_line` 的逻辑存在两次；约束被刻意收窄（上限、容忍 CRLF、区分 EOF 与截断），再由测试把**同一串字节**喂给两个实现断言结论一致。

**读写各有一次踩过的坑。** `BufReader` 为了回答一次 `read_line` 会把能读到的字节全部搬进自己的缓冲，所以：握手必须用**客户端随后继续使用的那一个 reader**（`DaemonHandle::dial` → `read_events`），而 socket 的读任务**绝不能作为 `select!` 的分支**——分支被取消时读了一半的行连同缓冲区一起消失。详见 `progress/2026-10-03-daemon-split.md`。

### 3.2 daemon (src/daemon.rs)

- **陈旧 socket**：启动时若 socket 文件存在，先试着连它——连得上说明已有 daemon 在跑，第二个直接退出（一张声卡上两个播放器不是这个程序回答得了的问题）；连不上（`ECONNREFUSED`/`ENOENT`）说明是死掉的 daemon 留下的，删掉重建。daemon 退出时自己 `remove_file`。
- **空闲退出**：`clients.is_empty() && !status.is_active()` 持续 `DAEMON_IDLE_EXIT_SECS`（300 秒）。**暂停算空闲**——它只是按住一个位置而不是在做事情，而一个暂停可以挂好几天，把它算作活动就等于永不退出。判定收在 `Daemon::should_exit(now)`，`now` 是参数，测试因此不必等五分钟。
- **退出前存现场**：`Player::shutdown()` 先 `save_state()` 再静音设备——顺序是承重的（`stop_playback()` 是回卷，先静音就会把 `position_secs: 0.0` 存进去）。
- **MPRIS 在 socket 之后装配，且从不致命**：没有会话总线就记一条日志照常播放。
- **`spawn_detached` 在 `cfg(test)` 下拒绝执行**：测试二进制里 `current_exe()` 是测试 harness，spawn 它等于把整个测试套件脱离地重跑一遍。

### 3.3 Player：daemon 的内核 (src/player/)

`Player` 拥有 `AudioEngine`、队列、续播策略、曲库、歌单、`state.json`。对外只有两个动词：`execute(Request) -> Vec<Event>`（客户端要它做的事）和 `tick() -> Vec<Event>`（时间推进后它自己产生的事）。**换曲的唯一收口是 `Play`**，自动续播也走同一条路。

- **位置**：仍由 daemon 的墙钟推导（`Instant` 补偿暂停），跨 IPC 后精度不变；客户端不做本地插值推进。
- **`resume_at`**：从 `state.json` 恢复时把曲目和位置「停驻」在架上而不出声——针停在**声音**上：`AudioEngine::play_file_at(path, offset)` 把偏移同时交给时钟和 `spawn_decoder`，只拨时钟会让显示与扬声器整首歌都对不上。
- **曲库（`library.rs`）**：`LibraryDb`（SQLite + FTS5）+ 扫描任务登记 + `library.json`。扫描的剪枝只在**完整走完**时进行（`complete` 标志）——一次部分遍历（有子目录读不了）绝不能删掉索引里的行。
- **歌单（`playlists.rs`）**：身份是 **id**，不是名字（可以有两个都叫 "Mix" 的歌单），也不是位置（位置是调用者手里那份拷贝的光标，而那份拷贝可能刚被另一个客户端改过）。`#[serde(default)]` 让旧文件（全是 0）照常加载并在 load 时补号写回，迁移是一次性的。
- **封面（`cover.rs`）**：`mpris:artUrl` 必须是个 URL，所以封面落盘到 `$XDG_CACHE_HOME/tmper`，文件名按**曲目路径**取而不是按内容（桌面只在元数据变化时取图）。快照里带路径，客户端直接读同一个文件——封面不必过 socket。
- **MPRIS（`mpris.rs`）**：见 §2.3。

### 3.4 客户端句柄 (src/app/handle.rs)

```rust
pub trait PlayerHandle {
    fn dispatch(&mut self, request: Request) -> Vec<Event>;  // 命令
    fn poll(&mut self) -> Vec<Event>;                        // 非阻塞取事件
    fn tick(&mut self) -> Vec<Event>;                        // 推进时钟
    fn connection(&self) -> Connection { Connection::Live }  // 还能不能连上
}
```

**一个 trait，两种投递时机。** `LocalHandle` 直接持有 `Player`、同步应答，**只有测试构建它**；`DaemonHandle` 走 socket，`dispatch` 返回空、事件下一 tick 到货。**时机不同，别的一点都不能不同**——两者把事件交给同一个 `App::apply_event`，所以驱动 `LocalHandle` 的两百来个测试跑的就是 socket 客户端的代码。为测试补一条同步旁路会让绿色变得没有意义。

**`Connection` 是状态而不是事件**（`watch`），这就是它走 channel 而不是走消息的原因：toast 会超时，「播放器没了」不会因为过了十秒就不再是事实。横幅画到它不再说 `Lost` 为止。

`DaemonHandle` 自带监督：socket 断掉时置 `Lost`、把待发请求留在 outbox（连接恢复后按顺序送达，**按键的含义就是按下时的含义**）、反复拨号重连，并在每次断线期间**最多**用自己的 `current_exe()` 拉起一个 daemon（一次断线一次，崩溃循环不会变成 fork 循环）。`serve` 里唯一作为 `select!` 分支的是请求转发（对 channel 的 `send` 被取消是安全的），读 socket 永远在自己的任务里。

### 3.5 音频引擎 (src/audio/)

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

**关键实现**：`stop_and_replace()` — 换会话时**只替换、绝不 `stop()`**。旧 Sink 由「最后一个 `Arc` 消失」退休：rodio 的 `stop()` 只置一个标志位（`sink.rs:312`），而**下一次** `append` 落在「已置位且 `sound_count > 0`」的 Sink 上时会调 `sleep_until_end`（`sink.rs:111-114`），阻塞等待一个「声音结束」信号——在一个没人轮询的 Sink 上（headless、或卡住的设备）这个信号永远不会来。处于两次 `append` 之间的解码线程就此永久停在 rodio 里，而 `Runtime::drop` 会一直等这个 `spawn_blocking` 任务，于是整个进程（测试时是整个测试套件）间歇性挂死。丢引用触发 `Drop` 的静音机制与 `stop()` 完全相同（置 `stopped` + 清 `keep_alive_if_empty`，`sink.rs:356-365`），区别只是标志位落下的**时刻**推迟到解码线程下一次取消轮询（10ms / 收尾时 20ms），且**不可能**落在一次进行中的 `append` 下面。见 `progress/2026-10-03-sink-retirement.md`。

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

**位置追踪**：使用壁钟时间（`Instant`）并补偿暂停时间；解码器发出 `Ready` 后才重置会话计时，加载时间不计入播放位置。

**有界流式解码**：后台线程按实际未播放 PCM 样本数实施约 2 秒的高水位背压，不再把整首音频预先排入 Rodio。每次播放/seek 都使用独立 Sink、取消令牌和递增 generation，旧会话无法污染新会话。

**生命周期事件**：后台解码器向主线程发送 `Ready`、`Finished`、`Failed`；自动切歌以 Sink 真正排空的 `Finished` 为准，不再使用“壁钟位置达到标签时长”推断结束。

**seek**：`seek_relative(secs)` 使用容器原生 seek 并在后台重新建立流式会话；暂停状态跨 seek 保留。

**InstrumentedSource**：包装 Rodio Source，在 `next()` 中拷贝采样到 FFT ring buffer，并精确递减排队样本数；被取消时 `Drop` 释放尚未播放的计数。

**测试**：覆盖生命周期、背压上限、显式完成、快速替换会话、后台错误传播和暂停中 seek。

---

### 3.6 元数据读取 (src/metadata/reader.rs)

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

### 3.7 歌词系统 (src/lyrics/)

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

### 3.8 频谱可视化 (src/visualizer/)

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

### 3.9 音乐库 (src/library/)

#### database.rs — SQLite 索引

```rust
pub struct LibraryDb { conn: Connection }
pub struct TrackRow { /* 对应 tracks 主表全部字段 */ }
```

**Schema**：`tracks` 主表 + external-content `tracks_fts` FTS5 虚拟表；insert/update/delete 触发器保持全文索引同步，`PRAGMA user_version` 管理版本。

**操作**：`upsert()`、`get_by_path()`、`search()`（FTS5 前缀全文搜索）、艺术家/专辑查询、文件指纹快照、删除目录下已消失条目。

**测试**：覆盖 upsert、FTS 更新/删除同步、搜索、分组查询和缺失文件清理。

#### scanner.rs — 后台增量目录扫描

通过 `WalkDir` 递归发现音频文件，默认不跟随符号链接。扫描线程对比 `file_size + mtime` 指纹，只解析新增或变化文件的元数据，通过 Tokio channel 将结果交回主线程写入 SQLite；支持进度、取消以及扫描完成后的缺失文件清理。

**测试**：扩展名过滤验证（1 个）。

#### playlist_manager.rs — M3U 导入导出

- `import_m3u()` — 解析 EXTINF 标签，相对路径解析
- `export_m3u()` — 写入扩展 M3U 格式

**测试**：往返测试、相对路径测试。

---

### 3.10 CLI (src/cli.rs)

```rust
#[derive(Subcommand, Debug)]
pub enum Command {
    Play { file: Option<PathBuf> },   // 带文件=开界面播放；不带=接着放
    Pause, Next, Prev, Stop,
    Volume { percent: u8 },
    Status, Quit,
    Daemon,                           // 前台跑播放器，排障用
}

impl Command {
    pub fn controls_a_running_player(&self) -> bool { … }
}
```

支持：`tmper`（交互模式）、`tmper play <path>`（播放**单个文件**；目录播放暂未实现）、`tmper play`（接着放）、`pause` / `next` / `prev` / `stop` / `volume <0-100>` / `status` / `quit`（发给正在运行的播放器）、`tmper daemon`（前台运行）。

**一次性动词不启动 daemon。** 没东西在放的时候 `tmper pause` 应该说这句话，而不是悄悄起一个播放器好把它暂停。这条边界就是 `controls_a_running_player()`。

---

### 3.11 错误处理 (src/error.rs)

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
TUI 进程                                     daemon 进程
────────                                     ────────────
1. Enter → handle_key_event → play_selected(path)
      │
2. DaemonHandle::dispatch(Request::Play{path})
      │
      ├─ 序列化成一行 JSON ──────── socket ──────► 连接任务读到
      │                                             │
      │                                   3. Player::execute(Play)
      │                                      ├─ 不在队列里就入队
      │                                      ├─ AudioEngine::play_file(path)
      │                                      │    ├─ AudioDecoder::open(path)
      │                                      │    │    └─ Symphonia 探测+建解码器
      │                                      │    ├─ spawn_blocking 解码，
      │                                      │    │   按 ~2s PCM 高水位背压
      │                                      │    └─ InstrumentedSource::next()
      │                                      │        拷贝采样到 pcm_buffer
      │                                      └─ 产生 Snapshot 事件
      │                                             │
      ◄───────────── Event::Snapshot / Queue ───────┘
      │
4. App::apply_event：更新 UiState（标题、时长、队列、封面路径）
      │
5. 每 tick（~33ms）：
      ├─ DaemonHandle::poll() 取回该 tick 的新事件 → 同一个 apply_event
      ├─ Snapshot.position_secs → 进度条
      ├─ Snapshot.cover_path → 读缓存文件 → 封面渲染
      ├─ LyricEngine::sync(position) → 当前歌词行
      └─ 若订阅了频谱，Event::Visualizer.bars → UiState.visualizer_data
      │
daemon 侧同 tick：
      ├─ engine.position_secs() → 填进快照
      ├─ 检测 Sink 真正排空 → Finished → 按 RepeatMode 自动切歌（同样走 Play）
      └─ route(Snapshot)：客户端 + MPRIS 各收一份
      │
6. UI 渲染 (每次事件后)：
      └─ terminal.draw(|f| ui::render(f, &app))
           └─ 根据 active_view 分发到对应 view_render 函数
```

**没有队列变化就没有队列消息。** 队列、歌单、曲库路径只在变化时推；快照每 tick 都推。客户端因此可能比 daemon 晚一个 tick 知道队列变了——所以按位置寻址的命令（删第 n 首）在协议里是按**路径**寻址的（队列里路径唯一，位置会 stale 一格）。

### 4.2 状态管理原则

- **单一写入者（进程内）**：`UiState` 只在 `handle_event` / `apply_event` 中修改
- **单一写入者（进程间）**：`state.json`、`library.db`、`playlists.json`、`library.json` 各自只有 daemon 写；客户端经 IPC 请求改动。这是结构保证，不是约定
- **只读渲染**：UI 渲染函数接收 `&UiState`，不做修改
- **跨进程没有共享内存**：FFT 数据经 `Event::Visualizer` 过 socket（**仅在客户端订阅时**——没开可视器就不算 FFT、不占带宽）；位置经 `Event::Snapshot`
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
    pub player: PlayerCore,              // 镜像：曲目信息、position/duration、tracks、选中、滚动
    pub volume: f32,
    pub repeat_mode: RepeatMode,
    pub lyrics: LyricsState,             // 歌词：lyric_track、current_lyric_index、lyrics_offset_ms
    pub visualizer_data: Vec<f32>,       // 最近一帧频谱（来自 Event::Visualizer）
    pub view: ViewState,                 // 当前视图 + 帮助标志（active_view、show_help）
    pub playlist_state: PlaylistManagerState,
    pub file_browser_state: FileBrowserState,
    pub library_state: LibraryState,
    pub settings_state: SettingsState,
    pub active_playlist: Option<u64>,    // 打开的歌单，**按 id**（位置会 stale，名字会重名）
    pub command_mode: bool,
    pub command_buffer: String,
    pub search_mode: bool,               // 全局 / 搜索（播放器队列实时过滤）
    pub search_query: String,
    pub notification: Option<(String, std::time::Instant)>,  // 瞬时提示，会过期
    pub connection: Connection,          // 持久状态：连接还在不在（断了就画横幅）
    pub visible_rows: Cell<usize>,
    pub cover_rect: Cell<(u16, u16, u16, u16)>,
    pub cell_px: Cell<(u16, u16)>,       // 终端单元格像素，两个封面图层共用
    pub native_cover: Cell<bool>,        // 原生图像在位时半块字符让位
}
```

`connection` 与 `notification` 并排放着，正是为了对照：**通知是「发生了什么」，连接是「现在是什么」**。所以前者会超时消失，后者是 `watch` 上的一个状态，说到它不再为真为止。

`active_playlist` 从 `Option<usize>` 变成 `Option<u64>`（id）之后，`active_playlist_song` 与 `playlist_name` 一起消失了：前者是「播放在哪」的第二份拷贝，写在 A 视图、读在 B 视图，每次编辑歌单就 stale 一次；后者写了两处、从来没有读过。位置现在由 daemon 从当前曲目和它正在走的那个列表推导。

### 5.3 主题系统

`src/ui/theme.rs` 定义了一个 **13 色槽语义化 `Theme` 结构体**（另含 `name` 字段），
优先从 `$XDG_CONFIG_HOME/tmper/themes/<name>.toml` 加载，并回退到二进制内嵌的 5 套配色。
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
    // 1. TerminalGuard::enter()：raw mode + alternate screen
    // 2. probe_terminal_once()：一趟往返问单元格尺寸 + 图形能力
    //    （必须在 enter 之后、输入线程之前：它读的是 TUI 自己的 tty）
    // 3. 起 burst 输入线程（poll/read → unbounded_channel）
    // 4. 事件循环：
    loop {
        tokio::select! {
            Some(batch) = event_rx.recv() => {
                // burst 模式：整批 Vec<CrosstermEvent> 统一处理
                // 逐事件 → KeyHandler → handle_key_event()
                // 搜索/插入模式绕过 KeyHandler 双键延迟
            }
            _ = tick_interval.tick() => {
                self.handle_event(AppEvent::Tick);   // → handle_tick
            }
        }
        terminal.draw(|f| ui::render(f, &self))?;
        if self.should_quit { break; }
    }
    // 5. player.detach()（socket 实现什么都不做：音乐在别的进程里）
    //    恢复终端
}
```

`handle_tick` 每 tick 做四件事，顺序有意：`player.tick()` 与 `player.poll()` 取事件 → **读一次 `player.connection()`** → `apply_event` 应用事件 → 歌词同步。

**连接状态在应用事件之前读**，这样「一个快照」和「送来这个快照的连接没了」不会以相反的顺序被报告。它不属于事件流：横幅要一直挂到它不再为真，而事件没有「之后」可以忘掉。

**daemon 侧一次 tick 做的一件事**是 `Player::tick()`：推进引擎的时钟、把 `Ready`/`Finished`/`Failed` 变成事件、按循环策略续播、填一份新快照。`route` 再把它按订阅关系分发给客户端，并顺手交给 MPRIS。

---

## 7. 配置系统

### 7.1 文件位置

配置和运行时数据遵循 XDG：配置、数据、状态、运行时、缓存分别位于对应的 XDG 目录；测试可通过 `TMPER_*_DIR` 覆盖（`cfg(test)` 下全部落到每个进程自己的测试根）。

| 文件 | 写入者 | 用途 |
|------|--------|------|
| 内嵌 `config/default.toml` | — | 默认配置模板（编译进二进制） |
| `$XDG_CONFIG_HOME/tmper/config.toml` | **客户端** | 主配置（首次运行由内嵌模板生成） |
| `$XDG_CONFIG_HOME/tmper/keybindings.toml` | **客户端** | 自定义快捷键（可选） |
| `$XDG_STATE_HOME/tmper/state.json` | **daemon** | 音量/循环/队列/位置/歌词偏移（客户端用 `SetLyricsOffset` 请它代写） |
| `$XDG_STATE_HOME/tmper/playlists.json` | **daemon** | 歌单（带 id，一次性迁移旧文件） |
| `$XDG_STATE_HOME/tmper/library.json` | **daemon** | 曲库目录列表 |
| `$XDG_DATA_HOME/tmper/library.db` | **daemon** | 曲库 SQLite + FTS5 数据库 |
| `$XDG_DATA_HOME/tmper/{name}.m3u` | **daemon** | 导出的歌单 |
| `$XDG_CACHE_HOME/tmper/cover.*.png` | **daemon** | 封面缓存（供 `mpris:artUrl`，保留最新 8 张） |
| `$XDG_RUNTIME_DIR/tmper/socket` | **daemon** | IPC 端点，权限 0600 |
| `$XDG_STATE_HOME/tmper/tmper.log` | **客户端** | 界面日志（含图形探测结论） |
| `$XDG_STATE_HOME/tmper/tmper-daemon.log` | **daemon** | 播放器日志 |

**为什么日志要分两个文件**：`init_logging` 用 `File::create`，每次启动都会**截断**。两个进程共写一个文件，daemon 的历史——包括它临死前解释原因的那几行——会被下一次 `tmper` 抹掉。daemon 因此写自己的 `tmper-daemon.log`。README 让用户 `grep "graphics"` 的图形探测结论在客户端那一个里，位置不变。

### 7.2 配置结构

```rust
pub struct Config {
    pub playback: PlaybackConfig,     // default_volume, seek_step_small_secs
    pub visualizer: VisualizerConfig, // num_bars, frame_rate, smoothing
    pub ui: UiConfig,                 // theme, show_cover_art, cell_px
}
```

> `[library]` / `[lyrics]` 配置段及 `gapless`、`resume_on_startup`、`color_scheme` 等键均已移除，仅保留以上 8 个键。`cell_px` 是可选覆盖（`Option<(u16, u16)>`，默认 `None`），只在终端既不上报 `TIOCGWINSZ` 像素字段也不回答 `CSI 16 t` 时才有意义——见 §9.3 第 6 条。

**加载**：首次运行由内嵌模板生成 XDG `config.toml`；随后读取并解析，失败则使用 `Default::default()`。旧项目目录数据只复制迁移，不删除源文件。

**保存**：`write_config()` — `toml::to_string_pretty(&config)` → 写入文件（通过设置视图自动触发）

**配置归客户端，但它不全归客户端用**：`num_bars` 与 `smoothing` 存在客户端的 `config.toml` 里，消耗它们的是 daemon 的 FFT 线程。客户端用 `Request::SetFftParams` 把它们推过去（变更时再推一次），而不是让 daemon 也去读那个文件——配置文件只有一个读者，规则才简单。`default_volume` 同理，在 `Hello` 时下发。

### 7.3 配置优先级

```
设置视图在线修改 > XDG config.toml > 内嵌默认值
```

---

## 8. 持久化

**一个文件一个写入者**，见表 §7.1。客户端不再自己写任何文件：`app/persistence.rs` 整个删除了，歌单、曲库路径、state 的读写都变成请求。

### 8.1 状态保存（daemon）

`state.json` 在三种时刻由 daemon 写出：收到 `Shutdown`、空闲退出、以及收到 `Shutdown` 之前的 `stop` 收尾。

```json
{
  "volume": 0.8,
  "repeat_mode": "Sequential",
  "lyrics_offset_ms": 0,
  "last_track_path": "/home/user/Music/song.flac",
  "queue": ["/home/user/Music/song.flac"],
  "queue_index": 0,
  "position_secs": 42.5
}
```

> 启动时 `load_state()` 恢复音量、循环模式、歌词偏移，**并把队列和位置停驻在架上**——曲目已经加载、进度条停在那一秒，但**不出声**，按播放才接着放。只存 `last_track_path` 是不够的：那只能让 `play` 从头再放一遍。恢复再叠一层保护：偏移夹到 `duration * 0.999`，正好落在最后一个采样上的 seek 会立刻结束、下一 tick 就换歌。

**存盘顺序是承重的。** `Player::shutdown()` 先 `save_state()` 再静音设备：`stop_playback()` 是回卷，先静音就会把 `position_secs: 0.0` 忠实地写进去。测试 `a_shutdown_saves_the_position_before_it_silences_the_player` 盯着这个顺序。

### 8.2 播放列表持久化（daemon）

`playlists.json` — 歌单 `{ id, name, songs: [路径] }`。**id 是身份**：名字可以重复，位置是别人手里那份拷贝的光标。旧文件（无 `id`，全是 0）在 load 时补号并立刻写回，所以迁移是一次性的。

### 8.3 曲库路径持久化（daemon）

`library.json` — 曲库目录列表，整体作为 `Event::LibraryPaths` 推给每个新客户端。

---

## 9. 键盘处理

### 9.1 处理流程

```
crossterm KeyEvent
    ├─ 文本输入模式（命令/队列搜索/曲库搜索/歌单重命名）
    │     → 绕过 KeyHandler，直接 handle_key_event()（累积字符）
    └─ 其余
          → KeyHandler (双键序列检测: gg, dd) → handle_key_event()
                ├─ 视图切换键 (1-7, 8)
                ├─ Esc (清除搜索/帮助)
                ├─ 视图专用按键 (Playlists/Browser/Library/Settings)
                └─ 全局按键 (Space, n/p, j/k, Enter, ...)
```

**文本输入模式必须绕过 `KeyHandler`**（`App::in_text_entry`，见 `handlers/mod.rs`）。`KeyHandler`
既解析 `dd` 又持有退出键，若在打字时仍然生效，搜索框里输入 `dd` 会删掉当前曲目、输入 `q`
会退出程序、含 `g`/`d` 的查询会丢字符。绕过时同时丢弃挂起的前缀键，避免它稍后冒出来。

### 9.2 双键序列

`KeyHandler` 把裸 `g` / `d` 暂存一个按键的时间，因为二者都可能开启序列（`gg` → JumpTop,
`dd` → RemoveSelected）。不变式是**每个按键都恰好按顺序投递一次**，因此挂起键有两条释放途径：

1. 下一个按键到达 —— 能组成序列就组成，否则挂起键先于当前键投递；
2. 窗口（200ms）过期后由 `flush_expired()` 释放 —— 事件循环每个 tick 调用，所以单个 `g`/`d`
   不会一直挂着等下一个按键。

只有**不带修饰键**的 `g`/`d` 才开启序列：`Ctrl+d`（半页滚动）若被当作 `d` 前缀，连按两次就会
变成 `dd` 并删除曲目。窗口过期后，挂起键不再与后续按键配对。

### 9.3 Cover 渲染与输入隔离

SIXEL/Kitty 封面数据直接写入 stdout（绕过 ratatui 差分缓冲），Konsole 等终端可能把转义字节误读为 stdin 产生虚假按键。2026-08-03 起的设计（详见 `progress/2026-08-03-cover-refactor.md`）：

1. **一次性发送**：SIXEL/Kitty 是持久图形层——发送后不随 ratatui 重绘消失。封面只在变化时发送一次（换歌、改渲染区域、切回播放器视图），不再每帧重发。**每帧重发是伪按键与卡顿的历史根因**（见 `progress/2026-08-01-cover-rollback.md`）。
2. **协议互斥**：Kitty 与 SIXEL 按终端能力检测二选一，避免两者同时写入争抢同一区域。Kitty 载荷有三条硬性要求，都属于「载荷看着正常、终端一声不响地丢掉」的类型（实测见 `progress/2026-10-03-terminal-graphics-probe.md`）：`f=100` 声明的格式是 **PNG**，载荷就必须是 PNG（曾经发的是裸 RGB，尺寸像素都对，Konsole 按 PNG 解不开就整张丢弃）；`p` 是 **placement id** 而不是像素坐标，定位必须用 `CSI <row>;<col> H` 先把光标移到封面矩形；超过一条转义序列的载荷只有**第一块**带完整命令头，后续块是 `ESC _ G m=<more>;<data> ESC \`（每块重复命令头 = N 张被截断的图，Konsole 对每张回 `ENOENT`，而那条 APC 回复会被读成按键）。所有图形命令带 `q=2`（不回复），原因相同。
3. **输入零防御**：不再需要 guard / 帧抑制 / 控制字符过滤等防御层。`handle_key_event` 不拦截任何按键。
4. **清理**：离开播放器视图、隐藏封面、或新曲目无封面时，置 `clear_pending` → 事件循环 `terminal.clear()` 覆盖 SIXEL 残留（Konsole 对 ED 清 SIXEL 的 workaround）。
5. **几何对齐**（2026-10-03，详见 `progress/2026-10-03-cover-aspect-fit.md`、`progress/2026-10-03-chafa-cell-units.md`、`progress/2026-10-03-icy-sixel-encoder.md`）：封面矩形由 `player_view::fit_cover_rect` 按图片**像素**宽高比收缩居中，两个图层共用这一个盒子。SIXEL 载荷由 `IcySixelEncoder` 在进程内编码，**请求的尺寸就是矩形的像素**（`rect 格数 × cell_px`）——chafa 时代这里要先换算成「chafa 的格」：chafa 会自己从 `TIOCGWINSZ` 读单元格（读不到用它自己的 10×20），再把 `--size` 乘回去，于是 tmper 必须用 `chafa_cell_px()` 而不是 `terminal_cell_px()` 去除，除错时整张图按两者之比缩放（Konsole 上封面曾只占盒子的 0.8×0.75）。编码器搬进进程后这个「第二个单元格」不复存在，整类不匹配随之消失。单元格像素每帧经 `terminal_cell_px()` 读取，优先级：启动探测值 → `TIOCGWINSZ` 的 `ws_xpixel`/`ws_ypixel`（ioctl，不写 stdin）→ `FALLBACK_CELL_PX`（10×20）。
6. **终端探测（一次往返，两个答案）**（2026-10-03）：`probe_terminal_once()` 在 `App::run` 里 `TerminalGuard::enter()` 之后、输入线程启动之前**只问一次**——此时 tty 刚进 raw 模式（否则行规程会扣住回复），且没有第二个读者。查询串 = 单元格尺寸（`CSI 16 t`，拿不到则 `CSI 14 t` ÷ `CSI 18 t`）+ **DA1**（`CSI c`），DA1 放在最后：回复按查询顺序返回，且所有终端都答 DA1，因此「收到完整的 DA1 回复」即屏障——它之前的答案都已到达，可以立刻停止等待。等待用 `poll(2)` + 80ms 预算：只看不取，不回答的终端只损失预算、绝不吞按键；迟到的 `CSI 16 t` 回复落在输入线程上只会被 crossterm 丢弃（终字节 `t` 不在其解析表内，`read()` 用 `if let Ok`），迟到的 DA1 回复更安静——crossterm 把它解析成 `InternalEvent::PrimaryDeviceAttributes`，没有对应的公开 `Event`，读线程直接忽略。结果缓存进 `OnceLock`（`terminal_caps()`），并在日志里留下「测到多少 / 支持与否」或「为什么没测到」。单元格尺寸两者都不上报的终端可用 `[ui] cell_px` 手工指定（clamp 丢弃非单元格值）。

   **图形能力判定（两个协议，一次往返）**：同一趟探测里先发 Kitty 图形协议自己的能力查询（`ESC _ G i=31,s=1,v=1,a=q,t=d,f=24;AAAA ESC \`），答 `OK` 即支持；DA1 的参数里含 `4`（xterm 的「Sixel graphics」属性）则支持 SIXEL。二者都**只认终端的明确回答**，选择顺序 Kitty → SIXEL → 半块字符（真彩 vs 256 色寄存器）。Konsole 26.08 回 `ESC _ G i=31;OK ESC \` 与 `CSI ? 62 ; 1 ; 4 c`（它就是靠这条才被判成 Kitty 终端——它一个相关环境变量都不设，环境变量判据认不出它）；foot 回 `CSI ? 62 ; 4 ; 22 ; 28 ; 52 c`；VTE 系（GNOME Terminal、xfce4-terminal）、Alacritty 两问皆无回答，封面区保持字符画而不是空白（以前只检查 `chafa` 二进制是否存在，于是那些终端上封面区是空白——`native_active` 一置位就把字符画撤了）。DA1 参数按**数值**匹配，「24」不会被当成「4」。

   **复用器（`TMUX` / `STY` / `ZELLIJ`）**：检测到就不发图形查询、也不采信图形回答——复用器默认吞掉 DCS/APC 载荷，而查询回答可能仍来自底下的真终端，「回答支持 + 载荷被吞」正是空白面板的组合。passthrough 包装（`ESC Ptmux;…`）需要 tmux 3.4+ 且 `allow-passthrough on`，属于用户侧配置，探测不到，因此不做，直接退半块字符。
7. **字符画让位**：原生图像生效时（`CoverRenderer::native_active`）用 `Clear` 抹掉封面矩形内的半块字符，而不是不画——`Block` 只重置样式，保留的 `▄▀` 会以默认色露出成像素块。因写入单元格会擦除下层图形层，`blocks_suppressed` 并入 SIXEL 缓存键，在被改写的那一帧重发一次。
8. 渲染器经注入式 writer + encoder 可测试，探测的读循环经注入式 fd 在管道上可测试（`query_terminal_on`），共 43 个单元测试锁定（`src/ui/cover/mod.rs`）。注意单元测试只能断言「字节发对了」，断言不了「终端接受了」——载荷格式类的问题（见第 2 条）是靠「抓流 + 回放进真终端」发现的。

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

### 10.1 测试分层与分布

#### 测试分布（按测试数）

> 下表统计的是**测试用例数量**，不是**行覆盖率**。行覆盖率需用 `cargo llvm-cov` 单独测量
> （见 [10.3 行覆盖率](#103-行覆盖率)）。
> **现状（2026-10-03 实测）**：总行覆盖率 **89.96%**（函数 88.63%、
> 区域 90.61%；少数计时敏感测试会让该数字每次浮动 ~0.3%）。同日先是把测试从
> 80.09% 补到 88.87%（补齐 `input/handler.rs`、`library/scanner.rs`、`config.rs` 等此前零测试的
> 模块），随后守护进程拆分新增的模块——`ipc/`（`mod.rs` 99%、`proto.rs` 100%）、`daemon.rs`、
> `player/*`、`app/handle.rs`、`client.rs`——在**继续涨**的情况下把总数推到
> 89.96%。拆分新增的代码里覆盖率最低的是 `player/mpris.rs`
> （82.22%：zbus 的接口层需要一条真总线才能跑到，能测的映射/diff/镜像逻辑都测了）。
>
> 剩余的未覆盖部分是**结构性**的，不是遗漏：
> `audio/engine.rs` 77.25%（6 个 `#[ignore]` 设备测试的函数体本身计入未覆盖，
> 另有 `new`/`play_file` 需要真实声卡）、`audio/output.rs` 65.15%（`new` 要开真实设备，
> headless 路径已覆盖）、`app/mod.rs` 70.02%（`TerminalGuard` 与 `run` 事件循环需要
> 真实 tty）、`paths.rs` 53.38%（非 `cfg(test)` 分支在测试构建下根本不参与编译）、
> `main.rs` 0%（二进制入口）。**测试数不是覆盖率**——要说「覆盖了」就跑
> `cargo llvm-cov --all-features --workspace`。

| 模块 | 测试数 | 覆盖内容 |
|------|--------|----------|
| ipc/proto.rs | 6 | 每个 `Request`/`Event` 变体的 serde 往返（新增变体没有 serde 形状会在这里红，而不是在 socket 那一端红）、浮点精确过网、tag 拼写被钉住、未知字段忽略而未知 tag 报错 |
| ipc/mod.rs | 11 | 行分隔封帧：往返、载荷内的换行、超过读缓冲的长行重组、第二行等它自己那次读、EOF 在行间是干净的结束而在行中是错误、坏 JSON、超长行不缓冲就先拒绝、CRLF 容忍、空行、非 UTF-8 |
| daemon.rs | 21 | 新客户端一次拿齐镜像（快照+队列+路径+歌单）、客户端 id 不复用、命令广播到每个客户端、离场客户端的命令被忽略、频谱只发给订阅者、FFT 跟随最后一个订阅者、卡住的客户端从不被等待、错过事实的客户端被丢弃、有人连着或出声就不空闲、暂停也算空闲、空闲计时被打断后重来、`Shutdown` 向所有人道别并结束循环、Hello 握手与版本拒绝、静默连接从不注册 |
| client.rs | 6 | `tmper status` 的输出（播放中/空载/失败各一条）、每个控制动词都有对应的 `Request`、越界音量被夹住而不是照做 |
| player/mod.rs | 37 | 队列与续播策略：播放即入队且去重、队列版本只在队列真变了时动、`Toggle`/`Stop`/`Resume`/seek 语义、删曲目时 `playing_index` 的移动、循环三模式、活动歌单优先于队列且两端环绕、shuffle 落在活动列表内、订阅才产频谱、曲库请求与回带 key 的答复（艺术家/专辑/曲目/搜索）、加路径与索引队列、扫描事件、`Shutdown` 只报一次 |
| player/library.rs | 14 | 艺术家→专辑→曲目查询、前缀搜索、忘掉目录/单文件删行、**不完整或取消的扫描不剪枝**而完整扫描才剪枝、扫描索引目录并报告完成、路径列表往返并丢掉已消失的、同一路径加两次只留一条 |
| player/playlists.rs | 12 | id 随歌单往返、载入时丢弃已不存在的歌、无 id 的文件补号并写回、重复 id 重新编号、同曲去重、按位置删曲、对不存在的 id 编辑是 no-op、**id 永不复用**、导入成为编号歌单、读不了的导入会报告、不指名歌曲的歌单仍是歌单、损坏文件被忽略 |
| player/persistence.rs | 11 | state 往返与部分字段恢复、缺失/损坏文件、恢复后停在同曲同一秒、`resume` 从停驻处继续、`stop` 之后停驻位置不再生效、存盘记下队列与实时位置、**shutdown 先存盘后静音**（把顺序调回去这条会红）、越界 `queue_index` 被丢弃 |
| player/cover.rs | 8 | PNG/JPEG 按扩展名落盘、未知格式不缓存、同曲复用同一文件、不同曲不同文件、缓存不随听歌历史增长、指纹稳定且互异、非 UTF-8 路径也有名字 |
| player/fft.rs | 3 | 取最新样本窗口、样本不够时是 `None`、环形缓冲满了仍取尾部 |
| player/mpris.rs | 25 | 首份快照播报全量而后续只播报动了的属性、位置移动不算变化、Loading/Seeking 对桌面是 Playing、循环模式映射到两个 flag、曲目变化的元数据、空载元数据、无时长不可 seek、传输方法发的就是按键发的那些请求、秒与分数过网、音量越界被夹、`SetPosition` 变成相对 seek、未加载曲目/空载的 `SetPosition` 无操作、getter 读的是 daemon 发布的值、**循环模式经总线往返**、清掉从未设过的 shuffle 不带走单曲循环、无封面就没有 `artUrl`、恢复的会话是「还没有封面的曲目」、track id 由路径拼出、`file://` 转义、时间是微秒、一次 `Set` 不播报两次、音量经加宽仍往返 |
| app/handle.rs | 11 | `LocalHandle` 同步应答、`poll` 恒空、`tick` 报出引擎动过之后的状态、`detach` 停播、只有本地句柄拿得到 player；`DaemonHandle`：**欢迎词后面的问候不被吞**、握手前就走的 daemon 被报告、socket 中途死掉终结一次性动词、daemon 死了竖横幅并接回来、故障期间发出的命令在恢复后到达、永不回来的 daemon 也不拖垮客户端 |
| app/mod.rs | 14 | 视图切换、音量、循环、加载播放、停止、命令模式、搜索、文本输入模式旁路集合（含 4 个 tokio 集成式） |
| app/playback.rs | 8 | 播放选中项、越界是 no-op、选择移动的两端 clamp 与滚动跟随、空队列 no-op、换曲清掉上一首的歌词、歌词偏移生效 |
| app/handlers/mod.rs | 42 | 键位匹配、视图分发切换、滚动 clamp、命令分发全分支（quit/theme/volume/seek/shuffle/view/import/export 及失败路径）、`AppEvent` 分发、帮助覆盖层按键、tick 的频谱衰减与配置同步、tick 读取连接状态、test_support 辅助 |
| app/handlers/browser.rs | 16 | 焦点切换、库/文件系统导航与 clamp、Enter 进入目录/加库去重、混合目录（子目录+音频）选中行不串位、Backspace 边界、刷新过滤排序 |
| app/handlers/library.rs | 22 | 面板导航、搜索输入/回车/回退、`clamp_scroll`、答复到达时替换列表并夹住光标、加目录请求扫描、加文件只排队不声称扫描、Enter 播放、队列被顺带索引 |
| app/handlers/playlist.rs | 18 | 焦点切换、新建歌单插入模式、展开/删除/重复保护、M3U 导出、flat-model 解析、clamp |
| app/handlers/settings.rs | 16 | 布局 19 行、j/k 导航 clamp、主题/柱数/平滑/音量/步长/封面循环、跳过行、Enter 动作、M3U 导出、config 持久化（写同一个 `config.toml` 的测试用 `paths::config_file_lock()` 串行化：每个 cycle 都会落盘，读回校验的那个测试会被并发的写入者灌进别人的配置） |
| library/database.rs | 10 | upsert、重复更新、搜索、artists、albums、delete、前缀精确匹配、`delete_missing_under` 不误伤同前缀兄弟目录 |
| library/scanner.rs | 10 | 扩展名过滤、遍历错误使列表不完整（缺失根、不可读子目录）、指纹命中跳过与变更重读、元数据失败计入 failed 但不影响 complete、取消、进度上报 |
| library/playlist_manager.rs | 2 | M3U 往返、相对路径 |
| audio/decoder.rs | 3 | 解码 WAV、不存在的文件、seek |
| audio/engine.rs | 24 (18+6) | 播放状态机、暂停/seek/完成（headless）、背压上界、会话替换丢弃陈旧事件、打开失败上报、`InstrumentedSource` 环形缓冲与 DoD 释放计数、会话辅助函数、队列诊断；6 个 `#[ignore]` 的需要真实/虚拟声卡 |
| audio/output.rs | 1 | 退休的 sink 仍可 `append`（`stop()` 过的 sink 会让下一个 `append` 卡在 rodio 的 `sleep_until_end` 里，而持有它的解码线程会把整个运行时拖住不退出） |
| lyrics/parser.rs | 16 | 标准 LRC、元数据、多时间戳、逐字、空文件、损坏行、排序、BOM/UTF-8/GBK/Shift-JIS 编码检测、文件读取 |
| metadata/reader.rs | 3 | FLAC、WAV（无标签）、不存在的文件 |
| visualizer/fft.rs | 1 | 440Hz 峰值检测 |
| visualizer/processor.rs | 2 | 桶数量、平滑收敛 |
| visualizer/render.rs | 6 | 渲染输出、颜色渐变、行宽恰为 width（含 num_bars > width）、`bar_at_column` 跨过居中留白、绘制与配色逐列对齐 |
| ui/mod.rs | 10 | 最小支持尺寸渲染（七个视图）、极窄终端不 panic、通知弹窗绘制与过期、**连接横幅**（双宽字形按格写，所以按字符逐个断言）、播放器还在时不画横幅、命令面板、帮助覆盖层优先级、全屏频谱画出**带颜色的**柱 |
| ui/theme.rs | 5 | hex 颜色解析（有效/无效回退）、多字节输入不 panic、缺失主题回退默认、真实主题 13 色槽加载 |
| ui/views/file_browser_view.rs | 3 | 空/填充渲染、聚焦样式 |
| ui/views/library_view.rs | 6 | 三面板标题、数据行、搜索栏、光标闪烁、长列表滚动保持选中行可见 |
| ui/views/lyrics_view.rs | 4 | 空提示、歌词+当前高亮、offset 标签、滚动保持当前行可见 |
| ui/views/playlist_view.rs | 8 | flat-model 行数/行号/解析、styled lines（展开/输入/播放前缀）、渲染与通知弹出 |
| ui/views/settings_view.rs | 3 | rebuild_settings 布局与配置值、渲染冒烟（含 scroll clamp） |
| ui/views/player_view.rs | 21 | cover 块渲染（空字节/零面积/内存 PNG）、封面矩形自适应（正方形/带余量的一边/宽图/竖图/退化输入/永不越界）、原生图层生效时字符画让位、渲染冒烟、搜索命中与无匹配、迷你歌单、歌词区（空/当前行/跟随滚动）、歌曲信息各槽位、控制栏进度与零时长 |
| ui/cover/mod.rs | 43 | 一次性发送不变量、区域重发、视图切换/隐藏/无封面清除、编码失败不重试、Kitty 尺寸变化重发、搜索覆盖层清理两个协议、载荷尺寸即矩形像素（含退化矩形）、Kitty 载荷三规则（定位用光标移动且 `p` 槽为空、超长载荷只有首块带命令头、`f=100` 的载荷必须真是 PNG）、真实编码器（PNG → SIXEL 栅格头、非图片字节报错）、单元格尺寸解析与回退、DA1 解析（属性 4 / 参数按数值匹配 / 半包不算答复 / 混在其他回复中）、`a=q` 答复解析（含半包与非答复流量）、复用器识别与「复用器下不问图形问题」、探测读循环（管道：应答/半包拼接/DA1 屏障提前返回/超时/EOF）与查询写入、抑制字符画时的重发（注入式 writer/encoder/fd） |
| ui/widgets/help_popup.rs | 4 | 帮助文案与当前键位/XDG 路径一致、绘制、滚动到底后 clamp、小于自身边距的终端 |
| input/command.rs | 5 | `:q` / `:q!` 的区别、theme、volume、unknown |
| input/handler.rs | 15 | 双键序列、非组合键两个按键都按序投递、超时释放挂起键、过期前缀不再配对、Ctrl+D 永不解析为 dd、退出键清理挂起、控制字符过滤 |
| input/keymap.rs | 5 | 单字符/^X/特殊名称、非 ASCII 单字符不再静默变空格、未知名称回退 |
| paths.rs | 5 | XDG 目录拼接、测试期重定向到临时根、运行时目录的优先级（覆盖变量 → XDG_RUNTIME_DIR → 状态目录下的 `run`）、socket 落在运行时目录里、`ensure_runtime_dir` 幂等；另有测试用的 `config_file_lock()`（不是测试，是给共用 `config.toml` 的测试串行化的锁） |
| config.rs | 11 | 模板与代码默认值一致、clamp 上下界与放行、`cell_px` 覆盖的读取与校验、f32 两位小数序列化、往返、部分/空/含未知键的文档解析 |
| playlist.rs | — | （仅存 `PlaylistData` 数据模型，逻辑在 `player/playlists.rs`） |
| **总计** | **528** | **521 默认运行 + 7 忽略（6 个设备门控 + 1 个生成 README 截图的工具）** |

#### 测试分层

| 层级 | 环境与范围 |
|------|------------|
| 纯逻辑 | 歌词、命令解析、FFT、频谱处理、路径和扫描过滤，不访问音频设备 |
| 数据库 | 使用临时的 SQLite `:memory:` 数据库，覆盖 schema、FTS5 同步和增量清理 |
| 解码/元数据 | 只读取 `tests/fixtures/`，不创建 Rodio 输出流 |
| UI 渲染 | 使用 Ratatui `TestBackend`，覆盖极小终端与全部主视图 |
| 播放状态机 | 使用 headless output 和可注入 fake decoder 事件，验证切歌 generation、暂停、seek、完成，无需设备 |
| 音频输出 | 测试名统一以 `audio_output_` 开头并标记 `#[ignore]`，需要真实设备或虚拟 ALSA |

默认测试集合不会打开 ALSA/PulseAudio。音频输出层单独运行，避免让普通开发和 CI 的逻辑测试依赖声卡。

### 10.2 运行测试

```bash
cargo test                                      # 全部无设备测试；自动跳过音频输出层
cargo test <test_name>                          # 单个无设备测试
cargo test audio_output_ -- --ignored --test-threads=1  # 真实/虚拟音频设备
```

### 10.3 行覆盖率

```bash
# 一次性安装
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --locked

# 测量（输出各模块行覆盖率与总计）
cargo llvm-cov --all-features --workspace
```

> CI `coverage` job 在每次 push 时运行同一测量。无音频设备的环境需先配置 null ALSA
> （`~/.asoundrc`，见 CLAUDE.md），否则构造 AudioEngine 的测试会失败。

---

## 11. 项目结构

```
tmper/
├── Cargo.toml                  # 包名 tmper，Rust 2021 edition
├── DESIGN.md                   # 本文件 — 架构与实现文档
├── README.md                   # 用户手册
├── STATUS.md                   # 当前能力、限制与近期计划
│
├── config/default.toml         # 编译进二进制的默认配置
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
├── src/                        # 源代码（一个 binary，两个角色）
│   ├── main.rs                 #   入口：按动词分流 → daemon::run() / client::run_control() / TUI
│   ├── cli.rs                  #   clap CLI：daemon 与一次性控制动词
│   ├── daemon.rs               #   ★ daemon：socket 监听、每客户端任务、空闲退出、MPRIS 装配
│   ├── client.rs               #   ★ 一次性动词的实现（连上、发一条、打印、退出）
│   ├── config.rs               #   Config 加载/默认值
│   ├── constants.rs            #   运行时调优常量（集中管理 magic numbers）
│   ├── error.rs                #   AppError + AppResult<T>
│   ├── event.rs                #   AppEvent 枚举（客户端内部事件）
│   ├── playlist.rs             #   PlaylistData 数据结构（唯一歌单模型）
│   ├── paths.rs                #   XDG 路径（含 runtime/cache）与旧数据迁移
│   │
│   ├── ipc/                    #   ★ 两个进程之间的唯一契约
│   │   ├── mod.rs              #     行分隔 JSON 封帧：读一行、写一行、超时
│   │   └── proto.rs            #     Request / Event 枚举 + PROTOCOL_VERSION
│   │
│   ├── player/                 #   ★ daemon 的内核（播放真值在这里）
│   │   ├── mod.rs              #     Player：引擎 + 队列 + 续播策略 + 每 tick 的推进
│   │   ├── library.rs          #     曲库索引与扫描任务（面板查询也在这里回答）
│   │   ├── playlists.rs        #     歌单的增删改与编号
│   │   ├── persistence.rs      #     state.json 的读写（daemon 是唯一写入者）
│   │   ├── cover.rs            #     封面缓存文件（供 MPRIS 的 file:// URL）
│   │   ├── fft.rs              #     订阅者的频谱：取 PCM 环形缓冲的最新窗口
│   │   └── mpris.rs            #     MPRIS2 接口（zbus）
│   │
│   ├── app/                    #   客户端：TUI 的应用核心
│   │   ├── mod.rs              #     App struct + run() 事件循环
│   │   ├── handle.rs           #     ★ PlayerHandle trait + LocalHandle（测试）/ DaemonHandle（socket）
│   │   ├── playback.rs         #     选择移动、歌词跟随（播放控制已改为发请求）
│   │   └── handlers/           #     按键事件处理器
│   │       ├── mod.rs          #       handle_event、switch_view、全局按键
│   │       ├── playlist.rs     #       歌单视图按键
│   │       ├── library.rs      #       曲库视图按键（查询改为 IPC 请求）
│   │       ├── browser.rs      #       文件浏览器按键
│   │       └── settings.rs     #       设置视图按键 + write_config
│   │
│   ├── audio/                  #   daemon 私有的音频引擎
│   │   ├── decoder.rs          #     Symphonia 解码适配
│   │   ├── output.rs           #     Rodio Sink 封装
│   │   └── engine.rs           #     AudioEngine: 播放/暂停/seek/位置
│   │
│   ├── metadata/               #   元数据
│   │   └── reader.rs           #     lofty 标签读取 → TrackInfo
│   │
│   ├── lyrics/                 #   歌词系统（客户端：位置是快照里来的）
│   │   ├── types.rs            #     LyricLine、LyricTrack、LyricMetadata
│   │   ├── parser.rs           #     LRC 解析 + 编码检测
│   │   └── engine.rs           #     歌词查找 + 同步
│   │
│   ├── visualizer/             #   频谱可视化（客户端渲染）
│   │   ├── fft.rs              #     FFT 分析 (2048 点 Hann 窗)
│   │   ├── processor.rs        #     对数分桶 + 平滑 + 归一化
│   │   └── render.rs           #     Block Elements 字符渲染
│   │
│   ├── library/                #   音乐库的底层实现（归 daemon 调用）
│   │   ├── database.rs         #     SQLite CRUD + 搜索
│   │   ├── scanner.rs          #     后台增量目录扫描
│   │   └── playlist_manager.rs #     M3U 导入/导出
│   │
│   ├── ui/                     #   用户界面
│   │   ├── mod.rs              #     UiState、ViewMode、render() 入口（含连接横幅）
│   │   ├── theme.rs            #     13 色槽语义主题（themes/*.toml 加载）
│   │   ├── cover/              #     封面图渲染（终端协议直接输出）
│   │   │   └── mod.rs          #       CoverRenderer: 能力探测、Kitty/SIXEL 互斥、一次性发送、几何对齐、43 测试
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
│       ├── keymap.rs           #     KeyBindings 配置
│       └── command.rs          #     : 命令解析器
│
└── tests/
    └── fixtures/               #   测试数据
        ├── test.wav            #     440Hz 正弦波 (2s, 44100Hz, stereo)
        ├── test.flac           #     带完整标签的 FLAC
        └── test_notags.wav     #     无标签 WAV（验证 fallback）
```

★ = 守护进程拆分新增的模块。

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
| 图像 | image | 0.25 | 封面解码 + Lanczos3 缩放（半块字符与 SIXEL 共用） |
| SIXEL | icy_sixel | 0.5 | 进程内 sixel 编码（quantette：Wu 量化 + Floyd–Steinberg 抖动，≤256 色） |
| 配置 | toml | 0.8 | TOML 序列化/反序列化 |
| 序列化 | serde + serde_json | 1 | 配置/状态 JSON；**也是 IPC 的线格式**（行分隔 JSON，无额外协议库） |
| 桌面集成 | mpris-server | 0.10 | MPRIS2 服务端（`tokio` feature，走 zbus 5）；没有会话总线时只是记日志，不拒绝启动 |
| 终端探测 | libc | 0.2 | 单元尺寸探测要 `poll(2)`：等 tty 但不从它读走字节 |
| CLI | clap | 4 | 命令行参数解析 |
| 数据库 | rusqlite | 0.32 | SQLite（bundled） |
| 目录遍历 | walkdir | 2 | 递归目录扫描 |
| 正则 | regex | 1 | LRC 解析正则 |
| 编码 | encoding_rs | 0.8 | GBK/Shift-JIS 歌词编码检测 |
| 日志 | tracing + tracing-subscriber | 0.1/0.3 | 结构化日志（客户端与 daemon 各写各的文件） |
| 错误 | thiserror + anyhow | 2/1 | 错误类型 + 传播 |
| 路径 | dirs | 6 | 家目录与 XDG 兜底 |
| 文本宽度 | unicode-width | 0.2 | 中日韩宽字符的列宽（通知横幅按格写） |
| Base64 | base64 | 0.22 | Kitty 图形载荷编码 |
| 随机 | rand | 0.8 | shuffle 洗牌 |
