# 项目进度记录

> 项目：termusic — 终端音乐播放器
> 开始日期：2026-07-12

---

## 整体进度

| Phase | 名称 | Session 数 | 完成数 | 状态 |
|-------|------|-----------|--------|------|
| P0 | 项目脚手架 | 1 | 1/1 | ✅ 完成 |
| P1 | 核心播放 MVP | 4 | 4/4 | ✅ 完成 |
| P2 | 播放列表 + 元数据 | 6 | 0/6 | ⬜ 待开始 |
| P3 | 歌词系统 | 4 | 0/4 | ⬜ 待开始 |
| P4 | 频谱可视化 | 4 | 0/4 | ⬜ 待开始 |
| P5 | 曲库 + 高级 UI | 7 | 0/7 | ⬜ 待开始 |
| P6 | 扩展 | 9 | 0/9 | ⬜ 待开始 |

**总进度：5 / 35 Sessions (14%)**

---

## Session 0.1 — 初始化项目骨架 ✅

**日期**: 2026-07-12
**Commit**: `896f4ca`

### 新建文件
| 文件 | 说明 |
|------|------|
| `Cargo.toml` | 13个核心依赖 + release 优化 |
| `src/main.rs` | 模块声明 + tracing 初始化 |
| `src/error.rs` | AppError 枚举 + AppResult 类型 |
| `src/**/mod.rs` (9个) | 全部模块声明 |
| `src/**/*.rs` (30个) | 空桩文件 |
| `config/default.toml` | 完整默认配置 |
| `.github/workflows/ci.yml` | CI 流水线 |
| `.gitignore` | 忽略规则 |

### 验收结果
- `cargo build`: ✅ 零 warning
- `cargo test`: ✅ 通过
- `cargo clippy`: ✅ 零 warning
- `cargo run`: ✅ 输出 "termusic starting..."

---

## Session 1.1 — Symphonia 音频解码器 ✅

**日期**: 2026-07-12
**Commit**: `ce8dd4b`

### 新建/修改
| 文件 | 操作 | 说明 |
|------|------|------|
| `src/audio/decoder.rs` | 重写 | AudioDecoder 实现 |
| `tests/fixtures/test.wav` | 新建 | 440Hz 测试音频 |

### 关键实现
- `AudioDecoder::open(path)` — 自动探测容器格式和编码
- `AudioDecoder::read_packet()` — 解码为 f32 PCM 采样
- `AudioDecoder::duration_secs()` — 总时长
- 使用 `symphonia::default::get_probe()` 探测格式
- `SampleBuffer<f32>` 统一转换为 f32

### 验收结果
- `cargo test`: ✅ 2/2 通过 (decode_wav + nonexistent_file)
- `cargo clippy`: ✅ 零 warning

---

## Session 1.2 — Rodio 音频输出 ✅

**日期**: 2026-07-12
**Commit**: `ce8dd4b`

### 修改
| 文件 | 操作 | 说明 |
|------|------|------|
| `src/audio/output.rs` | 重写 | AudioOutput 实现 |

### 关键实现
- `AudioOutput::new()` — 创建 OutputStream + Sink
- `play_raw()` / `pause()` / `play()` / `stop()` / `set_volume()`
- `_stream` 字段保持 OutputStream 存活

### 验收结果
- 编译通过，无单独测试（在 S1.3 集成测试）
- `cargo clippy`: ✅ 零 warning

---

## Session 1.3 — AudioEngine ✅

**日期**: 2026-07-12
**Commit**: `ce8dd4b`

### 修改
| 文件 | 操作 | 说明 |
|------|------|------|
| `src/audio/engine.rs` | 重写 | AudioEngine 实现 |

### 关键实现
- `play_file(path)` — 全量解码 → 送入 Sink
- `Arc<Mutex<u64>>` 线程安全位置追踪
- `pause()` / `resume()` / `stop()` / `set_volume()`
- `position_secs()` / `duration_secs()` / `is_playing()`

### 验收结果
- `cargo test`: ✅ 3/3 通过 (lifecycle + position + stop_clears)
- `cargo clippy`: ✅ 零 warning

---

## Session 1.4 — TUI 骨架 + 事件循环 + MVP ✅

**日期**: 2026-07-12
**Commit**: `ce8dd4b`

### 修改
| 文件 | 操作 | 说明 |
|------|------|------|
| `src/config.rs` | 重写 | Config 加载（XDG + toml） |
| `src/cli.rs` | 重写 | clap CLI (play 子命令) |
| `src/event.rs` | 重写 | AppEvent 枚举 |
| `src/ui/mod.rs` | 重写 | ratatui 渲染（标题栏+进度条） |
| `src/input/handler.rs` | 重写 | 键盘处理 |
| `src/app.rs` | 重写 | 主事件循环 |
| `src/main.rs` | 重写 | 入口：tokio + tracing + 启动 |
| `Cargo.toml` | 修改 | 添加 futures-util，启用 crossterm event-stream |

### 关键实现
- crossterm raw mode + alternate screen
- tokio::select! 事件循环（键盘 + 100ms tick）
- Space 暂停/恢复，q 退出，- = 调节音量
- 实时进度条 + 播放位置显示
- `cargo run -- play <file>` 播放音频

### 验收结果
- `cargo build`: ✅ 零 warning
- `cargo test`: ✅ 5/5 通过
- `cargo clippy`: ✅ 零 warning
- `cargo run`: ✅ 启动正常（需真实 TTY）

---

## 当前项目状态

```
termusic/
├── Cargo.toml          ✅ 14 个依赖
├── Cargo.lock          ✅ 锁定
├── config/default.toml ✅ 默认配置
├── .github/workflows/ci.yml ✅ CI
├── src/
│   ├── main.rs         ✅ 入口
│   ├── app.rs          ✅ 事件循环
│   ├── event.rs        ✅ 事件类型
│   ├── config.rs       ✅ 配置加载
│   ├── cli.rs          ✅ CLI 参数
│   ├── error.rs        ✅ 错误类型
│   ├── playlist.rs     ⬜ 空桩
│   ├── audio/
│   │   ├── mod.rs      ✅
│   │   ├── decoder.rs  ✅ Symphonia 解码
│   │   ├── output.rs   ✅ Rodio 输出
│   │   └── engine.rs   ✅ 播放引擎
│   ├── metadata/       ⬜ 全部空桩
│   ├── lyrics/         ⬜ 全部空桩
│   ├── visualizer/     ⬜ 全部空桩
│   ├── library/        ⬜ 全部空桩
│   ├── ui/             ⬜ 仅 mod.rs 有内容
│   └── input/          ⬜ 仅 handler.rs 有内容
└── tests/
    └── fixtures/
        └── test.wav    ✅ 测试音频
```

---

## 下一步

**Phase 2: 播放列表 + 元数据（6 Sessions）**
- S2.1: 元数据读取（lofty 集成）
- S2.2: 播放列表数据结构
- S2.3: 目录扫描器
- S2.4: 完整 Player View UI
- S2.5: Vim 风格键盘导航
- S2.6: 搜索过滤 + Config 完整加载 + 播放模式
