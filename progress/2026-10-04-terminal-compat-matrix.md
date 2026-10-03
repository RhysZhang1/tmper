# 2026-10-04 终端兼容性实测：三层回退在五种终端上各画了一次

> 这是一次**只测量、不改运行时代码**的验证。产出是一张实测表、两条文档订正，以及一个
> 顺带发现的日志缺陷。前一天的调研（`2026-10-03-encoder-and-terminal-compat.md` 第三节）
> 预判了各终端的支持情况，本文是对那份预判的核对。

## 为什么要做

封面的三层回退（Kitty 协议 → SIXEL → 半块字符）在这台机器上**只跑过第一层**。本机唯一的
终端是 Konsole 26.08，它答 Kitty；于是 SIXEL 编码器和半块字符回退从来没在真实终端里执行过。
`STATUS.md` 里「Alacritty 两种都不支持」当时是照抄文档的断言，不是量出来的。

## 方法

装四个终端（kitty 0.48.2 / foot 1.28.0 / xterm 411 / alacritty 0.17.0，均在 `extra`，
合计约 76 MiB），每个跑一次 tmper，取**两个互相独立的信号**：

| 信号 | 回答的问题 | 来源 |
|---|---|---|
| 探测日志 | tmper **选了**哪一层 | `terminal graphics: sixel … (DA1), kitty … (a=q)`，读自 `$TMPER_STATE_DIR/tmper.log` |
| 截图 | 那一层**真的画出来了吗** | `spectacle -b -n -a`（活动窗口），再用 Read 看 PNG |

两个都要，因为「终端回答支持、面板却是空白」正是唯一值得担心的失败模式，而它单独看任何一个
信号都发现不了——日志会说选对了，肉眼扫一眼窗口也可能以为只是没封面。

测试素材是现造的，因为 `tests/fixtures/` 三个文件都没有内嵌封面，而封面是唯一走原生图形的
东西：60 秒正弦波 + 内嵌 PNG，两张图分别是 640×480 的 SMPTE 彩条（测宽高比自适应：4:3 且
左右不对称，缩放错、翻转、拉伸都看得出来）和一张对角渐变。

**渐变那张是关键。** 彩条是大片纯色，半块字符能把纯色块画得跟光栅一样像，光看彩条分不出
SIXEL 和字符画。渐变则相反：半块字符只能画成单元格阶梯，SIXEL 是连续的。没有这一步就只是
「看起来画出来了」，而不是「确实是 SIXEL 画的」。

隔离：`TMPER_*` 全部指向 `/tmp/tmper-ttytest/tree`，并在解析后重新校验路径，绝不碰真实的
`~/.local/state/tmper`。

## 结果

| 终端 | DA1 报 SIXEL | `a=q` 报 Kitty | tmper 选择 | 实际渲染 |
|---|---|---|---|---|
| Konsole 26.08.1 | 是 | 是 | **Kitty**（顺序优先） | ✓ 光栅 |
| kitty 0.48.2 | 否 | 是 | **Kitty** | ✓ 光栅 |
| foot 1.28.0 | 是 | 否 | **SIXEL** | ✓ 光栅 |
| xterm 411（`-ti vt340`） | 是 | 否 | **SIXEL** | ✓ 光栅 |
| xterm 411（默认） | 否 | 否 | **半块字符** | ✓ |
| alacritty 0.17.0 | 否 | 否 | **半块字符** | ✓ |

六种配置全部按预期渲染。单元格像素尺寸五家都答了 `CSI 16 t`（konsole 8×15、kitty 15×22、
foot 5×17、xterm 6×13、alacritty 8×23），所以宽高比自适应这一路拿到的都是真实数字，没有
走到 10×20 的兜底。

### 三条结论

1. **SIXEL 路径首次执行并通过。** foot 与 `xterm -ti vt340` 的封面是连续色调的渐变，带
   `icy_sixel`/`quantette` 的抖动纹理；对照 alacritty 的同一张图能清楚看到单元格网格。
   两条独立实现都吃下了 tmper 的编码输出。
2. **Kitty 载荷在第二个实现上得到验证。** 此前只有 Konsole 解析过这些字节——而 Konsole
   向来以「部分实现」著称，单靠它无法区分「载荷符合规范」和「Konsole 恰好容忍」。kitty
   0.48 是协议的参考实现，它渲染成功意味着 `f=100` 必须是真 PNG、`p` 是放置 id 而非像素
   偏移、分块传输只有首块带命令头这三条规则都真的写对了。
3. **xterm 的答案取决于启动参数，而且它答得没错。** SIXEL 是 VT340 的特性；Arch 的 xterm
   **编译进了** sixel（`strings /usr/bin/xterm` 能找到相关符号），但按终端类型决定要不要在
   DA1 里上报，默认答「不支持」。`xterm -ti vt340` 一问就答「支持」，封面立刻变成光栅。
   所以默认 xterm 上的半块字符**不是 tmper 的解析 bug**，是探测如实采信了终端当下的回答——
   正是「只采信终端的回答」这条设计想要的结果。

## 顺带订正的文档错误

两份 README 都把 **xterm 列在 SIXEL 终端**里（`README.md:116,407`、`README.en.md:128,391`），
照上面的实测这是错的：默认 xterm 走的是半块字符。同时 Konsole 被列进了 SIXEL 那一档，但它
两个都答、按顺序永远走 Kitty，列在 SIXEL 里只会让人以为它「用不了 Kitty」。两处都已改，
并补上了实测表和 `-ti vt340` 的说明。`STATUS.md` 里那句「VTE 系、Alacritty 两种都不支持」
也拆开了：Alacritty 现在实测过，VTE 系本机没装、仍是未实测的转述。

## 顺带发现的缺陷（已修）

**任何一次性控制命令都会截断 `tmper.log`。** `main.rs` 用 `File::create` 打开客户端日志，
所以 TUI 正跑着的时候执行 `tmper status` / `tmper pause`，会把那个正在运行的 TUI 的日志清空，
只留下该命令自己的「tmper starting...」。

这是本次测量**第一次跑就踩到的**：`probe.sh` 收尾时用 `tmper quit` 收掉隔离的 daemon，结果
日志里那条 `terminal graphics:` 被它抹掉了，只剩下一条时间戳比 daemon 启动还晚十秒的
`tmper starting...`。当时是先把日志复制出来再 quit 绕过的。

影响是真实的：README 教用户 `grep "graphics" ~/.local/state/tmper/tmper.log` 来查探测结果，
而在跑过一次控制命令之后这个文件里已经没有那条了。（CHANGELOG 里「日志分流」修的是 daemon
与客户端互相截断；两个**客户端**之间仍然如此。）

**修法：改成追加，并补一个上限。** `main.rs` 新增 `open_log()`，两份日志都用
`OpenOptions::create(true).append(true)` 打开，不再有 `File::create`。追加把「唯一约束文件
大小的东西」也一并去掉了，所以加了 `LOG_MAX_BYTES`（1 MiB）：超限时把旧文件**改名**为
`<name>.log.1`，新文件从零开始。

**轮转必须是改名，不能是截断**——这是这个修法里唯一需要想清楚的地方。正在运行的 TUI 握着
旧文件的 inode，改名之后它继续往那个 inode 写，内容一条不丢；如果轮转写成截断，那就在上限
处把刚修掉的 bug 又请了回来：某个客户端会在另一个客户端正跑着的时候清空它的日志。改名让
「谁都不毁掉别人的记录」这条性质对**上限**也成立，而不只是对启动时刻成立。

三个测试钉住行为（`src/main.rs` 的 `tests`）：追加不丢已有内容、超限被改名而不是截断、
未超限不动文件。测试直接打 `open_log` 而不是 `init_logging`，因为后者要初始化全局
subscriber，一个进程只能来一次。

## 复现

```bash
# 素材：带内嵌封面的测试曲（fixtures 里三个文件都没有封面）
ffmpeg -f lavfi -i "sine=frequency=440:duration=60" -i cover.png \
  -map 0:a -map 1:v -c:a flac -c:v png -disposition:v attached_pic probe.flac

# 起终端（TMPER_* 指向 /tmp 下的隔离树），等 9 秒，截活动窗口，再读日志
foot -T tmper-probe-foot /tmp/tmper-ttytest/run.sh
spectacle -b -n -a -o shot.png
grep -a graphics "$TMPER_STATE_DIR/tmper.log"
```

两个本轮才踩到的坑，值得留给下次：

- **`spectacle` 只能截活动窗口、光标下的窗口、区域或全屏，没有「按窗口 id 截」。** 而脚本
  启动的窗口会被 KWin 的焦点窃取防护挡住不激活，`-a` 于是截到了用户**原来**那个窗口（第一
  次就截成了另一个 Konsole）。解法是加载一段 KWin 脚本把目标窗口设为 `workspace.activeWindow`，
  并挂 `workspace.windowAdded` —— 一次性调用会和窗口创建赛跑。
- **sudo 需要密码时，`setsid`/`systemd-run` 都救不了后台安装**：sudo 工具自己没有凭据，
  命令会停在密码提示直到超时。装包这一步得由人跑。
