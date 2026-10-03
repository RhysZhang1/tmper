# 2026-10-03 会话（续）：封面编码搬进进程，chafa 依赖退出

> 承接 `2026-10-03-chafa-cell-units.md`。上一条把几何对齐修好了，代价是引入
> `chafa_cell_px()` 这个「换算成 chafa 的格」的中间层——它成立的前提是 tmper 能准确
> 预判 chafa 会读到哪个单元格。这一条把这个前提整个删掉：SIXEL 改为进程内编码，
> 请求的栅格就是盒子的像素，没有第二个进程、没有第二个单元格概念。
> 调查阶段的数据（编码质量、耗时、终端能力矩阵）在
> `2026-10-03-encoder-and-terminal-compat.md`。

## 决策

用户问：「换封面渲染能不能代替掉 chafa 依赖，当前效果还能不能完美实现？」两条都在
实测里得到肯定回答，于是按「先换编码器、再做兼容性」的顺序实施。

- **质量不降**：chafa 的 `-c full` 在 SIXEL 路径上并不是真彩——它只发 255 个颜色寄存器
  （SIXEL 用 8 位寻址颜色，256 是格式自身上限）。换成 `icy_sixel`（quantette：Wu 量化 +
  Floyd–Steinberg 抖动，`max_colors = 256`）后，三张测试图的 MAE 全部更低
  （3.70 / 1.33 / 2.40 对 chafa 的 4.26 / 2.12 / 3.47），PSNR 高 0.07–3.3 dB。
- **更快**：同尺寸 264×270，chafa 子进程 38.2 ms/次（含 spawn），icy 纯编码 5.5 ms，约 7×。
- **几何归零**：栅格尺寸成为入参，不再取决于「chafa 通过 `/dev/tty` 看到了什么」。
  `chafa_box()` / `chafa_cell_px()` 及 Konsole 上那类 0.8×0.75 缩放问题整类消失。

## 实现

`Cargo.toml` 增加 `icy_sixel = "0.5"`（纯 Rust，MIT/Apache-2.0）。`image` 升为两条路径
共用：半块字符与 SIXEL 都走 `image::load_from_memory` + `resize_exact(..., Lanczos3)`。

`SixelEncoder` trait 的语义从「格」改成「像素」：

```rust
fn encode(&self, cover: &[u8], px: (u16, u16)) -> Result<Vec<u8>, String>;
```

`IcySixelEncoder` 就是解码 → Lanczos3 缩放到 `px` → `sixel_encode`。`SIXEL_MAX_COLORS`
与 `SIXEL_DIFFUSION`（0.875，编码器自带默认值，适合照片）落在 `constants.rs`。调用方
`render_sixel` 只做一件事：把矩形的格数乘上 `cell_px`。

字段随语义改名：`chafa_available` → `sixel_available: Option<bool>`，`chafa_sixel_cache`
→ `sixel_cache`，`render_chafa` / `reset_chafa` → `render_sixel` / `reset_sixel`，
`chafa_failed` 类 sticky 标志同理。`native_active()` 现在读 `sixel_cache.is_some()`。
`which_chafa()`、`chafa_box()`、`chafa_cell_px()` 删除。

### `sixel_available: Option<bool>` 的由来

`CoverRenderer::new()` 在 `App::new()` 里跑，早于 `App::run()` 中的终端探测，此刻没有人
问过终端。`None` 表示「渲染时再问 `terminal_caps()`」，`Some(b)` 供测试注入。

## 图形能力判定：DA1 成为探测的收尾

旧逻辑里 SIXEL 的开关是「`chafa` 二进制存在吗」——终端支不支持完全不问，而 `--probe off`
又关掉了 chafa 自己的探测。于是 chafa 一旦可执行，tmper 就写出 DCS 载荷并把半块字符画
撤下，在不支持 SIXEL 的终端（VTE 系、Alacritty、未开 passthrough 的 tmux）上封面区是
**空白**而不是字符画兜底。

新探测把启动时的查询并成一趟往返（`probe_terminal_once`，预算 80 ms）：

```
\x1b[16t      单元格像素（Konsole 走这条）
\x1b[14t      窗口像素
\x1b[18t      窗口格数        → 14t ÷ 18t，给不答 16t 的终端
\x1b[c        DA1：主设备属性，参数里含 4 表示支持 Sixel
```

DA1 放在**最后**并充当屏障：它是所有终端都会回答的查询，答完即收工，不必等满预算。
参数按**数值**匹配，所以 `\x1b[?62;24;1c` 里的 `24` 不会被读成 `4`。
`parse_da1_sixel` 区分「还没有完整回复」（`None`）与「回复里没有属性 4」（`Some(false)`）。

`TerminalCaps { cell_px: Option<(u16,u16)>, sixel: bool }` 存在 `OnceLock` 里；没有 tty
或终端全程沉默时是 `Default`（`sixel: false`），封面走半块字符。

crossterm 0.28 会把迟到的 `CSI ? … c` 解析成 `InternalEvent::PrimaryDeviceAttributes`，
它没有对应的公开 `Event`，因此被静默丢弃——迟到的 DA1 回复不会变成伪按键（这一点比
`CSI … t` 更安全：后者会解析失败并被 `if let Ok` 丢掉，但类型上更接近错误路径）。

## 实测验证

- **端到端（pty）**：106×51 格、`TIOCSWINSZ` 报 8×15 的 Konsole 形状下，33×18 的封面盒
  现在得到 **264×270** 的载荷（52,578 字节；chafa 时代同一盒子是 208×195 / 108,321 字节）。
- **能力门**：DA1 回复里没有属性 4 的终端收到 **0 字节**图形载荷；全程不回复的终端同样
  0 字节。两条都退回半块字符画。
- **画质**：解码 tmper 自己写出的载荷与源图对比，MAE 3.48 / PSNR 31.81 dB。

## 测试与覆盖率

`ui/cover/mod.rs` 33 个测试（原 29 个）：`chafa_box_*` 四个改为编码器语义
（`the_encoder_is_asked_for_the_rects_pixels`：10×20 时 330×360、8×15 时 264×270；
`a_degenerate_rect_never_asks_for_zero_pixels`），新增
`the_encoder_emits_the_requested_raster`（真 PNG → 头部 `1;1;24;40`）、
`the_encoder_rejects_bytes_that_are_not_an_image`，探测侧新增
`probe_stops_at_the_da1_barrier`、`probe_reports_no_sixel_when_da1_omits_it`、
`da1_sixel_attribute_is_read`、`da1_without_the_attribute_is_a_no`、`an_incomplete_da1_reply_is_not_an_answer`、
`da1_is_found_among_other_traffic`。`query_terminal_on` 仍是 fd 入参，测试用管道驱动整条读循环。

- `cargo fmt --all` / `cargo clippy --all-targets -- -D warnings` 干净
- `cargo test`：**353 passed / 0 failed / 6 ignored**
- `cargo llvm-cov --all-features --workspace`：行 **88.54%**（函数 88.96%、区域 89.52%），
  其中 `ui/cover/mod.rs` 87.59% → **92.51%**

## 仍然没做的（下一步）

- **Kitty 路径仍然只看环境变量**（`KITTY_WINDOW_ID` 等）。协议探测（`CSI ? u` 或 1×1
  `_G` 探针）是下一步兼容性修复的内容。
- **tmux / screen 下没有 DCS passthrough 包装**，图形载荷会被复用器吞掉；目前靠能力门
  之外的因素退回字符画，属于「碰巧正确」而不是设计。
