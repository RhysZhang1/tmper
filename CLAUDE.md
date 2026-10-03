# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Terminal music player (codename: **tmper**) — a terminal-native music player for Arch Linux/KDE Plasma. Written in Rust with ratatui TUI framework. Supports multi-format audio decoding, metadata display, cover art, LRC lyrics syncing, spectrum visualizer, playlist management, a SQLite library index, and Vim-style keyboard navigation.

The project is **implemented and working** (~15,000 lines of Rust, 353 tests + 6 device-gated). The source of truth for the architecture is `DESIGN.md`; `STATUS.md` holds current capabilities/limits/plan; per-session change logs live in `progress/`. All docs (CLAUDE.md / README.md / DESIGN.md / STATUS.md) were reconciled with the code on 2026-10-03.

## Layout

Everything is XDG-based (`src/paths.rs`); the old project-local `config/` + `data/` layout is migrated automatically on first run (files are copied, originals kept).

- **Config**: `$XDG_CONFIG_HOME/tmper/` (usually `~/.config/tmper/`) — `config.toml`, optional `keybindings.toml` (hardcoded defaults exist), optional `themes/<name>.toml` overrides. The default config and the 5 palettes are compiled into the binary, so an installed binary needs no data files.
- **Data**: `$XDG_DATA_HOME/tmper/` (usually `~/.local/share/tmper/`) — `library.db` (SQLite + FTS5).
- **State**: `$XDG_STATE_HOME/tmper/` (usually `~/.local/state/tmper/`) — `state.json` (volume/repeat/lyrics-offset, restored at startup), `playlists.json`, `library.json`, `tmper.log`.
- Portable/override hook: `TMPER_CONFIG_DIR`, `TMPER_DATA_DIR`, `TMPER_STATE_DIR` env vars.
- **Themes**: 5 palettes (tokyo-night, dracula, nord, solarized-dark, catppuccin-mocha) embedded via `include_str!` and loaded by `src/ui/theme.rs`; a user copy in `$XDG_CONFIG_HOME/tmper/themes/` wins. UI colors come from `UiState.theme`, never hardcoded.

## Key Architecture Decisions

### Language: Rust
- ratatui (most mature Rust TUI framework) + crossterm terminal backend
- rodio + symphonia for audio (pure Rust, no system ffmpeg dependency, single binary distribution)
- Single binary: `cargo build --release`

### Concurrency Model
- **tokio** async runtime in `App::run()`; main loop uses `tokio::select!` over an event channel and a tick interval
- **Burst-mode input thread**: a background thread `poll()`s crossterm and batch-`read()`s events into an `mpsc::unbounded_channel` of `Vec<CrosstermEvent>`; the loop processes the batch and draws ONCE (handles key auto-repeat without scroll-after-release)
- Audio decode and seek run on background threads (`tokio::task::spawn_blocking`), feeding a **bounded streaming session**: each play/seek opens a new session (fresh `Sink`, incremented generation) and decoding is throttled by backpressure to a ~2s high-water mark, so a long file never decodes fully into memory. Stale sessions are cancelled via flag + generation comparison.
- FFT analysis runs on its own thread writing into a shared `Arc<Mutex<VecDeque<f32>>>` ring buffer; the UI reads a `Vec<f32>` snapshot each tick
- UI rendering is read-only over `&UiState`; all mutation happens in `App::handle_event`

### Audio Pipeline
```
Audio file → Symphonia (format probe + decoder) → PCM f32 samples
  ├─ Rodio Sink (fresh per session, fed from background thread, ~2s prebuffer) → sound card
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
├── library/            # database.rs (SQLite + FTS5), scanner.rs (incremental directory scan), playlist_manager.rs (M3U)
├── ui/                 # theme.rs, mod.rs (UiState + render), cover/, views/ (7), widgets/
├── input/              # keymap.rs (bindings), handler.rs (double-key gg/dd), command.rs (:cmd)
├── config.rs           # Config structs + load/ensure + clamps (only ~7 live keys)
├── event.rs            # AppEvent enum (Key, Tick, Quit, JumpTop, RemoveSelected)
├── cli.rs              # clap: `tmper play <file>`
├── playlist.rs         # PlaylistData { name, songs: Vec<PathBuf> } — single playlist model
├── paths.rs            # XDG config/data/state dirs + legacy-layout migration
├── constants.rs        # runtime tuning constants (timeouts, buffer sizes, FPS)
└── error.rs            # AppError (thiserror)
```

## Key Design Patterns

- **Single writer, read-only readers**: `App` (event loop) mutates `UiState`; `ui::render` only reads it. Render functions take explicit read-only `Params` structs (e.g. `PlayerViewParams`) rather than the whole state.
- **Background decode/seek**: heavy work is `spawn_blocking`, results land via shared `Arc` handles; the main thread stays responsive.
- **Immutable-ish updates**: state structs use `Default` + `..Default::default()`; version counters use `Cell` (`cover_gen`, `visible_rows`).
- **Error handling**: `AppResult<T>` / `AppError` (thiserror) for public APIs; background-thread errors go to `tracing` logs (never panics).
- **Config priority**: CLI args > `$XDG_CONFIG_HOME/tmper/config.toml` > compiled-in defaults. Unknown keys are ignored (no `deny_unknown_fields`).
- **Config reality check**: only `default_volume`, `seek_step_small_secs`, `num_bars`, `frame_rate`, `smoothing`, `theme`, `show_cover_art`, `cell_px` are live. Do not re-add speculative keys without a consuming implementation.

## Known Architectural Debt (do NOT re-litigate without a dedicated plan)

- **Cover art rendering** (`src/ui/cover/mod.rs`): writes Kitty/SIXEL escape sequences directly to stdout outside ratatui's buffer — inherent to native terminal graphics. Now stable: payloads are sent once per change and the protocols are mutually exclusive (see `progress/2026-08-03-cover-refactor.md`). The half-block art and the graphics layer are placed against the *same* aspect-fitted box (`player_view::fit_cover_rect`), and the blocks stand aside while a native image is up. **SIXEL is encoded in-process** by `IcySixelEncoder` (`icy_sixel`), asked for the box's exact pixels — no subprocess, no second program's idea of the cell size. That second cell size was the chafa-era bug: chafa multiplies its `--size` by the cell it reads from the terminal itself (not the probe answer tmper has), so the two had to be reconciled by `chafa_cell_px()`; the encoder now takes the box and nothing else, and the whole class of mismatch is gone (see `progress/2026-10-03-chafa-cell-units.md`, `progress/2026-10-03-icy-sixel-encoder.md`). Cell size — the box shape — comes from a one-shot startup probe (`probe_terminal_once`, falling back to `TIOCGWINSZ` and then 10×20, overridable with `[ui] cell_px`). The same round trip asks for **primary device attributes** (`CSI c`) and only writes a SIXEL payload when the terminal answers with attribute 4; a terminal that cannot display one keeps the block art instead of an empty panel.
- `tmper play <directory>` is NOT implemented — the CLI accepts a single file. Directories enter the library through the file browser instead (`a` scans the highlighted directory incrementally; `c` cancels).
- **Library ingestion is index-based**: `scan_incremental` prunes the index on a completed walk, so it is gated on a `complete` flag — a partial walk (unreadable subtree) must never prune, and prefix queries must not use `LIKE` (wildcards + ASCII case-insensitivity over-match, and every over-match deletes a real track).
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
cargo test                        # default: everything that needs no audio device
cargo test <test_name>            # single test
cargo test audio_output_ -- --ignored --test-threads=1   # needs a real/virtual device
                                  # (CI provides a null ALSA device via ~/.asoundrc)

# Lint & Format
cargo clippy -- -D warnings
cargo fmt --all

# Full pre-commit check
cargo fmt --all && cargo clippy -- -D warnings && cargo test

# Coverage (line coverage — one-time setup, then reusable)
rustup component add llvm-tools-preview   # one-time (needed by llvm-cov)
cargo install cargo-llvm-cov --locked     # one-time
cargo llvm-cov --all-features --workspace # prints per-module line coverage + a total
```

## Testing Conventions

- Test audio files live in `tests/fixtures/` (`test.wav`, `test.flac`, `test_notags.wav`; regenerate with ffmpeg: `ffmpeg -f lavfi -i "sine=frequency=440:duration=2" -ar 44100 -ac 2 tests/fixtures/test.wav`)
- The default suite is **device-free**: App/engine tests build a headless engine (`App::new_headless` → `AudioEngine::new_headless` → `Sink::new_idle()`), so `cargo test` passes with no sound card. Exactly 6 tests touch real output; they are named `audio_output_*` and marked `#[ignore]`
- Logical modules have `#[cfg(test)] mod tests { ... }` inline
- Tests follow Arrange-Act-Assert pattern; cover normal paths + boundary conditions
- **Line coverage 88.54%** (measured 2026-10-03 via `cargo llvm-cov`; a few timing-sensitive tests
  make this wobble by ~0.3% between runs). What remains is structural, not neglected:
  `audio/engine.rs` (73% — the six `#[ignore]`d device tests count as uncovered, plus `new` and
  `play_file` need a real sound card), `app/mod.rs` (62% — `TerminalGuard` and the `run` event loop
  need a real tty; the terminal probe is called from there), `paths.rs` (29% — the non-test XDG
  branches are not compiled under `cfg(test)`), `main.rs` (0% — the binary entry point). The probe's
  read loop is covered anyway: `query_terminal_on` takes its fd, so the tests drive it over a pipe.
  Do not claim "covered" from test counts alone — run `cargo llvm-cov --all-features --workspace`.
