# 2026-07-19 Session: dead_code 清理 + cover-escape guard 重写

## 改动内容

### 1. dead_code 清理 (commit a4db26d)
- **移入 `#[cfg(test)]`**: `Playlist::len/is_empty/remove/next`, `AudioEngine::play_file()`, `scanner::scan_directory()`, `Database::query_all/delete_by_path/count`
- **删除废弃文件**: `src/ui/widgets/lyrics_panel.rs`（player_view 有内联渲染）
- **删除未用方法**: `Database::get_paths_in_dir()`, `Playlist::titles()`
- **细化标注**: `TrackDisplay`、`LyricTrack.metadata`、`TrackInfo.track_total` 从结构体级 allow 改为精确字段级注解

### 2. cover-escape guard 重写 (commit a4db26d + 4181fb1)
- **旧方案**: 切歌后固定 800ms 盲拦所有 Char 事件
- **新方案**: `CoverRenderer.last_output_at` 记录实际输出时刻 → 仅在输出后 200ms 内拦截 Char 事件
- **关键修复**: `last_output_at` 不在 chafa 每帧重发路径上设置，避免 guard 永久不解锁
- 无封面/封面禁用时 guard 不启动，零误伤

### 3. 文档
- `DESIGN.md` §9.3: 新增 cover-escape guard 文档
- `DESIGN.md` 文件树: 移除 `lyrics_panel.rs` 引用

## 测试结果
- `cargo build`: 零警告
- `cargo test`: 54/54 通过
- `cargo clippy`: 未运行

## 提交
- `4181fb1` fix: cover-escape guard permanently blocked all Char input on Player view
- `a4db26d` refactor: eliminate dead_code warnings and improve cover-escape guard
（均在 master 分支，未推送）
