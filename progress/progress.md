# 项目进度记录

> 项目：termusic — 终端音乐播放器
> 开始日期：2026-07-12

---

## 整体进度

| Phase | 名称 | Session 数 | 完成数 | 状态 |
|-------|------|-----------|--------|------|
| P0 | 项目脚手架 | 1 | 1/1 | ✅ 完成 |
| P1 | 核心播放 MVP | 4 | 4/4 | ✅ 完成 |
| P2 | 播放列表 + 元数据 | 6 | 6/6 | ✅ 完成 |
| P3 | 歌词系统 | 4 | 0/4 | ⬜ 待开始 |
| P4 | 频谱可视化 | 4 | 0/4 | ⬜ 待开始 |
| P5 | 曲库 + 高级 UI | 7 | 0/7 | ⬜ 待开始 |
| P6 | 扩展 | 9 | 0/9 | ⬜ 待开始 |

**总进度：11 / 35 Sessions (31%)**

## 详细记录

| Session | 文件 | 状态 |
|---------|------|------|
| [S0.1](s0.1-scaffold.md) | 项目脚手架 | ✅ |
| [S1.1](s1.1-decoder.md) | Symphonia 解码器 | ✅ |
| [S1.2](s1.2-output.md) | Rodio 音频输出 | ✅ |
| [S1.3](s1.3-engine.md) | AudioEngine | ✅ |
| [S1.4](s1.4-tui-mvp.md) | TUI + 事件循环 | ✅ |
| [S2.1](s2.1-metadata-reader.md) | 元数据读取 | ✅ |
| [S2.2](s2.2-playlist.md) | 播放列表结构 | ✅ |
| [S2.3](s2.3-scanner.md) | 目录扫描器 | ✅ |
| [S2.4](s2.4-player-view.md) | Player View UI | ✅ |
| [S2.5](s2.5-vim-keys.md) | Vim 键盘导航 | ✅ |
| [S2.6](s2.6-search-config-modes.md) | 搜索 + 配置 + 模式 | ✅ |

## 当前项目状态

```
termusic/
├── Cargo.toml          ✅ 17 个依赖
├── Cargo.lock          ✅
├── config/default.toml ✅
├── .github/workflows/  ✅ CI
├── progress/           ✅ 进度记录
├── src/
│   ├── main.rs         ✅
│   ├── app.rs          ✅ 事件循环 + Vim 键位 + 搜索
│   ├── event.rs        ✅ JumpTop/JumpBottom/RemoveSelected
│   ├── config.rs       ✅ 5段完整配置
│   ├── cli.rs          ✅ CLI
│   ├── error.rs        ✅
│   ├── playlist.rs     ✅ TrackEntry + Playlist + SortKey
│   ├── audio/          ✅ decoder + output + engine
│   ├── metadata/       ✅ reader.rs (lofty)
│   ├── library/        ✅ scanner.rs (walkdir)
│   ├── lyrics/         ⬜ 空桩
│   ├── visualizer/     ⬜ 空桩
│   ├── ui/             ✅ 完整 Player View + 搜索过滤
│   └── input/          ✅ KeyHandler 双键序列
└── tests/fixtures/
    ├── test.wav        ✅ 440Hz 立体声
    ├── test.flac       ✅ 带标签
    └── test_notags.wav ✅ 无标签
```

## 验证状态

| 检查 | 结果 |
|------|------|
| cargo build | ✅ 零 warning |
| cargo test | ✅ 18/18 通过 |
| cargo clippy | ✅ 零 warning |

## 下一步

**Phase 3: 歌词系统 (4 Sessions)** — 可与 Phase 4 并行
**Phase 4: 频谱可视化 (4 Sessions)** — 可与 Phase 3 并行
