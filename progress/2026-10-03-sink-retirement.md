# 2026-10-03 会话（续三）：测试套件的间歇性挂死——退役 sink 而不是 `stop()` 它

> 现象：`cargo test` 偶尔（约几十次里一次）不再结束。挂住的**不是**某个固定的测试——
> 每次挂死时报出的「正在运行」的测试都不同，因为真正卡住的是一条后台线程，而卡住的
> 运行时拖住的是当前恰好持有它的那个测试。查到最后是一个 4 行的行为差异，根因在 rodio
> 0.20.1 的 `Sink` 里。

## 一、定位：抓一份挂死现场的线程栈

挂死是间歇的，靠读代码猜不出来。做法是让 gdb 在挂死时把**所有线程**的栈打出来：

```bash
# /tmp/hunt2.sh：跑测试套件，等到进程不再有输出（或超时）时 kill -INT gdb
gdb -p <pid> -batch -ex "thread apply all bt 25"
```

（多行 `commands … end` 块必须写进 `-x` 脚本文件，`-ex "commands … end"` 不会生效——第一次
尝试就踩了这个坑，断点命令没装上，跑完一份空记录。）

栈里最关键的一帧：

```
#0  rodio::sink::Sink::sleep_until_end (self=0x7fffe0072e30) at rodio-0.20.1/src/sink.rs:325
#1  rodio::sink::Sink::append (self=0x7fffe0072e30) at rodio-0.20.1/src/sink.rs:113
...
#N  tokio::runtime::blocking::pool::BlockingPool::shutdown   ← Runtime::drop 在等这个任务
```

一条 `spawn_blocking` 的解码线程，永久停在 `append` 里面。而**另一条**线程（tokio 的
blocking pool 关闭路径）在等它结束，于是 run 测试的那个运行时永远 drop 不掉。

## 二、根因：`stop()` + 下一次 `append` = 永久阻塞

rodio 0.20.1 `Sink` 的三段代码连起来才是完整的故事（行号即 0.20.1 源文件）：

```rust
// sink.rs:312 —— stop() 只置标志位，什么都不清、什么都不叫醒
pub fn stop(&self) { self.controls.stopped.store(true, Ordering::SeqCst); }

// sink.rs:111-116 —— append() 开头的「等旧声音放完」
if self.controls.stopped.load(Ordering::SeqCst) {
    if self.sound_count.load(Ordering::SeqCst) > 0 {
        self.sleep_until_end();          // ← 阻塞点
    }
    self.controls.stopped.store(false, Ordering::SeqCst);
}

// sink.rs:324-328 —— 等的是「声音结束」信号；这个信号由每次 append 重新装上
pub fn sleep_until_end(&self) {
    if let Some(rx) = self.sleep_until_end.lock().unwrap().take() { let _ = rx.recv(); }
}
```

`recv()` 等的是队列里那个声音播完（`Done` 包装器发出的信号）。**在一个没人轮询的 Sink 上
它永远不会响**——headless（`Sink::new_idle()` 的队列输出端被丢掉）如此；真实设备上正常
情况下由 5ms 的 `periodic_access` 把当前源停掉、从而释放等待，但如果设备卡住也一样。

于是挂死的链条是：

```
AudioEngine::stop() / begin_session()
  → AudioOutput::stop_and_replace()
     → Sink::stop()              置 stopped 标志（旧 Sink）
  → 新 Sink 装上，主线程继续跑
  ...
  解码线程（还握着旧 Sink 的 Arc，尚未轮询到取消标志）
     → sink.append()             命中「stopped && sound_count > 0」
     → sleep_until_end() → recv()   ← 永久停在这里
  ...
  测试结束 → Runtime::drop → BlockingPool::shutdown(None)
     → 等这条还在跑的任务           ← 整个测试挂死
```

用 gdb 断点确认了这条链的真实性（`break rodio::sink::Sink::stop` + `break sink.rs:111`，
输出见 `/tmp/gdbstops.txt`）：

1. 第一个 `SINK_STOP` 来自 `begin_session` → `stop_and_replace`（`engine.rs:223`，
   即 `play_file_async` 换会话）。
2. 第二个来自 `AudioEngine::stop`（`engine.rs:415`），调用方是
   `app/handlers/library.rs:805` 的 `test_enter_track_plays_fixture`。
3. 随后的 `BLOCK_APPEND` 打出的 `p self` 是 **同一个指针** `0x7fffe0072e30`——
   被 `stop()` 过的那个 Sink，紧接着就被解码线程 `append` 了。

这解释了「每次挂死时正在跑的测试都不同」：卡住的是后台线程，而**被它拖住的是当时持有
那个 runtime 的测试**。

## 三、修法：退役旧 sink（丢引用），而不是 `stop()` 它

```rust
pub fn stop_and_replace(&mut self) {
    // 注意：这里没有 self.sink.stop()
    let new_sink = match &self.stream_handle {
        Some(handle) => Sink::try_new(handle),
        None => Ok(Sink::new_idle().0),
    };
    match new_sink {
        Ok(new_sink) => self.sink = Arc::new(new_sink),
        Err(e) => tracing::error!("Failed to create new sink (audio may be unavailable): {e}"),
    }
}
```

为什么丢引用就够，而且静音机制一模一样：

- `Drop for Sink`（`sink.rs:356-365`）做两件事——`set_keep_alive_if_empty(false)` 和
  `stopped.store(true, Relaxed)`，和 `stop()` 置的是**同一个标志位**。
- 当前正在播的源由它自己的 5ms `periodic_access` 看到标志位后 `src.stop()` 结束；
  队列清空后 `keep_alive_if_empty == false` 让队列直接结束，而不是继续播放静音
  （`queue.rs:235-242`）。
- **区别只在「标志位何时落下」**：`stop()` 是立刻，丢引用是「解码线程下一次取消轮询时」
  （喂数据时 10ms，收尾时 20ms），代价是一次最多 20ms 的残留声音——听不出来。
- 而且 `Drop` 只在**最后一个** `Arc` 消失时运行，所以这个标志位**不可能**落在一次
  进行中的 `append` 下面——这正是原方案的危险所在。

## 四、回归测试（`src/audio/output.rs`）

```rust
#[test]
fn a_retired_sink_still_accepts_appends() {
    let mut output = AudioOutput::new_headless();
    let session_sink = output.sink_arc();
    session_sink.append(silence());     // stop() 的陷阱需要队列里先有东西
    output.stop_and_replace();
    // 从普通线程 append：卡住的话这里超时，测试干净地失败
    ...
    assert!(done_rx.recv_timeout(Duration::from_secs(2)).is_ok(), "...");
}
```

两个方向都验过：把 `self.sink.stop()` 加回去，测试在 **2.00s** 干净失败并打出
`appending to the retired sink blocked inside rodio's sleep_until_end`；去掉则通过。

**为什么测试放在 `output.rs` 而不是引擎层**：第一版写成了引擎级的 `#[tokio::test]`
（真的起解码线程），结果它复现了 bug——然后**跟着一起挂死**（`Runtime::drop` 等那条
卡住的任务），`timeout 180` 被 kill，什么都没输出。回归测试自己不能挂：现在这版没有
tokio 运行时、没有解码线程，卡住只是 `recv_timeout` 到期。

## 五、验证

- `cargo fmt --all && cargo clippy -- -D warnings && cargo test`：**364 passed + 6 ignored**。
- `/tmp/stress.sh`（连跑 30 遍完整套件）：**30/30 通过，无一次超时**。
  修复前这个脚本几十次里会挂一次。
