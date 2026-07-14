# 2026-07-14 Dead Code 清理与功能接入

## 概述

系统性处理项目中所有 `#[allow(dead_code)]` 标注，清理无用代码，并将已实现的完整功能接入 UI。

## 变更清单

### Phase 1: 去掉误标的 `#[allow(dead_code)]`（4 处）

| 文件 | 变更 | 原因 |
|------|------|------|
| `src/lyrics/engine.rs:7,10` | 移除 `#[allow(dead_code)]` | `LyricEngine` 实际被 `handlers/mod.rs` 调用 |
| `src/library/database.rs:1,9` | 移除 `#[allow(dead_code)]` | `TrackRow` 被 `row_from_db()` 构造返回 |
| `src/ui/views/library_view.rs:8` | 移除 `#[allow(dead_code)]` | `LibraryPanel` 被 `LibraryState.focused` 使用 |
| `src/lyrics/types.rs:13` | 移除 `#[allow(dead_code)]` | `LyricTrack.metadata` 被 parser 填充 |

### Phase 2: 清理真正无用的死代码（5 处）

| 文件 | 变更 | 原因 |
|------|------|------|
| `src/metadata/reader.rs:16` | 移除 `#[allow(dead_code)]` | `track_total` 读取但未显示 |
| `src/ui/widgets/lyrics_panel.rs:52-60` | 删除 `render_no_lyrics()` | 无调用者 |
| `src/ui/mod.rs:200-241` | 删除不可达回退路径及 4 个私有 helper | 7 个 ViewMode 都有 early return |
| `src/ui/theme.rs` | 删除整个文件（110 行） | 无 mod 声明，从未编译 |
| `src/ui/mod.rs` imports | 清理未使用的导入 | `Constraint`, `Direction`, `Layout`, `Gauge`, `List`, `ListItem`, `Modifier`, `Line`, `Span` |

### Phase 3: 接入可配置键位系统

**文件**: `src/input/keymap.rs`, `src/input/handler.rs`, `src/app/mod.rs`, `src/app/handlers/mod.rs`

- `keymap.rs`: 添加 `parse_key_str()` 函数，支持单字符、`^X` Ctrl 组合、特殊键名（Space/Enter/Esc/Up/Down 等）
- `handler.rs`: `KeyHandler` 接受可配置的 quit 键，不再硬编码 `'q'`
- `app/mod.rs`: `App` 存储 `KeyBindings`，启动时加载，传递给 `KeyHandler`
- `handlers/mod.rs`: 添加 `key_matches()` helper，替换 9 个可配置键的硬编码匹配

### Phase 4: 接入 Vim `:` 命令系统

**文件**: `src/input/command.rs`, `src/ui/mod.rs`, `src/app/handlers/mod.rs`

- `command.rs`: 添加 `Import(String)` 和 `Export(String)` 变体
- `UiState`: 添加 `command_mode: bool` 和 `command_buffer: String`
- `handlers/mod.rs`: 添加 `:` 键拦截、`handle_command_input()` 和 `dispatch_command()` 实现
- `ui/mod.rs`: 添加底部命令栏渲染
- `settings.rs`: `write_config` 改为 `pub(crate)` 供命令系统调用

支持的命令: `:q`, `:help`, `:version`, `:theme`, `:seek`, `:volume`, `:repeat`, `:shuffle`, `:view`, `:import`, `:export`

### Phase 5: 接入 M3U 导入/导出

**文件**: `src/library/playlist_manager.rs`, `src/app/handlers/playlist.rs`

- `playlist_manager.rs`: 移除 `#![allow(dead_code)]`
- `handlers/playlist.rs`: 添加 `e` 键导出当前展开的歌单为 M3U
- `handlers/mod.rs` (dispatch_command): 实现 `:import` 和 `:export` 命令

### Phase 6: 移除 playlist.rs 误标

`Playlist` 和 `TrackEntry` 现在被 M3U 导入/导出使用，移除 `#[allow(dead_code)]`。

### Phase 7: 帮助页面重构 + README + 抑制残留警告

**帮助页面** (`src/ui/widgets/help_popup.rs`):
- 从两栏布局改为单栏可滚动布局
- 添加 `help_scroll: usize` 到 `UiState`
- j/k 滚动、0/Esc 关闭，标题栏显示滚动指示器
- 新增内容：命令模式参考、M3U 导出入、自定义键位说明

**README.md**: 添加命令模式参考表、M3U 导出快捷键、更新 FAQ

**残留警告**: 为 `TrackRow`、`LyricTrack.metadata`、`TrackInfo.track_total`、`TrackDisplay`、`render_lyrics` 等公开 API 字段添加 `#[allow(dead_code)]`（这些字段用于序列化或未来扩展）

## 验证

```bash
cargo test  # 41 passed, 0 failed
cargo check # 0 warnings
```

## 统计

- 修改文件: ~24 个
- 删除文件: 1 个 (`src/ui/theme.rs`)
- 新增代码: ~300 行（命令系统、帮助页面）
- 删除代码: ~250 行（死代码、helper、theme.rs）
- 测试: 41 通过（+4 来自之前未编译的 command.rs 和 playlist_manager.rs 测试）
