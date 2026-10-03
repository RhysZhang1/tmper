# 2026-10-03 会话（续）：覆盖率攻坚

## 背景

承接同日的 `2026-10-03-bugfix-series.md`。修复系列结束时行覆盖率 80.09%，但若按**未覆盖行数**
而非百分比排序，缺口集中在一批从未被测试的模块上。本次按该排序逐个补齐。

## 结果

测试数 **223 → 320**（+6 设备门控），行覆盖率 **80.09% → 88.78%**（函数 88.24%、区域 87.75%）。

| 模块 | 之前 | 之后 |
|------|------|------|
| `audio/engine.rs` | 48% | 70% |
| `library/scanner.rs` | 30% | 98% |
| `app/persistence.rs` | 58% | ~100% |
| `app/playback.rs` | 64% | 85% |
| `app/handlers/mod.rs` | 75% | 88% |
| `ui/views/player_view.rs` | 72% | 92% |
| `config.rs` | 59% | 86% |
| `lyrics/parser.rs` | 78% | 95% |
| `ui/mod.rs` | 75% | ~85% |

## 过程中发现并修复的两个真实缺陷

### 1. 模式变更通知从未可见（`ui/mod.rs`）

`render` 把浮动通知画在**视图分发之前**，而视图随后渲染整帧 —— 通知被直接覆盖。
之所以一直没被发现，是因为测试只断言 `ui_state.notification`（状态），从不检查实际画到屏幕
上的内容。补上第一个检查渲染缓冲区的测试后它立刻失败。

通知现在在视图之后绘制（`render_notification` 辅助函数），浮层本就该在最上层。

### 2. 若干写入函数依赖"别人已经建好目录"

`export_m3u` / `save_playlists` / `save_library_paths` 都向一个假定存在的目录写入。
生产环境能工作**仅仅因为 `main.rs` 在启动时创建了 XDG 目录**；这使得它们既脆弱又无法单测。
现在与 `save_state` 一样自行创建父目录。

另有一处 `App::new` 的隐藏副作用：它在构造时调用 `load_state()` 读磁盘，导致一个测试保存的
`repeat_mode` 会改变其后**每一个** App 的初始状态（`test_repeat_mode_cycles` 因此失败，且
失败与否取决于线程调度）。持久化状态的恢复已移到 `run()` —— 那才是"启动应用"发生的地方。

## 补齐的内容

- **`audio/engine.rs`**：`InstrumentedSource` 的环形缓冲馈送与 `Drop` 释放未播放计数
  （每次切歌/seek 都依赖它，否则背压计数永不归零、解码彻底卡死）；会话辅助函数
  （`atomic_saturating_sub` / `update_peak` / `session_cancelled` / `send_failure`）；
  headless 状态机与 `drain_events` 的陈旧会话过滤。
- **`library/scanner.rs`**：遍历错误必须让列表标记为不完整（缺失根、不可读子目录，后者带
  root 环境守卫）；指纹命中跳过、变更重读；元数据失败计入 `failed` 但**不**影响 `complete`
  （损坏文件不是遍历错误）；取消；进度上报只从跳过分支触发。
- **`app/persistence.rs`**：state/歌单/库路径的往返、缺失与损坏文件、去重、丢弃已不存在的条目。
- **`config.rs`**：`clamp` 的上下界（0 会让帧率/柱数除零）、f32 两位小数序列化、
  以及"`config/default.toml` 与代码默认值一致"（此前无人保证）。
  文件读取路径（`load_or_default`）**有意不测**：配置路径是进程全局的，多个测试模块都会写它，
  读回会不稳。它依赖的解析逻辑改用 `toml::from_str` 直接覆盖。
- **`lyrics/parser.rs`**：BOM/UTF-8/GBK/Shift-JIS 检测、兜底替换字符路径、唯一会 Err 的输入。
  其中一个测试**钉住一个真实缺陷**而非断言行为正确：调用方传入
  `["utf-8", "gbk", "shift-jis"]`，而 GBK 几乎能解码任意字节序列，所以 Shift-JIS 歌词会被当成
  GBK 乱码，永远轮不到 shift-jis 分支。
- **`ui/views/player_view.rs`**：迷你歌单、歌词区（含跟随光标的滚动）、歌曲信息各槽位、
  控制栏进度与零时长。
- **`ui/mod.rs`**：通知弹窗、命令面板、帮助覆盖层优先级、窄终端下弹窗几何。

## 验证

```
cargo fmt --all -- --check                    ✅
cargo clippy --all-targets -- -D warnings     ✅ 零警告
cargo test                                    ✅ 320 passed; 0 failed; 6 ignored（无需声卡）
cargo llvm-cov --all-features --workspace     ✅ 88.78% 行
```

## 已知遗留

剩余未覆盖率是**结构性**的，不是遗漏：

- `audio/engine.rs` 70%：6 个 `#[ignore]` 设备测试的函数体计入未覆盖；`new` 与 `play_file`
  需要真实声卡。要提升只能引入虚拟音频设备（CI 已有 null ALSA 方案，但默认测试集刻意不依赖它）。
- `app/mod.rs` 63%：`TerminalGuard` 与 `run()` 事件循环需要真实 tty。若要可测，`TerminalGuard`
  需要把 writer 变成可注入的（目前硬编码 stdout）。
- `paths.rs` 29%：非 `cfg(test)` 分支在测试构建下不参与编译，百分比天然偏低。
- `main.rs` 0%：二进制入口。

另：`lyrics/engine.rs` 的 `find_lyrics` / `load` / `sync` 仍无直接测试（由 playback 的歌词
测试间接覆盖）。
