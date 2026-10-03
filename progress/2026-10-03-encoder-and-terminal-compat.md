# 2026-10-03 会话（续）：封面编码器能不能去 chafa、MPRIS 是什么、终端兼容性边界

> 三个问题的实测与代码核对记录。**本文只做调研与测量，没有改动运行时代码**（唯一的代码
> 事实是「当前实现没有 SIXEL 能力探测」，这是读出来的，不是改出来的）。

## 一、进程内 SIXEL 编码器能否取代 chafa

### 结论

能，而且**质量不吃亏**。候选是 `icy_sixel` 0.5.1（libsixel 的纯 Rust 移植，编码器基于
`quantette`：Wu 量化 + Floyd–Steinberg 抖动，调色板上限 256），ratatui-image 用的就是它。

### 关键实测：chafa 的 `-c full` 在 SIXEL 路径上根本不是真彩

SIXEL 的经典调色板上限是 256 个颜色寄存器，chafa 无论 `-c full` 还是 `-c 256` 都只发
**255 个** `#N;2;r;g;b` 定义（实测：mandelbrot、渐变色块、testsrc2 三张图一致；全部是
type-2 RGB 定义，没有漏掉 HLS 形式）。也就是说两条路的**色彩天花板相同**，差别只在
「谁的量化和抖动更好」。

### 质量 A/B（同一张图、同一栅格 264×270）

方法：原图 Lanczos 缩到 264×270 作为基准 → 让 chafa 在 **pty**（`TIOCSWINSZ` 报 8×15 像素
单元格，即 Konsole 的真实几何）里跑 tmper 的那套参数 `--probe off -f sixels -c full
--stretch -s 33x18` → 自己写的 SIXEL 解码器把载荷解回 RGB → 与基准逐像素比。icy 一侧用
同样的 264×270 输入、`max_colors=256`、默认扩散 0.875。

| 测试图 | chafa `-c full` | chafa `-c 256` | icy_sixel 256 |
|--------|-----------------|----------------|---------------|
| mandelbrot（照片型） | MAE 4.26 / PSNR 32.16 dB | MAE 9.04 / PSNR 25.83 dB | **MAE 3.70 / PSNR 32.23 dB** |
| testsrc2（色块线条） | MAE 2.12 / PSNR 35.87 dB | MAE 1.73 / PSNR 33.97 dB | **MAE 1.33 / PSNR 36.80 dB** |
| 双色渐变（考条带） | MAE 3.47 / PSNR 34.62 dB | MAE 9.89 / PSNR 26.46 dB | **MAE 2.40 / PSNR 37.92 dB** |

（重采样噪声底：image crate 的 Lanczos 与 ffmpeg 的 Lanczos 之间 MAE 0.03/255，可忽略。）

三张图 icy 都赢，且 PSNR 高 0.07–3.3 dB。chafa 的 `-c full` 明显好于它自己的 `-c 256`
（后者大概退回了固定调色板），但仍是 icy 更好。

### 速度

同一张 264×270：chafa 子进程 **38.2 ms/次**（spawn + 编码），icy 纯编码 **5.5 ms**，加上
PNG 解码与缩放端到端 **10.1 ms**。

### 换成 icy 之后能顺带解决的事

- **几何不再靠猜**：栅格尺寸变成 tmper 传进去的参数，不再依赖「chafa 通过 `/dev/tty`
  看到了什么单元格」。`chafa_cell_px()` 这一类换算及其全部边界情形随之消失。
- 没有子进程：无 PATH 依赖、无 spawn 失败路径、无 chafa 版本差异、无 5 秒探测陷阱的
  历史包袱（`--probe off` 那条注释可以退休）。
- 单文件分发更纯粹（虽然 chafa 从来不是必需依赖）。
- icy 自带解码器，tmper 可以解自己的输出做「请求 vs 实得」校验。

### 代价与风险

- 仍然需要**自己判断终端支不支持 SIXEL**——icy 只负责编码，判断能力是第三节的事。
- 透明度语义（SIXEL P2=1）、调色板/抖动参数需要按视觉口味调一轮。
- crate 很年轻（0.5.x）；缓解因素是 ratatui-image 依赖它。

### 建议

`SixelEncoder` trait 已经是可注入的，替换被限制在一个实现里：加一个 `IcyEncoder`，
chafa 实现保留作诊断/对照，按真实截图 A/B 后再决定默认值。

## 二、MPRIS 到底是什么；关掉 TUI 之后怎么继续控制

### 它是什么

会话 D-Bus 上的一组标准接口：`org.mpris.MediaPlayer2`（Raise/Quit/Identity）与
`org.mpris.MediaPlayer2.Player`（方法 `PlayPause`/`Next`/`Previous`/`Seek`/`SetPosition`，
属性 `PlaybackStatus`/`Metadata`（标题、艺术家、专辑、`mpris:artUrl`、时长）/`Position`，
信号 `PropertiesChanged`/`Seeked`）。**关键在总线名的所有权**：谁 `RequestName` 成功，
谁就是 KDE Plasma 的媒体小组件、锁屏、媒体键（KDE 的 Media Controller）、`playerctl`、
GNOME 的 MPRIS 扩展所控制的对象。应用一退出，名字释放，控件立刻消失——它们并不「记住」
这个播放器。

### 别的项目怎么处理「关掉界面还要能控制」

| 项目 | 播放进程 | 界面 | 结果 |
|------|----------|------|------|
| mpd + ncmpcpp/rmpc | `mpd` 守护进程，音乐在它里面 | 客户端，随时断开重连 | 关界面不影响播放；MPRIS 由 `mpDris2` 一类桥接补 |
| termusic | workspace 拆出 `playback`/`server`，TUI 是客户端 | 可脱离 | 关 TUI 不停播，重开接管 |
| spotify-player | `--daemon` 常驻，自己持有 MPRIS | TUI 客户端 | 关 TUI 仍可被媒体键/桌面控件控制 |
| **tmper（现状）** | **和 TUI 同一个进程**（rodio Sink 在 App 里） | 无 daemon | **退出即停播**，没有任何东西可以后续控制 |

### 所以两件事要分开

1. **进程内加 MPRIS**（便宜，约 200 行，`mpris-server`/`zbus`）：tmper 开着的时候，Plasma
   媒体小组件、媒体键、`playerctl` 都能用。这解决「在 KDE 里控制」，**不解决**「关掉 TUI
   继续放」。
2. **拆出常驻播放进程**（贵，动架构）：音频、队列、状态搬进 daemon，daemon 持有 MPRIS
   总线名，TUI 变成可随时断开/重连的客户端。这才是「关掉 TUI 音乐继续播放、之后再控制」
   的完整答案。代价是 tmper 目前把引擎、状态、曲库全放在 TUI 进程里，`state.json` 只是
   「下次启动恢复」，不是「此刻的真相源」。

建议顺序：先 1 后 2。1 的收益（KDE 媒体键、桌面小组件）立刻可见；2 应该作为独立计划，
不要顺手做。

## 三、KDE 之外 / Konsole 之外的兼容性

### 协议 × 终端

| 协议 | 支持者 | 不支持者 |
|------|--------|----------|
| Kitty 图形 | kitty、WezTerm、Ghostty；Konsole ≥22.04（**部分**实现，有渲染残影报告） | Alacritty、VTE/GNOME Terminal、foot |
| SIXEL | xterm、mlterm、foot、WezTerm、Konsole ≥22.04、iTerm2、contour、mintty | VTE/GNOME Terminal、Alacritty、kitty（上游明确不做） |
| iTerm2 内联图 | iTerm2、WezTerm；Konsole ≥22.04 | 多数其余终端 |
| 半块字符 | 全部 | —— |

跨桌面环境本身不是变量（协议支持取决于**终端**，不取决于 GNOME/KDE/XFCE）；真正的变量是
「用户装的是哪个终端」，而默认终端在 GNOME（GNOME Terminal / VTE 系）和 XFCE
（xfce4-terminal，同样 VTE）上都**不支持 SIXEL**。

### tmper 当前在那些终端上会怎样（代码事实）

- Kitty 路径：只看环境变量（`KITTY_WINDOW_ID` / `TERM_PROGRAM=WezTerm` /
  `GHOSTTY_RESOURCES_DIR`），不查询终端，因此在 Konsole 上永远不会命中（Konsole 也不设
  这些变量），tmux 里因为显式排除也不会命中。
- SIXEL 路径：`which_chafa()` **只检查 chafa 这个二进制能不能跑**，完全不问终端支不支持
  SIXEL；`--probe off` 又关掉了 chafa 自己的探测。于是 chafa 一旦存在，tmper 就会写出
  DCS 载荷，并在 `chafa_sixel_cache` 有值后把 `native_cover` 置位
  （`src/app/mod.rs:364`），**让半块字符封面让位**。
- 后果：在 GNOME Terminal / Alacritty / 未开 passthrough 的 tmux 里，封面区**什么都
  没有**——不是我们的字符画兜底，而是空白。屏幕不会花（符合规范的解析器会把不认识的 DCS
  一直吞到 ST），但比「没有原生图像」更糟。

这是当前终端兼容性的真正缺口：不是「画得不够好」，是**没有能力探测**。

### 修法（复用已有的启动探测机制）

`probe_cell_px_once` 已经具备「raw 模式下写查询、限时 80ms 读回复、不吞按键」的全部零件，
能力探测可以搭同一趟车：

- **SIXEL**：`CSI c`（DA1）→ 回复 `CSI ? Ps;… c`，参数里含 `4` 即支持（xterm 的 ctlseqs
  定义了 `Ps = 4 ⇒ Sixel graphics`；foot 实测回 `\x1b[?62;4;22;28;52c`）。DA1 还是个天然
  **屏障**：它之前的查询回复都已到达。备用信号 `XTSMGRAPHICS`（`CSI ? 1 ; 1 ; 0 S`）。
- **Kitty 图形**：`CSI ? u`（回 `CSI ? flags u`，bit 3）或直接发一个 1×1 查询图
  （`_Gi=1,s=1,v=1,a=q`）看是否有 OK/错误回复；ratatui-image 就是这么做的。
- **tmux/screen**：按 `$TERM` 前缀识别，载荷用 DCS passthrough 包裹（`\x1bPtmux;…`，
  需要 tmux 3.4+ 且 `allow-passthrough on`），否则直接走字符画。
- **原则**：在载荷被确认「这个终端能显示」之前，不要撤掉半块字符画。现在 `native_active`
  一置位就撤，等于把「chafa 存在」当成了「终端支持」。

另一条路是直接采用 ratatui-image 的 `Picker::from_query_stdio()`（多协议探测 + 超时 +
tmux 处理 + 单元格尺寸一把拿全），代价是封面管线要按它的 `StatefulProtocol`/`Resize`
模型重写。**先补探测、后决定要不要整体换**更稳。

## 复现方式

- chafa 侧必须在 **pty** 里跑（管道下它会退回 10×20 的兜底单元格）：`TIOCSWINSZ` 设
  `rows=18, cols=33, xpixel=264, ypixel=270`，参数与 tmper 一致。
- icy 侧：`EncodeOptions { max_colors: 256, diffusion: 0.875, quantize_method: Wu }`，
  `sixel_encode(rgba, 264, 270, &opts)`。
- 解码比对：最小 SIXEL 解码器（`#N;2;r;g;b` 定义、`#N` 选色、`!n` 重复、`$` 回行首、
  `-` 下移 6 像素、`?`–`~` 六位掩码），解回 RGB24 后与 Lanczos 基准逐像素算 MAE/PSNR。
