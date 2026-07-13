# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Terminal music player (codename: **tmper**) — a terminal-native music player for Arch Linux/KDE Plasma. Written in Rust with ratatui TUI framework. Supports multi-format audio decoding, metadata display, LRC lyrics syncing, spectrum visualizer, and Vim-style keyboard navigation.

The project is currently at the **design phase** — only `DESIGN.md` exists, with no code written yet.

## Development Approach

This project uses a **session-based AI development workflow** defined in `DESIGN.md` §13. Each session is a self-contained task:

- Each session includes all context (input files, output files, function signatures, test strategy)
- Sessions compile and test independently (checkpoint-style)
- Independent sessions can be parallelized (marked with ✅ in DESIGN.md)
- Context size is labeled: 🟢 <200 lines, 🟡 200-500 lines, 🔴 >500 lines

### Phase Order

```
Phase 0 (1 session):  Project scaffold (Cargo.toml, mod.rs stubs, error types, CI)
Phase 1 (4 sessions): Core playback MVP (decoder → output → engine → event loop + basic UI)
Phase 2 (6 sessions): Playlist + metadata (lofty reading, playlist data, scanner, player view, vim keys, search)
Phase 3 (4 sessions): Lyrics system (LRC parser, sync engine, offset tuning, fullscreen view) — parallel with P4
Phase 4 (4 sessions): Spectrum visualizer (InstrumentedSource, FFT, rendering, integration) — parallel with P3
Phase 5 (7 sessions): Library index + advanced UI (SQLite, incremental scan, library browser, M3U, themes, keybindings)
Phase 6 (9 sessions): Extensions (cover art, notifications, MPRIS2, online lyrics, EQ, Last.fm, packaging, macOS)
```

## Key Architecture Decisions

### Language: Rust
- ratatui (most mature Rust TUI framework) + crossterm terminal backend
- rodio + symphonia for audio (pure Rust, no system ffmpeg dependency, single binary distribution)
- Zero-cost abstraction for real-time audio decoding and FFT
- Single binary: `cargo build --release`

### Concurrency Model
- **tokio** async runtime with `tokio::mpsc` channels for inter-module communication
- Audio decoding and FFT run on dedicated threads (`tokio::task::spawn_blocking`)
- UI rendering on main thread at ~30-60 FPS
- State updates centralized in event loop; UI rendering is read-only (`&AppState`)

### Audio Pipeline
```
Audio file → Symphonia (format probe + decoder) → PCM f32 samples
  ├─ Rodio Sink → sound card
  └─ Ring buffer → FFT Analyzer → SpectrumProcessor → VisualizerData event
```

## Project Structure (target)

```
src/
├── main.rs             # Entry: tracing init, config load, App::run()
├── app.rs              # AppState + event loop
├── event.rs            # AppEvent enum
├── config.rs           # Config loading from XDG paths
├── cli.rs              # clap CLI args
├── playlist.rs         # Playlist + TrackEntry data structures
├── audio/              # AudioEngine, Symphonia decoder, Rodio output
├── metadata/           # Lofty tag reader
├── lyrics/             # LRC parser, sync engine
├── visualizer/         # FFT analysis, spectrum processing, rendering
├── library/            # SQLite index, directory scanner, M3U I/O
├── ui/                 # Full UI: views (player, library, lyrics, visualizer) + widgets
└── input/              # Keymap, handler (modal), command parser
```

## Key Design Patterns

- **Single writer, read-only readers**: AppState mutated only in event loop's `handle_event()`, UI reads `&AppState`
- **Channel-based communication**: Subsystems send results via `tokio::mpsc::unbounded_channel`
- **Immutable updates**: `update(old, changes) → new` pattern, no mutation of fields
- **Error handling**: `anyhow::Result<T>` for public APIs, `thiserror` for `AppError` enum in `src/error.rs`
- **Config priority**: CLI args > env vars > user config file (~/.config/tmper/config.toml) > hardcoded defaults

## Dependencies (core)

tokio, ratatui, crossterm, rodio, symphonia, lofty, clap, toml, serde, tracing, tracing-subscriber, anyhow, thiserror, dirs

## Commands (when code exists)

```bash
# Build
cargo build
cargo build --release

# Run
cargo run -- play <path/to/audio>

# Test
cargo test                        # all tests
cargo test <test_name>            # single test

# Lint & Format
cargo clippy -- -D warnings
cargo fmt --all

# Full pre-commit check
cargo fmt --all && cargo clippy -- -D warnings && cargo test
```

## Testing Conventions

- Test audio files go in `tests/fixtures/` (generate with ffmpeg: `ffmpeg -f lavfi -i "sine=frequency=440:duration=2" -ar 44100 -ac 2 tests/fixtures/test.wav`)
- Logical modules have `#[cfg(test)] mod tests { ... }` inline
- Tests follow Arrange-Act-Assert pattern
- Cover normal paths + boundary conditions (empty lists, corrupt files, missing metadata)
