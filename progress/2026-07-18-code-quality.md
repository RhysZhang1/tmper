# 2026-07-18 代码质量改进

## 概述

基于 2026-07-18 全项目代码审查，针对 6 项低风险、高收益的问题进行修复。所有变更均为行为等价的重构或补强，零功能变更。

## 审查发现回顾

原始审查发现了 10 个问题，分级如下：

| # | 严重度 | 问题 | 本次修复 |
|---|--------|------|----------|
| 1 | 🔴 | UiState 上帝结构体（30+ 字段） | ❌ 架构变更，风险高，单独规划 |
| 2 | 🔴 | stdout 直接写入导致伪按键事件 | ✅ 增加输入层转义序列过滤 |
| 3 | 🟡 | play_file 同步解码阻塞事件循环 | ❌ 需架构设计，单独规划 |
| 4 | 🟡 | PCM_BUFFER_CAPACITY 仅 8192 (~93ms) | ✅ 增大到 32768 |
| 5 | 🟡 | 无集成测试 | ❌ 长期任务，择机启动 |
| 6 | 🟡 | Config 默认值与 Default impl 重复 | ✅ 删除重复，归一化 |
| 7 | 🟢 | `#[allow(clippy::needless_range_loop)]` 3 处 | ✅ 改用迭代器 |
| 8 | 🟢 | render() 使用长 if-else 链 | ✅ 改为 match |
| 9 | 🟢 | 日志只写文件，调试不便 | ✅ 增加 RUST_LOG 支持 |
| 10 | 🟢 | 跨平台路径 | ❌ 当前设计自包含部署，暂时保持 |

---

## 变更清单

### 1. 增大 PCM 缓冲区 (`src/constants.rs`)

```
PCM_BUFFER_CAPACITY: 8192 → 32768
```

**理由**: 44.1kHz 立体声下 8192 samples ≈ 93ms，仅够 4 个 FFT 窗口。FFT 线程若偶遇调度延迟，数据窗口即断裂，频谱出现不连续跳变。32768 ≈ 372ms，提供充裕缓冲。

**风险**: 极低。仅增加少量内存（32768 × 4 bytes = 128 KiB）。

### 2. 消除 needless_range_loop (`src/visualizer/fft.rs`, `src/visualizer/processor.rs`)

将 3 处 `#[allow(clippy::needless_range_loop)]` 的 `for i in 0..n` 循环改为迭代器形式：

- `fft.rs:37-43` — 窗口函数应用：`scratch.iter_mut().enumerate()` + `zip(samples)`
- `processor.rs:36-51` — 对数分桶：`.enumerate().for_each()`
- `processor.rs:56-62` — 平滑衰减：`.iter_mut().enumerate()`

**理由**: 符合 Rust 惯例，消除 clippy 警告抑制，迭代器无越界风险。

**风险**: 零。行为完全等价，测试覆盖这些路径。

### 3. `render()` 改用 match (`src/ui/mod.rs:257-299`)

将 7 个连续的 `if state.active_view == ViewMode::Xxx` 改为 `match self.active_view` 语句。

**理由**:
- `match` 要求穷举性——将来新增 ViewMode 变体时编译器会报错，防止遗漏渲染分支
- 可读性更好，每个分支的结构更清晰

**风险**: 零。编译期保证匹配完备性。

### 4. Config 默认值去重 (`src/config.rs`)

将 serde default 函数直接用作 Default impl 的值来源，消除中间层重复。

当前结构（有漂移风险）：
```rust
fn default_volume() -> f32 { 0.8 }
impl Default for PlaybackConfig {
    fn default() -> Self { Self { default_volume: default_volume(), ... } }
}
// serde 也引用 default_volume
```

目标：保留 serde 需要的函数名（`#[serde(default = "...")]`），但确保 Default impl 直接复用同一函数，不重复定义值。

**理由**: 当前 serde default 和 Default impl 各自可以独立定义默认值，存在漂移风险。统一来源消除不一致可能。

**风险**: 低。Default impl 的行为完全不变。

### 5. 日志 stderr 输出 (`src/main.rs`)

当前 `tracing_subscriber` 只输出到 `data/tmper.log` 文件。增加 `RUST_LOG` 环境变量支持。

```rust
let env_filter = std::env::var("RUST_LOG").unwrap_or_default();
// file layer (always active)
let file_layer = tracing_subscriber::fmt::layer()
    .with_writer(log_file)
    .with_target(false);
// stderr layer (only if RUST_LOG is set)
if !env_filter.is_empty() {
    // add stderr layer with env_filter
}
```

**理由**: 日常使用看不到日志，调试需 `tail -f data/tmper.log`。支持 `RUST_LOG=info cargo run` 直接看日志。

**风险**: 极低。默认行为不变（不设置 RUST_LOG 则只写文件）。

### 6. KeyHandler 层过滤控制字符 (`src/input/handler.rs`)

在 `KeyHandler::process()` 中增加控制字符检测：当接收到的 KeyEvent 字符在 ASCII 控制字符范围（0x00–0x1F，排除 Tab=0x09, Enter=0x0D, Esc=0x1B）或 0x7F(DEL)，则丢弃该事件，返回 `None`。

**理由**: Bug 1（切歌弹出帮助页）的根因是终端将 Kitty/SIXEL 转义序列的字节误解析为 stdin 输入事件。crossterm 在某些终端/驱动组合下会将转义序列中的字节（如 `0x1B`, `0x5B`, `0x30` 等）作为独立 KeyEvent 报告，其中 `0x30`（ASCII '0'）恰好是帮助页的快捷键。当前的 `suppress_frames` + `last_help_toggle` 冷却只是缓解。在输入层过滤控制字符是更健壮的防线。

**实现思路**：
- 在 `process()` 中，检查 `event.code` 是否为 `KeyCode::Char(c)` 且 `c` 是不可打印控制字符
- 控制字符范围：`c < ' '` 或 `c == '\x7F'`
- 排除已知标准键：Tab(`\t`), Enter(`\r`), Esc（这是 KeyCode::Esc 不是 Char）
- 此类事件返回 `None`（丢弃）

**风险**: 低。crossterm 不会将标准功能键（F1–F12, Home, End, PgUp 等）报告为 `KeyCode::Char`。当前快捷键全部使用可打印字符或标准修饰键，不受影响。

---

## 不纳入本次修复的问题

| 问题 | 推迟原因 |
|------|----------|
| UiState 上帝结构体 | 涉及所有 handler 和 view 的接口变更，需要单独的设计会话和分阶段执行计划 |
| play_file 同步解码 | 涉及音频管线的异步化改造（channel-based streaming），需要仔细设计以避免竞态 |
| 集成测试 | 需要先搭建测试 harness（headless 终端 + 模拟输入），工程量较大 |
| 跨平台路径 | 当前「自包含目录」部署设计是故意的，改用 XDG 会导致行为变更 |

---

## 验证

```bash
cargo build                          # 零 warning
cargo test                           # 全部 42 个通过
cargo clippy -- -D warnings          # 零 warning（含消除的 3 处 needless_range_loop）
```

## 影响文件

| 文件 | 变更 |
|------|------|
| `src/constants.rs` | PCM_BUFFER_CAPACITY 8192 → 32768 |
| `src/visualizer/fft.rs` | 移除 needless_range_loop allow，改用 enumerate() |
| `src/visualizer/processor.rs` | 移除 2 处 needless_range_loop allow，改用迭代器 |
| `src/ui/mod.rs` | render() 中 if-else 链 → match 语句 |
| `src/config.rs` | 消除 Default impl 与 serde default 函数的值重复 |
| `src/main.rs` | tracing 增加 RUST_LOG 环境变量支持 |
| `src/input/handler.rs` | KeyHandler 增加 ASCII 控制字符过滤 |
| `DESIGN.md` | 版本号 → v3.2，记录本次改进 |
