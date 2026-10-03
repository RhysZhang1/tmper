# 2026-10-03 会话（续）：全项目评审缺陷修复系列

## 背景

承接同日的 `2026-10-03-xdg-streaming-merge.md`。合并完成后，本次针对评审列出的缺陷逐项修复，
按严重度分三批。测试数 **181 → 223**（+6 设备门控），行覆盖率 **75.55% → 80.09%**。

## P0：崩溃与错误行为

### 1. 文件浏览器索引错位（`handlers/browser.rs`）

`Enter` 用合并列表 `fs_items` 的行号去索引 `dirs` / `audio_files` —— 后两者各自只保留同类条目，
行号只在巧合下一致。目录 `[Dir(aaa), Audio(bbb.mp3), Audio(ccc.mp3)]` 下：第 1 行打开 `ccc.mp3`，
第 2 行索引越界 panic。既有测试的临时目录都只有单一类型，所以一直没暴露。

修法不是重新对齐数组，而是**去掉第二个事实来源**：`FsItem` 现在自带 `path`，
`FileBrowserState::{dirs, audio_files}` 删除。`refresh_file_browser` 同时改用
`scanner::is_audio_file`，不再自带一份扩展名表。

### 2. 输入层丢键 / 打字时删歌 / 打字时退出（`input/handler.rs`、`app/mod.rs`）

`KeyHandler` 在非组合键时**丢弃当前事件**（`d` 后按 `j`，`j` 消失），挂起键也没有超时释放；
`app/mod.rs` 只对歌单重命名做了旁路，而 DESIGN.md §6.2 声称旁路覆盖"搜索/插入模式"。
后果：搜索框里打 `q` 退出程序、打 `dd` 删除当前曲目、含 `g`/`d` 的查询丢字符。

另外 `starts_sequence` 不看修饰键，`Ctrl+D`（半页滚动）被当作 `d` 前缀，连按两次即 `dd` 删歌。

- `process()` 改为返回按序的事件序列（而非单个 `Option`），释放的挂起键与当前键都不丢；
- 新增 `flush_expired()`，事件循环每 tick 调用，挂起键由时间释放；
- 新增 `discard_pending()`，进入文本输入时丢弃挂起键；
- 序列只认**无修饰键**的 `g`/`d`；
- `App::in_text_entry()` 覆盖命令/队列搜索/曲库搜索/歌单重命名四种模式。

该文件此前 **0 测试、19% 覆盖**，现在 15 个测试、97.66% —— 所有缺陷都是先写失败测试复现的。

### 3. 主题解析 panic（`ui/theme.rs`）

`parse_hex` 用字节长度 `s.len() != 6` 校验后按字节切片。`"€€"` 是 6 字节 2 字符，通过校验后
`&s[0..2]` 切在字符中间直接 panic；而 `Theme::load` 在 `App::new` 里调用，用户主题文件的笔误
就是启动崩溃。改为要求 ASCII（hex 本就是 ASCII）。

### 4. `prev_track` 越界（`app/playback.rs`）

`active_playlist_song` 在一个视图设置、另一个视图消费，歌单被删改后不会重新指向，而
`prev_track` 直接 `songs[prev_song]`。`next_track` 用取模、`on_track_ended` 有 `.min()` 保护，
只有它没有。改为先钳制游标。

## P1：正确性

### 5. 频谱滞后 ~0.7 秒（`app/playback.rs`）

`InstrumentedSource` 把新样本 `push_back`、满容量时 `pop_front`，队首是**最旧**的样本；
FFT 线程却用 `buf.iter().take(fft_size)` 取队首 = 窗口里最旧的一段。默认 32768 缓冲下，
分析的是 `(32768-2048)/44100 ≈ 0.70s` 之前的音频。窗口选择提取为纯函数 `latest_window`，
使不变量可直接测试（旧测试只断言 `!lines.is_empty()`）。

### 6. 频谱配色与柱错位（`visualizer/render.rs`、`ui/widgets/visualizer_panel.rs`）

`render_bars` 用 `left_pad` 居中，面板却用整行下标除以柱宽算颜色，两者相差 `left_pad` 列；
默认 100 列 + `num_bars=64` 时错位 18 列。布局现在只有一份实现（`bar_layout` / `bar_at_column`）。
同批修掉同函数里两个相关缺陷：行填充用**字节数**当列数（块字符 3 字节，填充实际从不生效）、
`num_bars > width` 时行会超出宽度（现在绘制到宽度即停）。

### 7. 曲库三栏不滚动（`ui/views/library_view.rs`）

`render_panel` 把全部条目交给 `List` 裁剪，从不读 `scroll_artists/albums/tracks` ——
处理器一直在维护、渲染器从不使用，选中项移出面板即消失。现在按窗口切片，
并在渲染层保证选中项可见（搜索栏出现时面板会变矮，用终端高度算的偏移会差几行）。

### 8. 封面：尺寸变化与搜索覆盖层（`ui/cover/mod.rs`）

- Kitty 路径只比较封面身份，不比渲染区域 → 终端 resize 后图片停留在旧位置与旧尺寸
  （SIXEL 路径本来就比了）。补 `last_kitty_rect`。
- `CoverParams` 没有搜索状态 → 按 `/` 后结果面板与持久图形层重叠，图片盖住文字。
  新增 `search_active`，与帮助/命令覆盖层同等对待。
- 协议选择改为构造时解析一次（`kitty_available`，与既有 `chafa_available` 同形），
  并删除 `render_kitty` 内重复的环境检测 —— 它使整条 Kitty 路径无法被测试。

## P2：工程卫生

- **`frame_rate` 半活**：它设置 tick 间隔，但上面压着硬编码的 50ms 节流，20fps 以上的配置值
  全部无效，而文档把它描述为可视化帧率。节流现在由同一个值推导。
- **配置文件浮点伪影**：`f32` 序列化时被提升为 `f64`，`0.1` 写成 `0.10000000149011612`。
  用 `serialize_f32_rounded` 在 `f64` 空间取整（先取整 `f32` 只会复现伪影）。
- **`parse_key_str` 按字节计数**：非 ASCII 单字符（如 `"↑"`）跳过单字符分支，静默变成空格，
  与播放/暂停冲突。改为按字符计数，未识别名称改为告警而非静默。
- **`SavedState` 缺字段级容错**：state.json 少任一字段则整体反序列化失败，音量/循环/歌词偏移
  一起丢失。改为逐字段可选，部分文件仍恢复其包含项。
- **依赖**：移除 crossterm 的 `event-stream`（burst 输入改造后已成死特性，只白拖 futures 三件套）；
  tokio 从 `features = ["full"]` 裁剪为实际使用的四项。
- **Clippy 升级到 `--all-targets`**（CI 同步）：测试模块里积累了 12 处 lint，正是因为此前不检查
  测试代码。全部为机械修复。

## 验证

```
cargo fmt --all -- --check                    ✅
cargo clippy --all-targets -- -D warnings     ✅ 零警告（此前 12 处）
cargo test                                    ✅ 223 passed; 0 failed; 6 ignored（无需声卡）
cargo llvm-cov --all-features --workspace     ✅ 80.09% 行（82.13% 函数、79.16% 区域）
```

## 已知遗留

- `library/scanner.rs` 30%：生产扫描器的错误路径（walkdir 失败、元数据读取失败）仍无单测。
  该路径的正确性由 `complete` 标志的调用方测试间接保证。
- `paths.rs` 29%：非测试分支在 `cfg(test)` 下不会编译，覆盖率数字天然偏低。
- `audio/engine.rs` 48%：合并带进来的会话/背压重写，取消与错误路径覆盖不足。
- `ui/views/player_view.rs` 72%：封面渲染分支与歌词窗口逻辑。
- `last_track_path` 只写不读（按设计仅作记录）；若确认无人使用可删除。
