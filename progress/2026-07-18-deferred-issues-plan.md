# 遗留问题详细实施计划

> **日期**: 2026-07-18
> **参考**: 2026-07-18 代码审查 §已知技术债
> **状态**: 规划设计阶段，待确认后分批执行

---

## 目录

1. [问题 1: UiState 上帝结构体拆分](#1-uistate-上帝结构体拆分)
2. [问题 2: stdout 直接写入根除](#2-stdout-直接写入根除)
3. [问题 3: play_file 异步化解码](#3-play_file-异步化解码)
4. [问题 4: 集成测试体系](#4-集成测试体系)
5. [执行顺序建议](#5-执行顺序建议)

---

## 1. UiState 上帝结构体拆分

### 1.1 现状分析

`src/ui/mod.rs:54-101` — `UiState` 包含 **36 个字段**，横跨播放、歌词、频谱、封面、视图切换、搜索、命令输入、帮助面板等 8 个职责域。

**影响范围**:
- `src/app/handlers/mod.rs` — 230 处 `self.ui_state.xxx` 引用
- `src/ui/mod.rs` — `render()` 传递 `&UiState` 到 7 个视图
- `src/ui/views/player_view.rs` — 8 个函数接收 `&UiState`，各自只用到其中若干字段
- `src/ui/cover/mod.rs` — `render_kitty()` / `render_chafa()` 接收 `&UiState` 但实际只用 5-6 个字段

**导致的问题**:
- 无法从类型系统判断一个 view 究竟读写哪些状态
- 视图切换时需要手动重置分散的状态位
- 新增字段时编译器不提示哪些 view/handler 需要更新
- 测试困难：无法以隔离方式测试单个视图渲染，必须构造整个 UiState

### 1.2 拆分方案

#### 第一阶段：抽取只读视图参数（低风险 🔵）

将每个视图的渲染参数从 `&UiState` 改为专用的参数结构体：

```rust
// src/ui/views/player_view.rs

pub struct PlayerViewParams<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub position: f64,
    pub duration: f64,
    pub volume: f32,
    pub is_playing: bool,
    pub repeat_mode: RepeatMode,
    pub album: &'a str,
    pub genre: &'a str,
    pub year: &'a str,
    pub codec: &'a str,
    pub cover_art: Option<&'a Arc<Vec<u8>>>,
    pub show_cover_art: bool,
    pub cover_rect: (u16, u16, u16, u16),
    pub lyric_track: Option<&'a LyricTrack>,
    pub current_lyric_index: usize,
    pub visualizer_data: &'a [f32],
    pub playlist_state: &'a PlaylistManagerState,
    pub playing_index: Option<usize>,
    pub tracks: &'a [TrackDisplay],
    pub cover_gen: u64,
}
```

其他视图同理，每个视图 4-6 个字段。

**收益**: render 函数签名字文档化，新增字段时编译器精确报错。不改变任何运行时行为。

#### 第二阶段：抽取可写状态组（中等风险 🟡）

将紧密耦合的状态字段分组：

```rust
// 播放器核心状态（高频读写）
pub struct PlayerCore {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub year: String,
    pub codec: String,
    pub position: f64,
    pub duration: f64,
    pub is_playing: bool,
    pub tracks: Vec<TrackDisplay>,
    pub playing_index: Option<usize>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub cover_art: Option<Arc<Vec<u8>>>,
    pub show_cover_art: bool,
    pub cover_gen: Cell<u64>,
}

// 歌词状态
pub struct LyricsState {
    pub lyric_track: Option<LyricTrack>,
    pub current_lyric_index: usize,
    pub lyrics_offset_ms: i64,
}

// 视图状态
pub struct ViewState {
    pub active_view: ViewMode,
    pub show_help: bool,
    pub help_scroll: usize,
    pub last_help_toggle: Option<std::time::Instant>,
}
```

给每个分组添加 `reset_on_track_change()` 方法，集中管理状态清理：

```rust
impl PlayerCore {
    fn reset_on_track_change(&mut self) {
        self.position = 0.0;
        self.is_playing = false;
    }
}
```

#### UiState 最终形态

```
36 字段 → 13 个分组 + 3 个 Cell
每个分组有明确的职责边界和生命周期。
```

### 1.3 执行步骤

| 步骤 | 内容 | 风险 | 预计 |
|------|------|------|------|
| 1.1 | 为 7 个视图 + CoverRenderer 定义只读参数结构体 | 🟢 | 1h |
| 1.2 | 修改 render 函数签名，从 `&UiState` 到 `&XxxParams` | 🟢 | 30min |
| 1.3 | 在 App 中为每个 render 调用构造参数 | 🟢 | 20min |
| 1.4 | 定义状态分组：PlayerCore, LyricsState, ViewState | 🟡 | 30min |
| 1.5 | 将 UiState 字段迁移到分组，更新所有引用 | 🟡 | 1.5h |
| 1.6 | 为每个分组添加 reset_xxx() 方法 | 🟡 | 30min |
| 1.7 | 用 reset 方法替换 on_track_ended 中的手动重置 | 🟡 | 15min |
| 1.8 | 编译 + 测试 + clippy | — | 15min |

**小计**: ~5 小时，分 2 个 session。

---

## 2. stdout 直接写入根除

### 2.1 现状分析

`src/ui/cover/mod.rs` 中，Kitty 和 chafa SIXEL 渲染绕过 ratatui 差分缓冲，直接写入 `std::io::stdout()`：

- **Kitty** (L159): `write!(std::io::stdout(), "\x1b_G...")`
- **SIXEL** (L290-293): `write!()` + `write_all()` + `flush()`

**当前缓解**（3 层，非根治）:
1. `suppress_frames(5)` — 切歌后 5 帧静默
2. `last_help_toggle` 500ms 冷却
3. KeyHandler 控制字符过滤

**根因**: Kernel PTY 是全双工通道——stdout 写入和 stdin 读取共用同一个 PTY master。部分终端/驱动会将 stdout 转义序列字节误解析为 stdin 事件。

### 2.2 推荐方案：ratatui-image crate

使用 `ratatui-image` 将封面渲染为 ratatui widget，走标准渲染管线：

```rust
// 当前
self.cover_renderer.render_kitty(&self.ui_state);  // 直接写 stdout

// 方案 A
let image_widget = ImageWidget::from_bytes(&cover_data).resize(cover_rect);
f.render_widget(image_widget, cover_rect);  // 走 ratatui 差分缓冲
```

**优点**: 完全消除 stdout 直接写入，利用 ratatui 差分缓冲，自动终端协议检测。
**风险**: 中。需要验证 Sixel 支持。

**备选（如 ratatui-image 不满足需求）**: 改进现有缓解——增大 suppress_frames、动态冷却时间、stdin 时序启发式过滤。

### 2.3 执行步骤

| 步骤 | 内容 | 风险 | 预计 |
|------|------|------|------|
| 2.1 | 调研 `ratatui-image` API 和协议支持矩阵 | 🟢 | 30min |
| 2.2 | 创建 `CoverWidget` 封装 | 🟢 | 45min |
| 2.3 | 在 `render_left_panel()` 中替换当前渲染 | 🟡 | 30min |
| 2.4 | 删除 `CoverRenderer` 中的 stdout 写入 | 🟢 | 15min |
| 2.5 | 移除 `suppress_frames`、`clear_pending` 缓解措施 | 🟢 | 15min |
| 2.6 | 多终端兼容性验证 | — | 30min |

**小计**: ~3 小时。

---

## 3. play_file 异步化解码

### 3.1 现状分析

`src/audio/engine.rs:106-152` — `play_file()` 同步循环解码：

```rust
while let Some(samples) = decoder.read_packet()? {
    self.output.append_source(source);
}
```

长文件（10min+）解码可达数百毫秒，UI 冻结。

### 3.2 改造方案

将解码移到 `spawn_blocking`，通过 `tokio::mpsc` 流式传输：

```
spawn_blocking { decode → send(channel) }
spawn { recv(channel) → append_source }
```

关键改动：
- `output.rs` 中 Sink 需要支持多线程访问（`Arc<Sink>` 或 `SinkHandle`）
- 增加 `DecodeProgress` 和 `cancel_decode()` 机制
- 保留同步路径用于短文件（< 10 MB）

### 3.3 执行步骤

| 步骤 | 内容 | 风险 | 预计 |
|------|------|------|------|
| 3.1 | output.rs 的 Sink 改为 cloneable 句柄 | 🟡 | 30min |
| 3.2 | 添加 `DecodeProgress` / `DecodedPacket` 类型 | 🟢 | 15min |
| 3.3 | 实现 `play_file_async()` | 🟡 | 1h |
| 3.4 | handle_tick 中添加解码状态检查 | 🟢 | 15min |
| 3.5 | 同步短文件路径保留 | 🟢 | 15min |
| 3.6 | 取消机制（切歌时 abort） | 🟡 | 20min |
| 3.7 | 测试：短文件、长文件、快速切歌 | — | 30min |

**小计**: ~3 小时。

---

## 4. 集成测试体系

### 4.1 现状

当前 42 个测试全部为单元测试。缺失：
- 跨视图切换状态一致性
- 事件循环路径（Tick → position → track_ended）
- 键盘事件分发
- 持久化往返

### 4.2 测试架构

```
tests/common/mod.rs   — TestApp harness（headless App + 模拟按键）
tests/integration/     — 集成测试（~15 场景）
tests/e2e/             — E2E 测试（暂缓，~10 关键路径）
```

#### TestApp Harness

```rust
pub struct TestApp {
    app: App,
}

impl TestApp {
    pub fn press_key(&mut self, code: KeyCode) { ... }
    pub fn tick_n(&mut self, n: u32) { ... }
    pub fn play_fixture(&mut self, name: &str) { ... }
    pub fn active_view(&self) -> ViewMode { ... }
    pub fn volume(&self) -> f32 { ... }
}
```

### 4.3 测试优先级矩阵

| 优先级 | 测试场景 | 原因 |
|--------|---------|------|
| P0 | 切歌后音量不变（Bug 2 回归） | 已知 bug |
| P0 | 切歌后帮助页不弹出（Bug 1 回归） | 已知 bug |
| P0 | 视图切换往返（toggle back to player） | 核心功能 |
| P1 | seek 后播放状态保持 | 音频管线 |
| P1 | 循环模式切换 | 播放逻辑 |
| P1 | 无标签文件 title 回退 | 边界条件 |
| P2 | 空歌单操作 | 边界条件 |
| P2 | 命令模式 dispatch | 命令系统 |
| P2 | 歌词偏移持久化往返 | 持久化 |

### 4.4 执行步骤

| 步骤 | 内容 | 风险 | 预计 |
|------|------|------|------|
| 4.1 | 创建 `tests/common/mod.rs` — TestApp | 🟢 | 1h |
| 4.2 | 为 App 添加公共只读方法 | 🟢 | 20min |
| 4.3 | 实现 P0 测试（Bug 回归 + 视图切换） | 🟢 | 45min |
| 4.4 | 实现 P1 测试（seek + 循环 + 边界） | 🟢 | 45min |
| 4.5 | 实现 P2 测试（命令 + 持久化） | 🟡 | 1h |
| 4.6 | 编译 + `cargo test` 验证 | — | 15min |

**小计**: ~4 小时。

---

## 5. 执行顺序建议

```
Session A (推荐先做):  问题 1 第一阶段 — 只读视图参数 (1.1–1.3)
                      风险最低，立即改善可读性，为后续铺路。~2h

Session B (随后):     问题 1 第二阶段 — 状态分组 (1.4–1.8)
                      拆分上帝结构体。~3h

Session C (可并行):   问题 3 — play_file 异步化
                      独立于 UiState 重构。~3h

Session D (依赖 B):   问题 2 — stdout 根除
                      需要 Session B 的 CoverParams。~3h

Session E (最后):     问题 4 — 集成测试
                      依赖 A-D 完成。~4h
```

| 路径 | 预估 |
|------|------|
| 总计 | ~15 小时 |
| Sessions | 5 个 |
| 并行可能 | C 与 A/B 可并行 |
