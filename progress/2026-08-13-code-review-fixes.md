# 2026-08-13 会话：代码评审整改 + 覆盖率工具 + 帮助面板键位修复

## 概述

用户要求对项目做一次全面评审后按评审意见整改。本次评审（会话开头）指向三项工程债 + 一处显示 bug，逐项修复如下。

## 变更清单

### 1. 收敛 `audio/engine.rs` 的 `Mutex::lock().unwrap()`
- 生产代码中 20 处 `Mutex::lock().unwrap()` 全部替换为 poison-recovery 辅助函数
  `fn lock<T>(&Mutex<T>) -> MutexGuard<T>`。
- 该函数用 `lock().unwrap_or_else(PoisonError::into_inner)`：引擎从不跨 `?` 持有锁，
  poison 时被守护数据仍一致，`into_inner()` 安全，且避免后台解码/FFT 线程因无关 panic 连带崩溃。
- 覆盖四把锁：`duration_secs` / `position` / `pcm_buffer` / `InstrumentedSource.buffer`。
- 唯一剩余 `unwrap()` 在测试断言内（`dur.unwrap()`），符合惯例。

### 2. 帮助面板键位显示修复（用户报告）
- `help_popup.rs` 标题与页脚「按 0 或 Esc 关闭」→「按 8 或 Esc」。
  根因：历史 commit `11fd607` 把帮助开关键从 0 改为 8，但文案未同步。
- 「可自定义 9 个键位」→「8 个键位」，并从键位清单移除 `stop`。
- `keymap.rs` 删除从未被读取的死字段 `stop`（全库 `key_bindings.stop` 无引用），
  与设置页 / README / 帮助页的 8 键对齐。

### 3. 补 `cargo-llvm-cov` 行覆盖率工具
- 评审发现「测试数 59」被当作「覆盖率」，而 `ui/views/*` 与 `app/handlers/*` 分发逻辑无单元测试。
- `CLAUDE.md` Commands、`README.md` 开发、`DESIGN.md` §10.3 三处补上 `cargo llvm-cov` 命令与一次性安装前提。
- `ci.yml` 新增 `coverage` job：`llvm-tools-preview` + `taiki-e/install-action` 装 `cargo-llvm-cov`，
  null ALSA 下运行 `cargo llvm-cov --workspace --all-features`。**上报不设门槛**（暂不 `--fail-under-lines`）。

### 4. DESIGN.md 测试表改名 + 澄清
- §10.1 标题「测试覆盖」→「测试分布（按测试数）」，加注：下表是测试**数量**非**行覆盖率**。
- 新增 §10.3 行覆盖率小节。

## 验证

```
cargo check --all-features              ✅
cargo clippy --all-features -- -D warnings  ✅ 零警告
cargo fmt --all -- --check              ✅（lock() 链式调用已由 rustfmt 重排）
cargo test（null ALSA 模拟无头）         ✅ 59 passed; 0 failed
engine.rs 残留 .lock().unwrap()         ✅ 0 处
```

## 已知遗留

- **行覆盖率基线（2026-08-13 实测）**：总 **45.09%** 行（59.51% 函数、42.73% 区域）。
  首次安装 `llvm-tools-preview` + `cargo-llvm-cov` 因沙箱网络超时失败，关闭沙箱后成功
  （`cargo-llvm-cov v0.8.7`，1m26s）。
  覆盖率高地：decoder 91%、engine 95%、metadata 97%、database 93%、playlist_manager 97%、fft/processor 97%。
  覆盖率为 **0** 的模块：`app/handlers/settings.rs`、`main.rs`、`ui/views/lyrics_view.rs`、`ui/views/player_view.rs`(440 行)、
  `ui/widgets/help_popup.rs`、`ui/widgets/visualizer_panel.rs`。接近 0：`app/handlers/playlist.rs` 5%、`playlist_view.rs` 7%。
  `--fail-under-lines 80` 门槛待覆盖达标后再启用。
- 覆盖率最大缺口仍是 `ui/views/*`（~1500 行）与 `app/handlers/*`（~1900 行）——分发/渲染层零单元测试。
- 未动（用户未要求，另立专项）：`handlers/mod.rs`(701 行) 准上帝文件、`cover/mod.rs` 终端图形渲染脆弱性、
  目录播放 `tmper play <dir>`、`library/scanner.rs` 无生产扫描器。
