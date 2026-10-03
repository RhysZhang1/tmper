# 守护进程拆分（阶段 1：缝合线）

**问题**：TUI 和音频引擎是同一个进程。关掉 TUI 就停播；桌面上（Plasma 媒体控件、媒体键、
`playerctl`）看不到 tmper。`STATUS.md:22` 把这两件事记在同一条限制里。

**这一阶段的产出**：声音搬到了另一个进程里。TUI 行为与今天一致，但 `q` 只关界面。

## 结构

```
src/
├── main.rs          # 参数 → daemon::run() / client::run_control() / App::connect()
├── cli.rs           # Play / Pause / Next / Prev / Stop / Volume / Status / Quit / Daemon
├── client.rs        # ★ 一次性动词：连上、发一条、等答复、打印、退出
├── daemon.rs        # ★ socket 监听、每客户端一个任务、空闲退出
├── ipc/
│   ├── mod.rs       # 行分隔 JSON 的收发（阻塞版 + tokio 版）
│   └── proto.rs     # Request / Event —— 两侧共享的唯一契约
├── player/          # ★ daemon 内核：engine + 队列 + 续播策略 + state.json
├── app/handle.rs    # PlayerHandle trait + LocalHandle（测试）+ DaemonHandle（生产）
└── app/ ui/ …       # 客户端，原地不动
```

单 binary 保持不变：daemon 用 `current_exe()` 重新 exec 自己（`setsid` + null stdio），
没有第二个可执行文件，也没有 workspace。

## 关键决定

**协议是不对称的。** 客户端发 `Request`；daemon 说的一切都是 `Event`，**包括对请求的答复**。
TUI 的循环在 `select!` 里，绝不能阻塞等回包。

**状态整体推送，不做增量。** 每 tick 一个 `StateSnapshot`（位置、时长、状态、音量、循环、
元数据、`queue_rev`），队列只在变化时推。快照是全量的，所以**重连不需要补课协议**——
下一个快照就是补课。`client_joined` 立刻推一份快照 + 队列，`tmper status` 就是读完这两条
就退出。

**一个 trait，两种投递时机。** `LocalHandle` 同步应答（测试用，~200 个测试因此不用改），
`DaemonHandle` 走 socket、下一 tick 到货。两者交给同一个 `App::apply_event`——**时机不同，
应用逻辑完全相同**，所以不存在"测试走一条路、生产走另一条路"的假绿。

**daemon 绝不因为客户端而等待。** 每客户端的信箱有界（256），满了丢**快照和频谱帧**
（下一 tick 就是新的，丢一帧只是闪一下）；丢**通知或队列变化**则断开该客户端——那是它
再也收不到的事实。两条路都不阻塞播放。

**`Player` 是 `!Send`**（cpal 的流），所以 daemon 的循环直接持有它，客户端只能通过 channel
够到它。这是音频栈强加的单写者规则，恰好和项目其余部分一致。

**读也是一份代码写两遍。** `BufRead` 和 `AsyncBufRead` 没有共同父 trait，所以 `read_line`
存在两次。约束被刻意收窄：上限、CRLF 容忍、EOF-与-截断的区别只表达一次，然后由测试把
**同一串字节**喂给两个实现，断言它们的结论一致。

**日志分流。** `main.rs` 用 `File::create` 截断 `tmper.log`，两个进程共写必然互相清空——
daemon 的历史（包括它临死前解释原因的那几行）会被下一次 `tmper` 抹掉。daemon 现在写
`tmper-daemon.log`。

**一次性动词不启动 daemon。** `tmper pause` 在没东西在放的时候应该说实话，而不是悄悄起一个
播放器好把它暂停。

**`q` / `:quit` 只关 TUI**（音乐继续），**`:quit!` / `tmper quit` 连播放器一起停**。帮助
浮层写清了哪个是哪个。

## 验证

自动：`cargo test` 438 通过 / 6 ignored（拆分前 380+6），`cargo clippy -- -D warnings` 干净。
新增覆盖：协议往返、两个 framing 实现逐字节一致、把真 socket 当 `UnixStream::pair()` 跑完整
握手 + 命令 + 答复、空闲退出（假时钟）、卡住的客户端被丢弃而不是被等待、`:quit!` 与 `:quit`
的区别。

**手工（在真桌面上，这一步只能你做）**：见下面「待你验证」。已用 `TMPER_*_DIR` 指向临时目录
在本机跑过一遍真 daemon：`tmper status` 在无 daemon 时报错并可读；TUI 路径拉起分离的 daemon；
一个裸 socket 客户端（python）握手、播放 fixture、断开，一秒后 `tmper status` 仍报 `playing`
——**音乐活过了客户端**；`pause` 后 daemon 不退；`quit` 后 socket 被清掉、`state.json` 落盘
（`volume: 0.4`、`last_track_path` 正确）。

## 待你验证（我做不了的部分）

1. 开 TUI 放歌 → `q` → **声音应该继续**；`tmper status` 应报 playing。
2. 重新 `tmper` → 应接回同一首、同一进度。
3. `tmper pause` / `next` / `volume 40` / `status`。
4. `tmper quit` 全停。
5. 放着不动 5 分钟（`DAEMON_IDLE_EXIT_SECS`）后 daemon 自己退出。
6. `nc -U $XDG_RUNTIME_DIR/tmper/socket` 手工发一行 JSON —— 这个协议存在的一半理由。

## 还没做

- **阶段 2**：`LibraryDb`、扫描器、歌单搬进 daemon；曲库搜索/下钻变成 `LibraryQuery`；
  客户端不再打开 SQLite。
- **阶段 3**：MPRIS2（`mpris-server` + zbus）；封面缓存成真文件供 `mpris:artUrl`。
- **阶段 4**：daemon 中途死掉后的**自动重连**（现在读到 EOF 会提示并退出 TUI）、陈旧 socket
  的显式处理、四份文档、`cargo llvm-cov` 复测。
