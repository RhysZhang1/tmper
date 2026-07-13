# 封面图显示开关 — 2026-07-13

## 需求

设置界面添加"是否加载图像"选项。选否时，键 1 播放器视图左上角不再显示封面图，改为显示歌曲详细信息（专辑、流派、年份、编码格式等）。

## 变更

### 涉及的 6 个文件

1. **`src/ui/mod.rs`** — `UiState` 新增 `show_cover_art: bool` 字段，默认 `true`
2. **`src/app/mod.rs`** — `App::new()` 中从 `config.ui.show_cover_art` 初始化
3. **`src/ui/views/settings_view.rs`** — 第 8 行从"键位文件"改为"显示封面图"开关
4. **`src/app/handlers/settings.rs`** — `cycle_setting()` 新增 index 8 切换逻辑
5. **`src/ui/views/player_view.rs`** — `render_cover_art()` 新增 `state.show_cover_art` 判断
6. **`src/app/handlers/mod.rs`** — `handle_tick()` 末尾同步 config → ui_state

### 效果

| 封面图开（默认） | 封面图关 |
|:---:|:---:|
| chafa 风格彩色块字符封面 | 居中文字信息 |
| 标题 + 艺术家 + 时间 | 标题 + 艺术家 + 时间 |
| | 专辑行 |
| | 流派 + 年份 + 编码行 |

## 验证

- `cargo build` — clean, 0 warnings
- `cargo test` — 37/37 passed
- `cargo clippy -- -D warnings` — clean
