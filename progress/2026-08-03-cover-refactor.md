# 2026-08-03 会话：封面渲染收敛 — 一次性发送 + 移除防御层

## 概述

在 `2026-08-01-cover-rollback.md` 根因分析的基础上，对封面子系统做了结构性重构：**以"SIXEL 是 Konsole 上的持久图形层"为唯一前提，把渲染收敛为"每次变化发送一次"**，并删除了围绕"每帧重发"堆叠起来的整套防御层。

## 变更清单

### commit 48d79d3 — perf: cache half-block cover render（半块渲染缓存）
- `CoverLinesCache`：半块封面渲染（decode + Lanczos3 + 抖动）从每帧重算改为按 `cover_gen` + 渲染区域缓存
- 属独立优化线，不与本重构冲突

### commit 71a4c8d — refactor: cover rendering（本次核心）
**`src/ui/cover/mod.rs` 重写**：
- **一次性发送**：chafa SIXEL 只在 `cover_gen` 变化、渲染区域变化、或首次显示时发送；不再每帧重发
- **协议互斥**：`CoverRenderer::render()` 按终端检测在 Kitty 与 chafa SIXEL 间二选一（原先两者每帧都写，在 Kitty 终端上争抢同一区域）
- **sticky `chafa_failed`**：chafa 输出为空（终端不支持 SIXEL）或子进程失败时置位，直到封面/区域变化才重试——消除"每帧 spawn chafa 子进程"的浪费
- **无封面曲目清除残留**：新曲目无封面时置 `clear_pending`（原版只清 cache 不设 clear，旧 SIXEL 会残留覆盖 fallback 文字）
- **移除裸 `\x1b[?25l`**：光标隐藏绕过 ratatui 光标管理，退出后光标停在隐藏态
- **可测试性**：writer（`Box<dyn Write + Send>`）+ encoder（`trait SixelEncoder`，chafa 可替换为 stub）可注入；`CoverRenderer` 保持 `Send`（tokio 多线程运行时要求）

**删除的防御层**：
- `handlers/mod.rs`：200ms 盲拦 Char 的 cover-escape guard（`last_output()` 消费方）
- `playback.rs`：`on_track_ended` 里的 `suppress_frames(10)`
- `constants.rs`：`COVER_SUPPRESS_FRAMES`
- `cover/mod.rs`：`suppress_countdown` / `suppress_frames` / `last_output_at`
- **保留**：`input/handler.rs` 的 ASCII 控制字符过滤（通用卫生，非封面专用）

### 新增测试（6 个，`src/ui/cover/mod.rs`）
| 测试 | 锁定不变量 |
|------|-----------|
| `chafa_sends_once_until_cover_changes` | 封面/区域不变 → 零写入；换歌 → 重发 |
| `chafa_resent_on_area_change` | 终端 resize → 重发 |
| `leaving_player_view_requests_clear` | 离开播放器视图 → 置 clear_pending |
| `hiding_cover_schedules_clear` | 关闭封面 → 置 clear_pending |
| `coverless_track_clears_stale_sixel` | 新曲目无封面 → 清除残留（原版 bug） |
| `empty_sixel_output_is_not_retried_every_frame` | chafa 失败 → 不每帧重试 |

## 验证

```
cargo fmt --all               ✅
cargo clippy --all-targets -- -D warnings  ✅ 零警告
cargo test                    ✅ 59 passed（53 + 6 新增封面测试）
```

## 用户需要在 Konsole 上验证

一次性发送依赖"Konsole 的 SIXEL 持久"这一前提（已由 ratatui-image 实验证明，但未在真机确认过发送一次后的完整生命周期）：

1. **切歌**：连续播放 2-3 首带封面的歌，确认封面切换正常、无旧图残留
2. **切到无封面歌曲**：确认旧的 SIXEL 被清除，不残留覆盖 fallback 文字
3. **切视图往返**：Player → 其他视图 → Player，确认封面重新出现、无 ghost
4. **改终端尺寸**：resize 后封面按新区域重发
5. **伪按键**：切歌/操作后观察是否还有幻影按键（如自动弹出帮助页、音量突变）
6. **退出**：确认退出后终端光标可见（不再隐藏）

## 回滚

两个 commit 相互独立：
- `git revert 71a4c8d` 还原重构（恢复 guard + 每帧重发 + suppress）
- `git revert 48d79d3` 还原半块缓存
若一次性发送在 Konsole 上失效（切歌后封面不更新），首选方案是回到"按脏标记周期性重发"，而非恢复整摞防御层。

## 已知遗留

- 半块字符渲染在 SIXEL/Kitty 下仍会被覆盖（`player_view.rs` 总是先画，再被原生图形盖住）。已被缓存降低到"每次封面一次"的成本，可接受。
- Kitty 路径在 Konsole 上仍是死代码（环境检测恒 false）——保留作为其他终端的正确路径，非本次范围。

---

## 追加（当日）：伪按键真正根因 — chafa 终端探测

### 用户实测反馈

一次性发送修复了残留/切回重显，但伪按键依旧，且暴露了新细节：
- 显示半块像素 → **卡 3-4 秒** → 进命令模式
- 幻影输入：`fcfc/fcfc/fcfc\]11;rg:2323/2626/2727\`
- 手动 Esc 退出后可能再次卡进命令模式；从其他页面切回也可能触发

### 根因定位（一次实证）

幻影输入里出现 `rgb:2323/2626/2727`（深色背景）与 `fcfc/fcfc/fcfc`（白色前景）——这是 **OSC 10/11 颜色响应格式**。tmper 自身从不发 OSC 查询（grep 证实），于是怀疑 chafa 子进程。

用受控 pty（`script`）复现确认：
- **`--probe` 开启（默认）**：chafa 向终端发 `\x1b]10;?`（查前景）与 `\x1b]11;?`（查背景），**阻塞等待响应 5.05 秒**（默认超时 5.0s）
- **`--probe off`**：36ms 完成，无任何查询，SIXEL 输出正常

**机制**：chafa 默认经 `/dev/tty`（ctty）探测终端能力。Konsole 响应 OSC 10/11 时，把 `\x1b]10;rgb:...\x1b\\` / `\x1b]11;rgb:...\x1b\\` 写回 **PTY 输入侧**——与 tmper 的 stdin 是同一通道。crossterm 把这些字节解析成按键 → 进命令模式 + 输入垃圾；chafa 等响应 5 秒 → 主线程 `wait_with_output()` 阻塞 → UI 卡 3-4 秒（屏幕停在半块封面）。这也解释了为何 2 周来所有 guard/抑制都只能减轻不能根除——问题在 chafa 进程，不在 tmper 的输出或输入路径。

**修复**（commit `3a03ac0`）：chafa 参数加 `--probe off`。tmper 选 chafa 就是为了 SIXEL，终端支持性已知，探测无意义。

**验证**：`chafa --probe off` 在管道模式下 36ms、无 OSC 查询、输出有效 SIXEL（`\x1bP` DCS）；59 测试全过。用户在 Konsole 上实测确认（第 5 项"伪按键"应彻底消失）。

