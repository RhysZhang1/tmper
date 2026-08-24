# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Terminal music player (codename: **tmper**) — a terminal-native music player for Arch Linux/KDE Plasma. Written in Rust with ratatui TUI framework. Supports multi-format audio decoding, metadata display, cover art, LRC lyrics syncing, spectrum visualizer, playlist management, a SQLite library index, and Vim-style keyboard navigation.

The project is **implemented and working** (~8,200 lines of Rust, 59 tests). The source of truth for the architecture is `DESIGN.md`; per-session change logs live in `progress/`. All docs (CLAUDE.md / README.md / DESIGN.md) were reconciled with the code on 2026-08-03.

## Layout

- **Paths follow XDG**: config in `$XDG_CONFIG_HOME/tmper`, SQLite in `$XDG_DATA_HOME/tmper`, and state/logs in `$XDG_STATE_HOME/tmper`. `TMPER_CONFIG_DIR`, `TMPER_DATA_DIR`, and `TMPER_STATE_DIR` override them for tests.
- **Defaults are embedded**: `config/default.toml` and all five `themes/*.toml` palettes are compiled into the binary. User themes in `$XDG_CONFIG_HOME/tmper/themes/` override built-ins.
- **Legacy migration is copy-only**: project-local `config/` and `data/` runtime files are copied once when the XDG destination is absent; old files are never deleted.

## Key Architecture Decisions

### Language: Rust
- ratatui (most mature Rust TUI framework) + crossterm terminal backend
- rodio + symphonia for audio (pure Rust, no system ffmpeg dependency, single binary distribution)
- Single binary: `cargo build --release`

### Concurrency Model
- **tokio** async runtime in `App::run()`; main loop uses `tokio::select!` over an event channel and a tick interval
- **Burst-mode input thread**: a background thread `poll()`s crossterm and batch-`read()`s events into an `mpsc::unbounded_channel` of `Vec<CrosstermEvent>`; the loop processes the batch and draws ONCE (handles key auto-repeat without scroll-after-release)
- Audio decode and seek run on background threads (`tokio::task::spawn_blocking`) feeding a shared `Arc<Sink>` — track switches and seeks don't freeze the UI
- FFT analysis runs on its own thread writing into a shared `Arc<Mutex<VecDeque<f32>>>` ring buffer; the UI reads a `Vec<f32>` snapshot each tick
- UI rendering is read-only over `&UiState`; all mutation happens in `App::handle_event`

### Audio Pipeline
```
Audio file → Symphonia (format probe + decoder) → PCM f32 samples
  ├─ Rodio Sink (shared Arc, fed from background thread) → sound card
  └─ Ring buffer → FFT thread → SpectrumProcessor → UiState.visualizer_data
```

## Project Structure

```
src/
├── main.rs             # tracing init, ensure config, App::run()
├── app/                # App + event loop + playback + persistence + handlers/
│   ├── mod.rs          #   App struct, run() event loop, burst-mode input
│   ├── playback.rs     #   move_selection, play_selected, next/prev, on_track_ended, start_fft
│   ├── persistence.rs  #   save/load state, playlists, library paths
│   └── handlers/       #   key dispatch (mod/playlist/library/browser/settings)
├── audio/              # engine.rs (AudioEngine), decoder.rs (Symphonia), output.rs (rodio Sink)
├── metadata/           # lofty tag reader (title/artist/album/cover art)
├── lyrics/             # LRC parser + sync engine + types
├── visualizer/         # fft.rs, processor.rs, render.rs (block-bar rendering)
├── library/            # database.rs (SQLite), scanner.rs (test-only), playlist_manager.rs (M3U)
├── ui/                 # theme.rs, mod.rs (UiState + render), cover/, views/ (7), widgets/
├── input/              # keymap.rs (bindings), handler.rs (double-key gg/dd), command.rs (:cmd)
├── config.rs           # Config structs + load/ensure + clamps (only ~7 live keys)
├── event.rs            # AppEvent enum (Key, Tick, Quit, JumpTop, RemoveSelected)
├── cli.rs              # clap: `tmper play <file>`
├── playlist.rs         # PlaylistData { name, songs: Vec<PathBuf> } — single playlist model
├── paths.rs            # project_root (CARGO_MANIFEST_DIR in debug, exe-relative in release)
├── constants.rs        # runtime tuning constants (timeouts, buffer sizes, FPS)
└── error.rs            # AppError (thiserror)
```

## Key Design Patterns

- **Single writer, read-only readers**: `App` (event loop) mutates `UiState`; `ui::render` only reads it. Render functions take explicit read-only `Params` structs (e.g. `PlayerViewParams`) rather than the whole state.
- **Background decode/seek**: heavy work is `spawn_blocking`, results land via shared `Arc` handles; the main thread stays responsive.
- **Immutable-ish updates**: state structs use `Default` + `..Default::default()`; version counters use `Cell` (`cover_gen`, `visible_rows`).
- **Error handling**: `AppResult<T>` / `AppError` (thiserror) for public APIs; background-thread errors go to `tracing` logs (never panics).
- **Config priority**: CLI args > XDG `config.toml` > embedded defaults. Unknown keys are ignored (no `deny_unknown_fields`).
- **Config reality check**: only `default_volume`, `seek_step_small_secs`, `num_bars`, `frame_rate`, `smoothing`, `theme`, `show_cover_art` are live. Do not re-add speculative keys without a consuming implementation.

## Known Architectural Debt (do NOT re-litigate without a dedicated plan)

- **Cover art rendering** (`src/ui/cover/mod.rs`): writes Kitty/SIXEL escape sequences directly to stdout outside ratatui's buffer — inherent to native terminal graphics. Now stable: payloads are sent once per change and the protocols are mutually exclusive (see `progress/2026-08-03-cover-refactor.md`). The chafa subprocess runs with `--probe off` — its default OSC 10/11 terminal probe was the root cause of the phantom keys (responses landed on stdin; commit `3a03ac0`). Residual: the half-block fallback still renders underneath a native overlay (cached, acceptable).
- `tmper play <directory>` (directory playback) is NOT implemented — CLI accepts a single file only.
- MPRIS2, EQ, online lyrics, notifications are not implemented.

## Dependencies (core)

tokio, ratatui, crossterm, rodio, symphonia, lofty, clap, toml, serde, serde_json, tracing, tracing-subscriber, anyhow, thiserror, dirs, rand, walkdir, regex, rustfft, rusqlite (bundled), encoding_rs, unicode-width, image (jpeg/png), base64

## Commands

```bash
# Build
cargo build
cargo build --release

# Run
cargo run -- play <path/to/audio>

# Test
cargo test                        # all tests (audio tests need an ALSA/Pulse device;
                                  # CI provides a null device via ~/.asoundrc)
cargo test <test_name>            # single test

# Lint & Format
cargo clippy -- -D warnings
cargo fmt --all

# Full pre-commit check
cargo fmt --all && cargo clippy -- -D warnings && cargo test
```

## Testing Conventions

- Test audio files live in `tests/fixtures/` (`test.wav`, `test.flac`, `test_notags.wav`; regenerate with ffmpeg: `ffmpeg -f lavfi -i "sine=frequency=440:duration=2" -ar 44100 -ac 2 tests/fixtures/test.wav`)
- Decoder/metadata tests are headless-safe (pure file I/O). App/engine tests construct a real `AudioEngine` and need an audio device — on a headless machine they fail at construction unless a null ALSA device is configured
- Logical modules have `#[cfg(test)] mod tests { ... }` inline
- Tests follow Arrange-Act-Assert pattern; cover normal paths + boundary conditions
