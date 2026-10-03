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

## 追加（同日）：暂停即空闲，退出前把现场存下来

`tmper stop` 修好之后剩下的那条规则悬着：暂停算不算「在做事」？原来的答案算（`is_engaged()`
把 `Paused` 和播放并列），于是**一个暂停的 daemon 永远不会退**——暂停可以挂好几天，空闲计时
永远不开始。你选了「暂停也算空闲」，代价是退出前必须把现场存下来，否则那 5 分钟会吃掉进度。
规则因此变成：**没人连着、也没出声（停止或暂停）才开始计时，5 分钟后退出；播放中永不退出**
（关掉 TUI 音乐继续，是 daemon 存在的理由）。

**只存 `last_track_path` 是不够的**，那只能让 `play` 从头再放一遍。现在 `state.json` 里还有
`queue`、`queue_index` 和 `position_secs`（退出那一刻的实时位置），启动时全部恢复——但**不出声**：
曲目在架上、进度条停在那一秒，按播放才接着放（`Player::resume_at` 是被「停驻」的针，第一次
start 用掉它）。

**针要停在声音上，不只是停在数字上。** `play_file_async` 原来硬编码从 0 解码，只把时钟拨到
4:30 会让显示和扬声器整首歌都对不上。`AudioEngine::play_file_at(path, offset)` 把偏移同时交给
`reset_position` 和 `spawn_decoder`（后者本来就收 `offset_secs`，`seek_relative` 一直这么用），
`drain_events` 的 `Ready` 分支重读 `base_offset`，所以偏移活过 Loading → Playing。恢复再叠加
一层：`offset` 夹到 `duration * 0.999`，正好落在最后一个采样上的 seek 会立刻结束、下一 tick 就
换歌。`Player::play` 与 `Player::start(path, offset)` 是同一条路径，不给自己留第二条。

**顺手改掉的两个真 bug：**

1. `Request::Shutdown` 原来先 `stop_playback()` 再让 daemon 存盘——而 stop 是回卷，于是每次
   `tmper quit` / `:quit!` 都忠实地把 `position_secs: 0.0` 写进去。现在安静设备这一步挪到
   `Player::shutdown()`，在 `save_state()` **之后**跑（测试 `a_shutdown_saves_the_position_before_it_silences_the_player`
   盯着这个顺序，把 `stop_playback()` 挪回去会红）。
2. `init_logging` 在 `create_dir_all` 之前就 `File::create`：全新安装时 state 目录还不存在，
   日志静默落到 `/dev/null`——**第一次运行恰好是唯一没有日志可读的那次**。现在先建目录。

`Stop` 会清掉停驻的针（停止就是回卷，这是 `stop` 的定义），`resume` 之后从头开始；只有
「从 `state.json` 恢复」这条路才带着偏移。

## 验证（本次）

自动：452 通过 / 6 ignored，clippy 干净。新增测试都验证过有牙：把 `Ready` 分支的
`base_offset` 改成 `0.0` → 偏移测试红（位置塌到 0.0000036）；把 `stop_playback()` 挪回
`Request::Shutdown` → 存盘顺序测试红（存下来的是 0）。

真桌面上跑通整条链路（`TMPER_*_DIR` 全指向临时目录）：

1. play → pause（0.73s）→ `tmper quit`：`state.json` 记下 `position_secs: 0.73`、`queue: 1`、
   `queue_index: 0`。
2. 起一个新 daemon：`tmper status` 显示曲目已在架上、`queue 1 tracks, playing #1`，**没有声音**。
3. `resume`：第一帧位置 1.03（= 0.73 + 已过时间），音轨在 1.53s 后自然结束——2 秒的文件从 0.73
   开始只剩约 1.27 秒，所以**声音真的从偏移开始**，不只是时钟。
4. 把 `DAEMON_IDLE_EXIT_SECS` 临时改成 5 秒重跑：暂停后断开所有客户端，daemon 自己退出、
   清掉 socket，`state.json` 里位置 0.678；再起一个 daemon，`play` 从 0.678 接着放。常量已改回 300。

用户真实的 `~/.local/state/tmper/` 全程未被触碰；临时目录与进程都已清理。

文档：README（后台播放一节 + `tmper play` 不带文件 = 接着放 + `q`/`:quit!` 的区别 + 持久化
那条从「不自动恢复上次曲目」改掉）与 STATUS（能力、空闲退出规则、MPRIS 之外「桌面上还看不到
这个播放器」）已按本次改动更新；行覆盖率复测 **88.87%**（升自 88.57%），CLAUDE.md 与
DESIGN.md 里的数字与那句话同步。**但 DESIGN.md 的架构叙述（以及 CLAUDE.md 的布局与并发模型）
仍写着拆分前的单进程样子**——那是阶段 4 的整体对账，这次只动了被本次改动直接证伪的行。
