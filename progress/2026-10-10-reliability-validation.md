# 2026-10-10 可靠性改进与完整本地验证

本轮基于 `codex/visualizer-live-settings`，重点解决真实音频、多个客户端、后台运行和异常退出的可靠性问题。未修改用户现有音乐库、配置或播放状态。

## 资料核对

- [Symphonia 0.5.5 官方格式说明](https://github.com/pdeljanov/Symphonia/tree/v0.5.5)：以当前锁定版本为准校正格式声明和扫描扩展名，避免将上游开发中的解码器算作已支持功能。Opus、WMA、APE、WavPack 和 HE-AAC 不在本项目当前支持范围。
- [Tokio channels](https://tokio.rs/tokio/tutorial/channels) 与 [spawn_blocking](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)：扫描线程使用有界通道施加背压；退出时主动关闭接收端，唤醒阻塞的发送者。
- Linux [rename(2)](https://man7.org/linux/man-pages/man2/rename.2.html) 与 [fsync(2)](https://www.man7.org/linux/man-pages/man2/fsync.2.html)：小型 JSON 状态采用同目录临时文件、文件同步、原子替换与父目录同步。

## 完成的改进

1. 音频：修复 Vorbis 零帧解码包触发的样本复制崩溃；修复 ADTS AAC 标签读取失败导致无法入库；精确 seek 丢弃目标前的帧，回退 seek 保留包内剩余样本；未知时长不再当作零时长。
2. IPC：查询、操作结果和错误通知只发给请求者，共享状态继续广播；CLI 使用执行屏障确认请求已处理。协议升级至 2，大集合分块传输，单行限制 1 MiB、逻辑消息限制 64 MiB；畸形、截断和过多分块有明确拒绝路径。
3. 后台运行：状态变更限频保存，活跃播放每 30 秒检查点；原子保存失败后重试；恢复有效的活动歌单。SIGTERM、Ctrl+C、quit 正常保存并清理连接；客户端断连停止闲置 FFT，重连恢复频谱订阅及设置。
4. 曲库：扫描通道容量 128，每 tick 最多处理 32 项，批量数据库写入；取消扫描能够唤醒被背压阻塞的线程；入库队列不携带不需要的封面大字节数组。
5. 歌词：解析并显示逐词时间标签，保留词间空格；重复行时间戳同步移动词时间；文件 offset 与用户 offset 共同用于定位和逐词样式。
6. 工程：提供音频生成脚本、完整验证脚本、真实 daemon/CLI/TUI 集成脚本；CI 新增后台进程与崩溃恢复测试，并覆盖 `codex/**` 分支；同步中英文 README、设计文档、状态和变更记录。

## 测试音频

用 FFmpeg 本地合成三秒、44.1 kHz、双声道、440 Hz 音调，编码为 WAV、FLAC、MP3、Vorbis/Ogg、AAC-LC/M4A、ALAC/M4A、AIFF、AAC-LC/ADTS 八个样本。没有下载第三方歌曲；样本按项目 MIT 许可证分发，来源、生成方式和 SHA256 见 `tests/fixtures/formats/`。

回归测试对每个样本执行元数据读取、完整解码到 EOF、有限且非静音 PCM 检查、解码时长检查以及 1.234 秒非包边界 seek 后的剩余时长检查。这些是格式回归样本，不代表所有编码参数、损坏媒体或多轨文件均已覆盖。

## 执行结果

| 验证 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --locked --all-targets --workspace --all-features -- -D warnings` | 通过，无警告 |
| 全量测试，包含默认忽略项，串行、本机默认音频设备 | 551 通过，0 失败，0 忽略 |
| `bash scripts/validate.sh`，独立虚拟 ALSA 设备 | 全流程通过；551 通过，0 失败，0 忽略 |
| 六项音频输出测试，本机默认设备与虚拟设备分别运行 | 两组均 6 通过，0 失败 |
| `cargo build --offline --locked --release` | 通过 |
| 发布二进制端到端测试，独立目录、虚拟 ALSA、私有 D-Bus 会话 | 以下七组全部通过 |
| 八个音频样本 SHA256 校验 | 全部通过 |

发布版端到端测试实际启动进程、Unix socket、SQLite 和伪终端，检查：

1. 两个客户端共享状态，但不会收到对方的操作回复。
2. 使用 `busctl` 读写真实 MPRIS 音量属性，与 CLI 状态一致。
3. CLI 执行确认、暂停状态 seek、歌词偏移以及运行时原子检查点。
4. 八种格式均进入真实 SQLite 曲库，扫描失败数为零。
5. 七个 TUI 视图与帮助画面可渲染；退出 TUI 后 daemon 继续存在。
6. SIGKILL 后重启恢复检查点，并处理遗留 socket。
7. CLI quit 和 SIGTERM 保存状态、清理 socket。

复现入口：`bash scripts/validate.sh`。需要 Rust 工具链、ALSA 开发库、Python 3、D-Bus；MPRIS 属性检查还需要 `busctl`，缺少时脚本会明确报告跳过。FFmpeg 仅在重新生成音频样本时需要。本次 MPRIS 检查实际执行并通过，没有跳过。

## 验证边界与升级说明

- 本次完整测试指仓库全部 551 项测试及上述集成场景，不能等同于所有终端、所有媒体编码和所有崩溃时刻的穷举测试。
- 本机默认音频设备测试确认输出接口和播放生命周期正常，未把自动化设备测试当作人工听音评价。
- Kitty/SIXEL 真终端封面画面与 Plasma 控件视觉效果未在本轮重新验收；TUI 使用伪终端验收。
- 未重新测量代码覆盖率。文档中 89.44% 行覆盖率为 2026-10-04 历史数据，不用于描述新增代码。
- 协议 1 与 2 不兼容：替换二进制前先用旧版 `tmper quit` 停止旧 daemon，再启动新版。
- 检查点允许异常退出丢失最近尚未保存的变更：状态修改通常在一秒检查点内保存，连续播放进度最多间隔 30 秒；磁盘故障时无法保证保存成功。rename 前失败保留旧文件；父目录同步失败时新文件可能已发布，仍报告失败并继续重试。
