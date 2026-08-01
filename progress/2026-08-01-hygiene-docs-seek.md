# 2026-08-01 会话：git 卫生 + 文档同步 + seek 异步化

## 概述

封面渲染问题按用户指示暂缓（architectural debt，另立专项）。本次按序处理其余问题。

## 变更清单

### 1. Git 卫生 (commit f0133c2)
- 分支 `master` → `main`（对齐 CI 触发分支）
- `git rm tmper.tar.gz`（133K 构建快照误入库）
- `.gitignore` 补 `/data/`（日志/SQLite/歌单 JSON）和 `*.tar.gz`
- 远程仓库未配置——用户选择暂缓

### 2. 文档漂移 (commit 7938f52)
- README 帮助键 `0` → `8`
- README/DESIGN 测试数 41 → 54（42 单元 + 12 集成）
- DESIGN §12 移除已删除的 `futures-util` 依赖
- DESIGN §6.1 AppEvent 对齐实际代码：移除 `VisualizerData` / `JumpBottom` 变体
- DESIGN §2.2/§4.1 频谱数据流改为共享 `fft_data`（Arc\<Mutex\>）

### 3. seek_relative 异步化 (commit 815b3b4)
- **根因**：seek 时同步重新解码整个文件，大文件跳转冻结 UI 数秒
- **修复**：镜像 `play_file_async` 模式——`spawn_blocking` 后台解码到共享 `Arc<Sink>`，位置立即跳到目标，错误走 tracing 日志
- 移除已成死代码的 `AudioEngine.sample_rate` / `channels` 字段（只写不读）
- `AudioOutput::append_source` 标记 `#[cfg(test)]`（生产路径用 `sink_arc().append()`）
- 新增 `test_async_seek_jumps_position`
- handlers 中 150ms seek 冷却注释更新（保留冷却防重复重解码）

### 4. magic number 收敛 (commit b20b705)
- `handlers/mod.rs` 硬编码 `visible_h = 10u16` → `runtime::VISIBLE_ROWS = 10`

### 5. Player 视图 `/` 搜索过滤（实现死桩）
- **根因**：全局搜索是死桩——`/` 累积进 `search_query` 但无人读取，且泄漏进其他视图
- **实现**：`search_mode` 迁入 `UiState`；`/`（仅 Player 视图）实时过滤播放器队列（title/artist/path 不区分大小写子串），左上面板显示 `Search: <query>` 结果列表；j/k 导航、Enter 播放退出、Esc/空退格退出；切视图清空
- `search_matches()` 辅助函数 + `test_search_enter_filter_navigate_exit`（56 total）

## 验证

```
cargo build                       ✅ 零 warning
cargo clippy -- -D warnings       ✅ 零 warning
cargo test                        ✅ 55 passed（54 + 1 新增）
```

## 已知遗留

- 封面渲染架构债（stdout 直接写终端协议）——另行专项
- 两个 playlist 模型（`crate::playlist::Playlist` vs `playlist_view::PlaylistData`）可合并
- FFT 数据每 tick `data.clone()` 全量拷贝（~30fps，32 浮点，量级可忽略）
- seek 时若处于暂停状态会恢复播放时钟（既有行为，未在本次改动）
- 无 git remote（备份风险，等用户定托管位置）
