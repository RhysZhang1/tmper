# 2026-10-03 会话：整合 codex/xdg-library-index（XDG + 流式播放 + 曲库扫描）

## 背景

本次会话从一次全项目评审开始。评审列出了若干真实缺陷（见文末"遗留"），其中两项是架构级的：
整文件解码常驻内存、曲库没有生产扫描器。随后发现远程存在一条独立演进的分支
`origin/codex/xdg-library-index`（基于 `00c7ec9`，4 个提交），已经把这两项连同 XDG 路径
一起做掉了。经逐提交评审后决定：**全部合并，手工解冲突**，并在合并结果上继续修剩余缺陷。

## 为什么不能直接合并

`git merge-tree` 预演显示 16 个冲突块，且四处"合了会更糟"：

1. **测试会污染开发者真实 XDG 目录** —— 分支把 `data_dir()` 改成真实 XDG 路径，并新增
   承接 `state.json`/`playlists.json` 的 `state_dir()`；main 的 115 个 handler 测试走
   `App::new()`，合并后每次 `cargo test` 都会写 `~/.config/tmper`、`~/.local/share/tmper`、
   `~/.local/state/tmper`。
2. **device-free CI 会挂** —— 分支的 CI job 不再提供 null ALSA，而 main 的
   `test_support::test_app()` 用真 `App::new()`（会打开 `OutputStream`）。
3. **锁健壮性倒退** —— 分支引擎重写新增 6 处 `.lock().unwrap()`，等于丢掉 main 的
   `7ec8930`（poison 恢复）。
4. **新扫描器有两个会误删曲库记录的 bug**（见下）。

## 冲突解决（16 块）

| 文件 | 块 | 解决方式 |
|------|----|----------|
| `audio/engine.rs` | 11 | 取分支的 session/generation 重写，重套 main 的 `lock()` poison 恢复（6 处裸 unwrap + `InstrumentedSource::next` 的静默丢弃改为恢复） |
| `paths.rs` | 1 | 取 XDG 版，把 main 的 `test_root()` 隔离扩到 config/data/**state** 三个目录 |
| `help_popup.rs` | 2 | 取分支的 XDG 文案（其测试断言该字符串）与 footer；修正分支"stop 当前未接入"——该字段在 `55694b3` 已整个删除，改为"共 8 个" |
| `DESIGN.md` | 1 | 两边互补：保留 main 的测试分布表 + 分支的测试分层表，合并为 §10.1 的两个子节 |
| `README.md` | 1 | 取分支的测试命令（默认无设备 + `audio_output_ -- --ignored`） |
| `CLAUDE.md` | modify/delete | 按用户决定**保留**，并整体更新为 XDG/扫描器/流式的新事实（否则主动误导） |

## 合并后的四项善后

- **(a) 测试隔离扩到 `state_dir`** —— `test_root()` 现覆盖 config/data/state 三个目录；
  新增 `runtime_dirs_are_isolated_under_test` 锁定该不变量。
- **(b) 测试改用 headless 引擎** —— `test_support::test_app()` → `App::new_headless()`；
  默认测试集现在**不需要声卡**（实测 0.93s 跑完）。
- **(c) poison 恢复锁** —— 见上表。
- **(d) 扫描器两个误删 bug**：
  - `scan_incremental` 用 `filter_map(Result::ok)` 静默吞掉 walkdir 错误，却把结果当作
    "完整文件列表"交给 `delete_missing_under` —— 任何权限/I/O 错误的子树都会被判为
    "文件已消失"而**从曲库删除**。现在统计 walk 错误并置 `complete: false`，调用方
    只在 `complete` 时剪枝（新增 `test_incomplete_scan_does_not_prune_library` 与
    配对测试 `test_complete_scan_prunes_missing_tracks`，防止退化成"永不剪枝"）。
  - `file_fingerprints_under` 用 `path LIKE '<root>/%'`：`%`/`_` 会被当通配符，且 SQLite
    的 LIKE 对 ASCII 大小写不敏感，`/music` 会连带匹配 `/music_extra` 与 `/MUSIC`。
    改为 `substr(path, 1, length(?1)) = ?1`（精确、大小写敏感；长度取自 SQLite 的
    `length()` 以避免 UTF-8 字节/字符混淆）。新增两个回归测试。
- **release 迁移修复** —— `legacy_project_root()` 原用 `env!("CARGO_MANIFEST_DIR")`，是编译期
  常量，安装版会指向构建机路径导致迁移静默失效。release 下改为从可执行文件向上查找
  `themes/`+`config/`；并补上 `themes/*.toml` 的迁移（原先漏掉，用户自定义主题会静默丢失）。

## 验证

```
cargo fmt --all -- --check        ✅
cargo clippy --all-features -- -D warnings   ✅ 零警告
cargo test                        ✅ 181 passed; 0 failed; 6 ignored（0.93s，无需声卡）
cargo llvm-cov --all-features --workspace    ✅ 75.55% 行（79.13% 函数、76.48% 区域）
```

覆盖率从合并前 77.81% 回落到 75.55%：合并带进约 880 行新生产代码（引擎重写 590 行、
扫描器 118 行、FTS5），新增测试只有 7 个。注意 `audio/engine.rs` 从 94% 掉到 49%。

## 已知遗留（本次未处理，按优先级在后续会话修）

- `handlers/browser.rs` 用合并列表 `fs_items` 的下标去索引各自压缩的 `dirs`/`audio_files`：
  目录里同时有子目录和音频文件时会打开错误条目或越界 panic。
- `input/handler.rs` 双键状态机：非组合键时丢弃当前事件（丢按键）；单个 `g`/`d` 永久挂起；
  退出键判断早于模式分发 → 搜索框里打 `q` 直接退出程序。该文件 19% 覆盖、0 测试。
- `ui/theme.rs` `parse_hex` 用字节长度校验后按字节切片 → 多字节输入 panic（启动即崩）。
- `app/playback.rs` `prev_track` 缺边界保护（`next_track`/`on_track_ended` 都有）。
- 频谱取环形缓冲最旧样本（滞后 ~0.7s）、配色与柱错位、曲库三栏不滚动、`frame_rate` >20 无效。
