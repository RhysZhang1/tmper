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

- ~~**阶段 2**：`LibraryDb`、扫描器、歌单搬进 daemon；曲库搜索/下钻变成 `LibraryQuery`；
  客户端不再打开 SQLite。~~ **已完成，见下。**
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

## 追加（同日）：阶段 2 —— 曲库、扫描器与歌单搬进 daemon

阶段 1 之后，daemon 拥有声音，客户端仍然拥有**索引和歌单**。这条缝很别扭：`Next` 走的是
客户端手里那份歌单，而歌单是「要放的东西」，本来就该和播放器待在一起。这一阶段把它搬完，
分三刀落地：

- **2a：索引与扫描器。** `LibraryDb`（SQLite + FTS5）与 `scan_incremental` 移入 `player/`；
  曲库面板的搜索与艺术家→专辑→曲目下钻变成 `LibraryQuery` 请求，进度变成
  `ScanProgress` / `ScanFinished`。客户端不再打开 SQLite。
- **2b-A：曲库目录（`library.json`）。** 加入/移除目录变成请求，目录表整体作为
  `Event::LibraryPaths` 推送——和队列一样，它只在变化时推。
- **2b-B：歌单。** 如下。

### 歌单的身份是一个 id

**名字不是身份。** UI 从来没有禁止过两个歌单都叫 "Mix"，按名字寻址的编辑会悄悄落在
daemon 先找到的那一个上。**位置也不是**——那是对「调用者手里那份拷贝」的光标，而那份
拷贝正是另一个客户端可能刚改过的。所以 `PlaylistData` 加了 `id`：由 store 发放、永不复用。
`#[serde(default)]` 让旧文件（全是 0）照常加载，`number()` 在 load 时补号并**立刻写回**，
所以迁移是一次性的，不是每次加载都要重做的。手改过的文件里重复的 id 同样重新编号——
这正是 id 要排除的那种失败（编辑落到错误歌单），所以宁可改号也不拒绝。

「打开的歌单」也是 id（`StateSnapshot.active_playlist`），并且 daemon **每次调用都从 store
现读**那份歌单（`active_list()`），而不是在字段里留一份拷贝。于是「编辑正在播放的歌单」
**就是**编辑正在走的那个列表，`sync_active_list` 从所有改动点消失了。

### 事件顺序：先镜像，后指认

`PlaylistCreate` 先推 `Event::Playlists`（整个 store）再推 `PlaylistAdded { id }`。
反过来就是把一个客户端还没见过的 id 交给它——`apply_playlist_added` 拿着 id 去镜像里找行，
找不到就直接返回，新歌单不会被展开。**引用某个状态的事件必须跟在该状态之后**，和
「快照先于队列」是同一条规则。客户端测试红过一次才发现的。

**创建与导入是两个事件**（`PlaylistAdded` / `PlaylistImported`），因为两个动词在客户端
有真正不同的反应：创建要把光标移到新行，导入要报条数。从一个事件里分辨这两者需要客户端
跟踪在途请求——为了省一个 enum 变体而引入一套状态机，不划算。

### 删掉的东西

`src/app/persistence.rs` 整个删除——客户端不再自己写任何文件。`ui_state.playlist_name`
（写了两处、从没读过）、`save_playlists()`、`sync_active_list()`、`Request::SetActiveList`
一并消失。导出仍然由 daemon 写文件，但只回报**事实**（写成的路径 + 失败数），句子由客户端
从事实里组——「2/3」是两个事实，不是一个措辞。

**两种通知各归其位**：面板自己的 `notification` 是**按键反馈**（「请先展开一个歌单」），
不往返；daemon 的答复走全局通知——它到达时用户可能已经在别的视图里了，答案不该落在一个
看不见的面板里。

### 验证

自动：**480 通过 / 6 ignored**，clippy 干净。真 socket + 隔离 `TMPER_*` 目录跑通全部动词：
建、加曲、去重、按位置删曲、两个同名歌单拿到不同 id、按 id 编辑只落在被点名的那一个、
对不存在的 id 编辑是 no-op、`set_active_playlist`、导出单个/全部、导入（不存在的路径被丢掉）、
导入失败变成一条 notice 而不是崩溃、删除正在打开的歌单后 `active_playlist` 归 None。
`state/playlists.json` 由 daemon 写出，`data/` 里是导出的 M3U。

**用户真实的 `~/.local/state/tmper/playlists.json` 全程未被触碰**（mtime 仍是 14:48，
文件里仍然没有 `id` 字段）。注意：**下一次真实启动会重写它**——补上 id 并写回，这是设计里
的一次性迁移，不是意外。内容不变（名字、歌曲、顺序都不动），只是多了 id 字段。

### 还没做

- **阶段 3**：MPRIS2（`mpris-server` + zbus）；封面缓存成真文件供 `mpris:artUrl`。
- **阶段 4**：daemon 中途死掉后的自动重连、陈旧 socket 的显式处理、四份文档对账、
  `cargo llvm-cov` 复测。

## 追加（同日）：阶段 3 —— MPRIS2

daemon 现在在会话总线上占名 `org.mpris.MediaPlayer2.tmper`（`mpris-server` 0.10，走 zbus——
这台机器本来就有的 D-Bus 栈）。它从**推给客户端快照的同一个地方**推给桌面：媒体控件看到的
状态和 TUI 看到的状态是一份状态、播报一次。

### 只报动了的部分

快照每秒来 31 个，其中绝大多数只差一个 `position_secs`，而位置不是 MPRIS 属性（客户端要
位置时会问 `Position`）。`State::apply` 把快照和上次播报过的镜像做 diff，只发变化的属性——
否则就是每秒 31 条 `PropertiesChanged` 和一块永远在重绘的控件。

两处细节是**承重**的：

- **音量要收窄。** tmper 的音量是 `f32`，直接加宽到 MPRIS 的 `f64` 会得到
  `0.800000011920929`——同一条 80%，后面挂了十一位噪声，而且不再往返：客户端设 0.8、读回
  0.8 却发现变了。`to_volume` 取三位小数（比任何音量滑条都细），往返精确，diff 也不会
  永远发现同一个「变化」。
- **setter 要先把值记进镜像。** zbus 对每个属性 `Set` 会**自己**发一条 `PropertiesChanged`，
  内容在 setter 返回那一刻从 getter 读——而我们的 setter 是 fire-and-forget，返回时 daemon
  还没应用。不记的话，每一次调音量都会被播报成「被替换掉的那个音量」，一毫秒后才是真的：
  滑条每动一格，先弹回去、再弹过来。`Player::record` 把打算生效的值先写进 getter 读的镜像，
  并且**按快照将来拼写的方式**写，否则随后到的那条快照又会发现一次差异——一次点击又变成
  两条信号。

  循环模式是这条规则里最尖的例子：tmper 一个三态 `RepeatMode`，MPRIS 是两个 flag
  （`LoopStatus` + `Shuffle`）。setter 只记**客户端点名的那一个**——另一个如果真被这次
  `Set` 带着动了（`Shuffle=true` 就是 `Playlist`；在乱序上点 `Track` 顺带关掉 shuffle），
  随后那条快照会播报它，而那是控件**需要**知道的事实，不是重复。（试过「两个 flag 一起记」
  来省掉这条信号：控件会停在「已乱序 + 未循环」的画面上，直到下一次不相干的变更为止。）
  镜像要挡的是另一种：zbus 从旧 getter 读出来的、一毫秒后就被真值推翻的那一条。

  同理，「关掉 shuffle」不能理解成「停止循环」——控件每次刷新都会重报自己的值，而 `Track`
  循环配一次 `Shuffle=false` 不该把循环一起删掉。`set_shuffle(false)` 因此要回读循环轴
  （即客户端眼里的现状），再决定请求 `Sequential` 还是 `SingleTrack`。

seek 是唯一「结果在所有属性里都看不见」的命令，所以拿到 MPRIS 为此专设的那条信号：
状态之后跟一条 `Seeked`。

### 封面

`mpris:artUrl` 必须是个 URL，所以封面落盘到 `$XDG_CACHE_HOME/tmper`（新增
`paths::cache_dir()`），文件名按**曲目路径**取而不是按内容：桌面只在元数据变化时取一次图，
换个名字就等于让控件重新下一张它已经有的图。缓存按 mtime 留最新 8 张（`KEEP`）；快照里带
的是路径，客户端直接读同一个文件，封面不必过 socket。

### 顺带的发现：客户端会吞掉欢迎词后面的问候

验证过程中 `tmper status` 偶发 `the player closed the connection`——25 次里 3 次，且只在
daemon 有流量时（MPRIS churn），安静时 25/25 全过。真因不在 daemon：
`DaemonHandle::connect` 的握手借了一个 `BufReader` 读 `Welcome`，读完就 drop。`BufReader`
为了回答一次 `read_line` 会把 socket 上**所有**能读到的字节搬进自己的缓冲（最多 8 KiB），
而 daemon 在接受握手的那一刻就把快照和队列发出来了——这些字节于是和那个 reader 一起进了
垃圾桶。落在行边界上表现为「丢失问候」（队列只在**变化**时推，所以客户端会永远等一个它
已经收到过的队列）；落在行中间就是 `malformed: expected value at line 1 column 1`，被客户端
自己的措辞报成了「连接被关闭」。修法是把握手挪到异步侧，**同一个 reader** 接着交给读任务
用——「一个字节都不丢」从「要记得的性质」变成结构上的性质。新测试对该行为回归有效（把旧
写法装回去，测试变红，已实测）。`ipc` 里那份阻塞版封帧随之失去最后一个调用者，一并删除：
一份实现不会和自己就上限、CRLF、截断行的含义产生分歧。

### 验证

自动：**513 通过 / 6 ignored**，clippy 干净。

真会话总线 + 隔离 `TMPER_*` 目录，用 `busctl` 逐项核对：身份 / desktop entry / 能力标志、
空载时的 metadata、播放中的 metadata（微秒长度、`/tmper/track/{:016x}` 的 track id）、封面
PNG 落在缓存里且 `artUrl` 指向它、每次 `Set` 只有一条信号且带的是新值、`PlayPause` 两个
方向、`Seek` 之后跟一条 `Seeked`、暂停反映在 daemon 自己的状态里。`tmper status` 在同样的
churn 下 40/40 通过（修之前 3/25 失败）。

**留给你的（这里验不了）**：Plasma 媒体控件真的把它画出来、媒体键真的送过来。本机能验的
只到总线这一层。

### 一次事故，如实记下

验证 MPRIS 的过程中，有一次 `source env.sh` 静默失败（那个文件属于上一个临时根），daemon
于是带着**默认 XDG 路径**起来了，碰了你真实的 `~/.local/state/tmper/`：`playlists.json` 在
21:36 被重写（原 mtime 14:48），并新建了一个 496 字节的 `tmper-daemon.log`，还有一个随后被
删掉的 socket。事后 diff：**`playlists.json` 与重写前的副本逐字节相同**（7864 字节，两个歌单
72/37 首，`id` 字段是 20:27 那次真实运行留下的）——内容没有丢、也没有改。`state.json`
（20:27）、`library.json`、`library.db`、`config.toml` 都没被碰：daemon 是被 `SIGKILL` 掉的，
`shutdown()` / `save_state` 没有机会跑。此后所有验证都走一个会**拒绝 `/tmp` 以外任何根**的
启动脚本。

### 还没做

- **阶段 4**：daemon 中途死掉后的自动重连（重连横幅）、四份文档对账、`cargo llvm-cov`
  复测。（空闲退出与陈旧 socket 已在前面阶段落地并有测试。）——**已完成，见下。**

## 追加（同日）：阶段 4 —— 死掉的播放器与四份文档

### 播放器死掉，TUI 留下来

阶段 3 结束时，daemon 一旦中途死掉，客户端读到 EOF 就报一句「连接被关闭」然后**退出 TUI**。
把界面也一起带走是最省事的答案，但它把「播放器没了」和「界面该关了」两件事混成一件——而
用户当时可能正在看曲库、正在输入一条命令。现在读到 EOF 只是**连接状态变了**：

- `Connection` 是 `UiState` 上的一个**状态**（Live / Lost），不是通知。通知会过期，而这个
  事实在恢复之前一直成立；画在顶行（第 1 行），一个双宽字形，渲染测试因此逐字符断言而不是
  比字符串。
- 断线期间按的键**不丢**：进 outbox，接上以后按原顺序发出。排队的是请求而不是「待办」，
  所以顺序就是用户按键的顺序。
- 后台每 `DAEMON_RECONNECT_RETRY_MS`（500ms，比启动时的 25ms 慢——那一个是客户端等一个
  正在启动的 daemon，这一个是等一个已经证明自己会死的）重拨一次；拨不通就
  **每段故障期最多**用 `current_exe()` 拉起一个 daemon，避免一边重试一边把进程表刷满。
- 连不回来也**不拖垮客户端**：重试是后台任务，TUI 照常响应，用户可以正常 `q`。播放状态是
  真的丢了（进程真的死了），横幅是诚实的说法，不是掩饰。

### 文档对账

四份文档此前只有被本次改动**直接证伪的行**被改过，架构叙述整体仍写着拆分前的单进程样子。
这次按实测重写：`CLAUDE.md`（两个进程一个 binary、两种投递时机、读 socket 的单一属主、
MPRIS 一节、socket 测试串行化）、`DESIGN.md`（§2.1 两个角色、§2.3 MPRIS2、§2.4 生命周期、
§3.1–3.4 新增 IPC/daemon/Player/客户端句柄、§7 文件归属表、§8 持久化、§10 测试表、§11 结构、
§12 依赖）、`README.md`（桌面集成、`tmper status`、断线重连、XDG 位置与两份日志、FAQ、
技术栈）、`STATUS.md`（MPRIS 与重连进能力，删掉两条已被证伪的限制）。

数字都来自实测而不是上一版：测试数按文件逐个 `grep` 对账（**526 = 520 默认 + 6 设备门控**），
覆盖率跑 `cargo llvm-cov --all-features --workspace`：**89.96% 行 / 90.61% 区域 / 88.63% 函数**。
其中 `ui/widgets/visualizer_panel.rs` 从 **17.24% → 100%**：它是视图 `4` 的全屏频谱，而其余
渲染测试都传空的 `visualizer_data`，正好走进 `render_visualizer` 的提前返回——面板的主体
（柱的布局与逐列取色）此前**一行都没被执行过**。这是真缺口不是结构性债务，补了一个测试
（断言画出了柱、且颜色不止一种，这样「布局」和「取色」两处走岔会红）。

### 验证

自动：`cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test` 干净，
**520 通过 / 6 ignored**。

**留给你的（这里验不了）**：Plasma 媒体控件真的把它画出来、媒体键真的送过来——本机能验的
只到总线这一层（`busctl` 逐项核对过，见上一节）。另外「放够 5 分钟 daemon 自己退出」这条
只在把 `DAEMON_IDLE_EXIT_SECS` 临时改成 5 秒时验过，300 秒那一档没有真等过。

**已由用户确认（2026-10-03）**：真 Plasma 上媒体控件能看到曲目和封面，控制正常。
空闲退出 300 秒那一档仍未真等过。

## 追加（同日）：1.0.0

拆分四个阶段全部落地后，版本从 `0.1.0` 提到 `1.0.0`（`Cargo.toml` 一直是 0.1.0，从项目
第一天起没动过）。同时补上 `tmper --version`——clap 的 `version` 属性此前没写，于是
「每个程序都会回答的那个参数」在这里报 `unexpected argument`。测试断言的是
`ErrorKind::DisplayVersion` 且输出里含 `CARGO_PKG_VERSION`，所以它测的是用户看到的行为，
不是「字段存在」。

README 重写为中文主文档 + `README.en.md`，新增 `CHANGELOG.md`。README 顶部那张截图是
**渲染器的真实输出**：新增 `print_the_player_view`（`#[ignore]` 的，因为它不断言任何东西，
不该占一个不会失败的名额）驱动 `TestBackend` 画一遍播放器视图，逐格取字符贴进代码块。
手画的示意图是第二份实现，而第二份实现会悄悄和第一份不一致，所以宁可要一个能重新生成的。

（附带一个坑：一开始按字节切行，中日韩宽字符每行都被切歪。行必须从 `Buffer::content()`
按 `width` 切——一格一个 cell，宽字符占两列。）

**同日修正——不要给它上色。** 最初这个生成器输出带 SGR 的文本，因此贴的是 `ansi`
代码块，理由是「GitHub 会把 ANSI 渲染成彩色」。这是错的：GitHub 的 Markdown 不支持
`ansi`，它把每个 ESC 换成 U+FFFD，剩下的 `[0;38;2;…m` 原样留在图里。推上去之后把渲染
结果取回来验证，那段代码块里有 **2086 个替换字符**——截图成了一片乱码。`TestBackend`
就是渲染器本身，单元测试看不见这一点；只有走一趟 GitHub 的渲染管线才看得见。现在生成
器只输出纯文本，那个 `sgr()` 已经删掉，免得下次有人顺手把颜色加回去。

**顺带发现，未修**：开了封面图之后，播放器主视图**任何位置都不显示当前曲目名和艺术家**
（`player_view.rs` 里那两行只存在于「没有封面」的文字回退分支）。已记进 `STATUS.md` 的
近期计划，要不要改由用户定。
