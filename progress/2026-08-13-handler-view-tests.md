# 2026-08-13 会话：补 `app/handlers/*` 与 `ui/views/*` 测试

## 概述

用户要求「补 ui/views/* 和 app/handlers/* 的测试」。此前这两层渲染/分发逻辑行覆盖率接近 0
（CLAUDE.md 记录的 45.09% 基线，`ui/views/*` 与 `app/handlers/*` 是最大缺口）。
本次为 11 个文件新增 115 个测试用例：handlers 87 + views 28，总测试数 59 → **174**。

## 变更清单

### 1. 测试隔离基础设施
- `src/paths.rs`（`#[cfg(test)]`）：`data_dir()`/`config_dir()` 重定向到
  `/tmp/tmper-tests-<pid>`（OnceLock，按进程），使 handler 测试写 `playlists.json`、
  `config.toml`、`*.m3u`、`library.db` 的副作用全部隔离；`project_root()` 不重定向，
  主题与 fixtures 仍解析到真实树。
- `src/app/handlers/mod.rs`：新增 `pub(crate) mod test_support`（`test_app()` =
  `App::new(&Config::default())`、`pl()`、`seed_settings()`）。

### 2. app/handlers（87 个用例）
| 文件 | 用例 | 覆盖点 |
|------|-----|--------|
| browser.rs | 14 | 焦点切换、库/文件系统导航 clamp、Enter 进目录/加库去重、Backspace 边界、刷新过滤排序 |
| library.rs | 17 | 面板导航、搜索输入/回车/回退、`clamp_scroll`、库加载 upsert/去重、Enter 播放（1 个 tokio） |
| playlist.rs | 18 | 焦点切换、插入模式建歌单、展开/删除/重复保护、M3U 导出、flat-model 解析、clamp |
| settings.rs | 16 | 19 行布局、j/k clamp、主题/柱数/平滑/音量/步长/封面循环、跳过行、Enter 动作、config 持久化 |
| mod.rs | 22 | 键位匹配、视图分发切换、滚动 clamp 等 |

### 3. ui/views（28 个用例）
| 文件 | 用例 | 覆盖点 |
|------|-----|--------|
| file_browser_view.rs | 3 | 空/填充渲染、聚焦样式 |
| library_view.rs | 4 | 三面板标题、数据行、搜索栏、光标闪烁 |
| lyrics_view.rs | 4 | 空提示、歌词+高亮、offset 标签、滚动保持当前行 |
| playlist_view.rs | 8 | flat-model 行数/行号/解析、styled lines、渲染/通知弹出 |
| settings_view.rs | 3 | rebuild_settings 布局与值、渲染冒烟 |
| player_view.rs | 6 | cover 块渲染（空/零面积/内存 PNG）、渲染冒烟、搜索命中/无匹配 |

渲染冒烟统一用 ratatui `TestBackend`（80×24）断言缓冲区文本。
**注意**：ratatui 对 CJK 字符渲染为「字符 + 延续空格」两格，断言多字节文本需按单字符
（`out.contains('导')`），不能按整串（`"设置"` 在缓冲区是 `"设 置"`）。

## 覆盖率（`cargo llvm-cov --all-features --workspace`，2026-08-13 实测）

- **总行覆盖率 45.09% → 77.81%**（函数 83.44%、区域 77.37%）。
- `app/handlers/*`：playlist 97%、browser 95%、library 93%、settings 94%、mod 76%。
- `ui/views/*`：library_view 100%、file_browser 99%、lyrics 98%、settings 98%、playlist 89%、player_view 72%。
- 距 80% 目标的小缺口：`app/mod.rs`（67%）、`ui/views/player_view.rs`（72%，封面/歌词渲染分支）。

## 验证

```
cargo fmt --all -- --check       ✅
cargo clippy -- -D warnings      ✅ 零警告
cargo test                       ✅ 174 passed; 0 failed
cargo llvm-cov --workspace --all-features  ✅ 总行覆盖率 77.81%
```

## 文档同步

CLAUDE.md / README.md / DESIGN.md §10.1 更新：测试数 59→174、覆盖率基线 45.09%→77.81%、
测试分布表新增 11 个 handler/views 行。
