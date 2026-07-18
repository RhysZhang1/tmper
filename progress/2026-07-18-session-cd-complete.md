# 2026-07-18 Session C+D 完成记录

## Session C: play_file 异步化解码

- `output.rs`: 新增 `handle()` → cloneable OutputStreamHandle
- `engine.rs`: 新增 `play_file_async` (spawn_blocking + 独立 Sink), `cancel_decode` 机制
- `persistence.rs`: 文件 >50MB 自动走异步路径
- `stop()` / `seek_relative()` 自动取消进行中的异步解码

## Session D: stdout 写入防护增强

- `constants.rs`: `COVER_SUPPRESS_FRAMES = 10`
- `app/mod.rs`: 新增 `drain_spurious_events()` — cover 渲染后 2ms 轮询排空伪输入

### 防御层次（4 层）
1. suppress_frames(10) — 330ms 静默
2. drain_spurious_events() — 渲染后排空
3. KeyHandler 控制字符过滤
4. last_help_toggle 500ms 冷却

## 验证
cargo build / test / clippy — 全部通过
