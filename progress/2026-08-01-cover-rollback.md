# 2026-08-01 会话：封面渲染 ratatui-image 尝试与回滚

## 概述

用户提出「规划封面问题根治」。按方案 A（ratatui-image 集成）实施，两次尝试在 Konsole 上均失败，最终回滚到原版（chafa SIXEL + cover-escape guard）。

## 尝试过程

### 1. ratatui-image 8.1.1 集成（SIXEL 自动检测）
- 用 `ratatui-image` widget 替换手写 Kitty 编码 + chafa 子进程 + 每帧 SIXEL 重发（`src/ui/cover/mod.rs` 重写、`app/mod.rs` draw 闭包内渲染）
- 删除 cover-escape guard、`suppress_countdown`、`clear_pending`、`KITTY_CHUNK_SIZE`、`COVER_SUPPRESS_FRAMES` 等 4 层防御
- **失败**：Konsole 的 SIXEL 不响应整屏清除（KDE bug 456354，ratatui-image 官方标 won't fix），切视图后封面残留；`terminal.clear()` 对 Konsole 无效

### 2. Konsole 强制半块协议
- `init()` 检测 `KONSOLE_VERSION` → `set_protocol_type(Halfblocks)`
- **失败**：半块字符显示质量差，用户反馈「不如原来的」

## 回滚

- `git checkout -- .` 恢复全部改动到 HEAD（5580b4d，上一会话最终原版）
- 原版恢复：chafa SIXEL 高清封面 + guard 兜底 + clear_pending

## 根因分析（关键洞察）

- **用户终端是 Konsole**：不支持 Kitty 协议，SIXEL 是唯一高清路径
- **SIXEL 是持久图形层**：ratatui-image 残留实验（切视图后仍显示）证明 Konsole 的 SIXEL 在单元格被覆盖后**仍保留** → 说明原版每帧重发其实不必要
- **伪按键 + 卡顿的真正来源很可能是「每帧重发」**：`render_chafa` 每帧向 stdout 写完整 SIXEL 缓存（~30fps × 全图）→ 海量原始字节持续流向终端 → Konsole 误判为键盘输入的概率大增；同时 Konsole 每帧渲染全图导致 UI 卡顿
- 原版 guard 设计只覆盖「内容变化时」的写入（`last_output_at` 不随每帧重发更新），对每帧重发产生的伪按键其实无效——这正是 guard 打了 3 轮补丁仍不干净的深层原因

## 后续方向

**验证「chafa SIXEL 只在封面变化时发送一次」在 Konsole 上能否持续显示。** 若能 → 伪按键与卡顿同时根治，guard 可移除，显示不变（仍是 SIXEL 高清）。若不能持续 → 退路：按周期/脏标记重发，或 buffer skip 标记（ratatui-image 思路）防止 ratatui 覆盖六素区域。

## 附加发现

- 上一会话遗留一个泄漏的 `cargo test` 子进程（`target/debug/deps/tmper-*`）从 21:20 挂到 23:30，持有音频设备导致新的 `cargo test` 在音频测试上阻塞。已 `kill -9` 清理。原版 49 测试验证通过。
