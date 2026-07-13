# 2026-07-13 — 全面重构与 Bug 修复

> 问题修复、架构重构、UI 重写、新功能

---

## 今日完成

### 1. 🐛 核心 Bug 修复

| 问题 | 原因 | 修复 |
|------|------|------|
| **进度条一开始就跳到 100%** | `position_secs()` 基于 `total_frames/sample_rate`，但所有帧在 `play_file()` 中同步解码完毕 | 改用 **Instant 计时** 跟踪播放位置，暂停时间累加扣除 |
| **FFT 频谱不跳动** | `InstrumentedSource` 包裹解码器后立即 `.collect()`，PCM buffer 在加载阶段填满 | **后移** InstrumentedSource 到 SamplesBuffer 之外，采样在 rodio 播放线程中实时写入环形缓冲区 |
| **回车播放没有声音** | `sink.stop()` 在 rodio 0.20 中永久断开音频线程，新声源无法播放 | 新增 **`stop_and_replace()`**：丢弃旧 Sink 创建全新 Sink，新声源立即播放 |
| **歌词中文显示问号** | `decode_with_fallback` 对所有 fallback 编码直接返回，忽视错误 | 跳过有解码错误的 fallback，仅返回无错误结果 |

### 2. 📦 架构重构

**app.rs 拆分 (1149 → 3 文件)**

| 文件 | 行数 | 职责 |
|------|------|------|
| `src/app/mod.rs` | 131 | `App` 结构体 + `new()` + 主循环 |
| `src/app/handlers.rs` | 843 | 事件分发 + 全部按键处理 + FFT + 歌词同步 |
| `src/app/persistence.rs` | 193 | 状态/歌单/曲库持久化 |

### 3. 🎨 UI 重写 (Player View)

两栏布局：

```
LEFT (33%)                    RIGHT (67%)
┌──────────────────┬────────────────────────────────┐
│ Now Playing      │ Lyrics                          │
│ 封面图/♫ 占位    │ (居中歌词 + 当前行高亮)           │
│                  │                                 │
│ Playlists        │ Spectrum (cava风格)             │
│ (同步 View 5)    │ 32柱 60Hz-8kHz 缓慢平滑          │
│ ▶ MyList ▼     │                                 │
│   Song A ◄     │ ▶ 03:12/05:55 [████] Vol:80% 🔁 │
└──────────────────┴────────────────────────────────┘
```

### 4. 🎯 新功能

| 功能 | 详情 |
|------|------|
| **← → 快进快退** | 真实音频 seek，重新解码文件跳到目标位置 |
| **歌单侧栏交互** | j/k 导航侧栏歌单，Enter 展开/播放 |
| **封面图展示** | `image` crate 解码内嵌封面，半块字符 (▄) + fg/bg 实现 2x 垂直分辨率，Lanczos3 缩放 + Floyd-Steinberg 抖动 |
| **cava 风格频谱** | 32 柱，统一缓慢平滑 (α=0.22)，30FPS 刷新 |

### 5. 🧹 其他改进

- 中文宽度计算：`playlist_view.rs` 通知弹窗改用 `unicode-width`
- `output.rs`：`append_source()` 改为泛型 `Source` 参数
- 频谱频率范围：20Hz–16kHz → **60Hz–8kHz**（去掉低频噪声区）

---

## 当前状态

- **测试**: 41 个测试全部通过
- **代码量**: ~5165 行 Rust
- **clippy**: 零 warning

## 待办/下一步

### P6 扩展功能 (待实现)

| 功能 | 优先级 | 说明 |
|------|--------|------|
| **Command Mode** | 高 | `:` 命令模式已解析但未接入 UI |
| **Library View** | 中 | View 2 目前是 placeholder |
| **M3U 导入导出** | 中 | 代码已实现但未接入 UI |
| **Seek 改进** | 低 | 当前为重新解码，可改为 symphonia 原生 seek |
| **封面图增强** | 低 | 接入 viuer Kitty 协议真正像素渲染 |
| **MPRIS2 集成** | 低 | KDE 媒体键控制 |
| **桌面通知** | 低 | 切歌时 D-Bus 通知 |
| **在线歌词** | 低 | 网易云/QQ 音乐 API 搜索 |

### 已知问题

- 频谱柱上升也缓慢（用户指定不要快速响应）
- seek 需要重新解码整个文件到目标位置（大数据量文件耗时较长）
- 部分 MP4 格式的 `lofty` 标签读取可能缺字段

---

**总测试**: 41 passed, 0 failed
**clippy**: ✅ 零 warning
