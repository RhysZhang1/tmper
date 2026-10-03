# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Terminal music player (codename: **tmper**) — a terminal-native music player for Arch Linux/KDE Plasma. Written in Rust with ratatui TUI framework. Supports multi-format audio decoding, metadata display, cover art, LRC lyrics syncing, spectrum visualizer, playlist management, a SQLite library index, and Vim-style keyboard navigation.

The project is **implemented and working** (~25,200 lines of Rust, 527 tests + 7 device-gated) and is **two processes in one binary**: a long-lived `tmper daemon` that owns the sound, the queue and the library, and a TUI client that attaches to it over a unix socket. Closing the TUI does not stop the music. The daemon also publishes the player on the session bus as an MPRIS2 player, so Plasma media controls, media keys and `playerctl` drive the same state the TUI shows.

The source of truth for the architecture is `DESIGN.md`; `STATUS.md` holds current capabilities/limits/plan; per-session change logs live in `progress/`. All docs (CLAUDE.md / README.md / DESIGN.md / STATUS.md) were reconciled with the code on 2026-10-03.

## Layout

Everything is XDG-based (`src/paths.rs`); the old project-local `config/` + `data/` layout is migrated automatically on first run (files are copied, originals kept).

- **Config**: `$XDG_CONFIG_HOME/tmper/` (usually `~/.config/tmper/`) — `config.toml`, optional `keybindings.toml` (hardcoded defaults exist), optional `themes/<name>.toml` overrides. The default config and the 5 palettes are compiled into the binary, so an installed binary needs no data files.
- **Data**: `$XDG_DATA_HOME/tmper/` (usually `~/.local/share/tmper/`) — `library.db` (SQLite + FTS5), exported `{name}.m3u`.
- **State**: `$XDG_STATE_HOME/tmper/` (usually `~/.local/state/tmper/`) — `state.json` (volume/repeat/queue/position/lyrics-offset, restored by the daemon at startup), `playlists.json`, `library.json`, `tmper.log` (client), `tmper-daemon.log` (daemon).
- **Runtime**: `$XDG_RUNTIME_DIR/tmper/socket` — the IPC endpoint, mode 0600, removed by the daemon on exit and replaced at startup if the file it finds refuses connections (a socket left by a daemon that died).
- **Cache**: `$XDG_CACHE_HOME/tmper/` — cover art for `mpris:artUrl`, newest 8 kept.
- Portable/override hook: `TMPER_CONFIG_DIR`, `TMPER_DATA_DIR`, `TMPER_STATE_DIR`, `TMPER_RUNTIME_DIR` env vars.
- **Themes**: 5 palettes (tokyo-night, dracula, nord, solarized-dark, catppuccin-mocha) embedded via `include_str!` and loaded by `src/ui/theme.rs`; a user copy in `$XDG_CONFIG_HOME/tmper/themes/` wins. UI colors come from `UiState.theme`, never hardcoded.

## Key Architecture Decisions

### Language: Rust
- ratatui (most mature Rust TUI framework) + crossterm terminal backend
- rodio + symphonia for audio (pure Rust, no system ffmpeg dependency, single binary distribution)
- Single binary: `cargo build --release`. The daemon is not a second executable — it re-execs `current_exe() daemon`, detached.

### Two processes, one binary
- `main.rs` dispatches on the verb: `daemon` → `daemon::run()`; `pause`/`next`/`status`/`quit`/… → `client::run_control()`; everything else → the TUI. `Command::controls_a_running_player()` is that split: a one-shot verb talks to a player that is already running and **never starts one**.
- **The wire contract is `src/ipc/proto.rs`** — `Request` (client → daemon) and `Event` (daemon → client, *including the answers*). Asymmetric on purpose: the TUI sits in a `select!` and must never block waiting for a reply. NDJSON, one object per line, `PROTOCOL_VERSION` checked in the `Hello`/`Welcome` handshake.
- **State is pushed whole**, never as deltas: a `StateSnapshot` per tick carries position, status, metadata, volume, repeat, queue revision, cover path. A full snapshot means a reconnecting client needs no catch-up protocol — the next snapshot *is* the catch-up. The queue and the playlist store are pushed whole whenever they change.
- **The daemon never waits for a client.** Each connection has a bounded mailbox; a full mailbox drops snapshots and spectrum frames (the next one supersedes them) and drops the *client* if it missed something unrecoverable (a queue change, a notice). Playback is never blocked by a stalled peer.
- **`Player` is `!Send`** (the cpal stream inside it), so `daemon::run`'s loop owns it outright and connection tasks reach it only through a channel — the audio stack's own single-writer rule.

### Client-side handles: one trait, two delivery timings
- `PlayerHandle` (`src/app/handle.rs`): `LocalHandle` (owns a `Player`, answers synchronously — **tests only**) and `DaemonHandle` (socket; a request returns nothing and the events arrive on a later tick).
- **The timing differs; nothing else may.** Both feed the same `App::apply_event`, so the ~200 tests driving a `LocalHandle` exercise the code the socket client runs, one tick's latency aside. Never add a second synchronous path for tests.
- `Connection` is a **state** (`watch`), not an event: a toast times out, "the player is gone" does not. `DaemonHandle` supervises the socket — when it dies, a banner goes up on row 1, requests queue in the outbox, and the supervisor dials a replacement, starting a daemon itself at most once per outage (`DAEMON_RECONNECT_RETRY_MS` between attempts).

### Concurrency Model (client)
- **tokio** async runtime in `App::run()`; main loop uses `tokio::select!` over an event channel and a tick interval
- **Burst-mode input thread**: a background thread `poll()`s crossterm and batch-`read()`s events into an `mpsc::unbounded_channel` of `Vec<CrosstermEvent>`; the loop processes the batch and draws ONCE (handles key auto-repeat without scroll-after-release)
- Audio decode and seek run on background threads (`tokio::task::spawn_blocking`), feeding a **bounded streaming session**: each play/seek opens a new session (fresh `Sink`, incremented generation) and decoding is throttled by backpressure to a ~2s high-water mark, so a long file never decodes fully into memory. Stale sessions are cancelled via flag + generation comparison.
- **Never `Sink::stop()` a sink a live decode thread may still append to** (`AudioOutput::stop_and_replace` retires it by dropping instead). rodio's `stop` only sets a flag, and the *next* `append` on a flagged sink with `sound_count > 0` blocks in `sleep_until_end` waiting for a "sound ended" signal that nothing sends on an unpolled sink. A decode thread between two appends parks there for good, and `Runtime::drop` then waits forever for that blocking task — an intermittent hang of the whole process. Dropping the sink silences it the same way (`stopped` + `keep_alive_if_empty=false`) and cannot land under a live append. Guarded by `audio::output::tests::a_retired_sink_still_accepts_appends`.
- FFT runs on the daemon's own thread writing into a shared `Arc<Mutex<VecDeque<f32>>>` ring buffer, and is published as `Event::Visualizer` **only while some client is subscribed** (`SubscribeVisualizer`); the client reads the bars out of the snapshot stream.
- UI rendering is read-only over `&UiState`; all mutation happens in `App::handle_event`

### Reading the socket: one line, one owner
- **A `BufReader` dropped after one line swallows every byte behind it**, and a read future dropped mid-line does the same — `read_line` reads *everything* available into its buffer to answer once. Two rules follow, and both are load-bearing:
  - The handshake reads `Welcome` on the same reader that then becomes the client's read task (`DaemonHandle::dial` → `read_events`); the reader is never re-created.
  - **The socket reader is never a `select!` branch.** In the daemon's `serve`, the request forwarder is one (cancelling a `send` on a channel is safe), while reading runs in its own spawned task that reports the reason the connection ended. This was a real bug: clients intermittently saw `the player closed the connection` (3 of 25 runs) because the handshake's reader was dropped with the greeting still in its buffer. See `progress/2026-10-03-daemon-split.md`.

### Audio Pipeline
```
Audio file → Symphonia (format probe + decoder) → PCM f32 samples   [daemon]
  ├─ Rodio Sink (fresh per session, fed from background thread, ~2s prebuffer) → sound card
  └─ Ring buffer → FFT thread → SpectrumProcessor → Event::Visualizer → UiState.visualizer_data
```

### MPRIS2 (daemon only, `src/player/mpris.rs`)
- `mpris-server` 0.10 over zbus, name `org.mpris.MediaPlayer2.tmper`. Started **after** the socket is advertised and never fatally: no session bus (SSH, plain console) logs and plays on.
- Snapshots feed MPRIS from the same place they feed clients, diffed against a mirror so only changed properties are announced — a snapshot arrives ~31×/s and almost all of them differ only in position, which is not an MPRIS property.
- `RepeatMode` is three-state, MPRIS is two flags: `Sequential→LoopStatus::None`, `SingleTrack→Track`, `Shuffle→Playlist` + `Shuffle(true)`. A setter records **only the flag the client named** into the mirror (zbus announces from the getter the moment the setter returns), and `set_shuffle(false)` reads the loop axis back before choosing `Sequential` vs `SingleTrack` — "stop shuffling" is not "stop looping". `to_volume` rounds to 3 decimals so the value round-trips. `SeekRelative` also emits `Seeked`.

## Project Structure

```
src/
├── main.rs             # verb dispatch; per-process log file (client vs daemon)
├── cli.rs              # clap verbs; controls_a_running_player() splits TUI from control
├── client.rs           # one-shot verbs: connect, send one request, wait, print, exit
├── daemon.rs           # listener, per-client tasks, idle exit, MPRIS wiring, Daemon policy
├── ipc/
│   ├── mod.rs          # NDJSON framing: read_message / write_message (+ async)
│   └── proto.rs        # Request / Event / StateSnapshot / RepeatMode — the wire contract
├── player/             # ★ the daemon's kernel
│   ├── mod.rs          #   Player: engine + queue + policy + execute()/tick()
│   ├── library.rs      #   LibraryDb + scan task registry + library.json
│   ├── playlists.rs    #   playlist store (identity is an id, never a name or a position)
│   ├── persistence.rs  #   state.json — the only writer of it
│   ├── cover.rs        #   cover cache for mpris:artUrl (newest 8 kept)
│   ├── fft.rs          #   FFT thread + subscription
│   └── mpris.rs        #   MPRIS2 interface
├── app/                # ★ the TUI client
│   ├── mod.rs          #   App struct, run() event loop, burst-mode input, terminal probe
│   ├── handle.rs       #   PlayerHandle + DaemonHandle (socket) + LocalHandle (cfg(test))
│   ├── playback.rs     #   move_selection, play_selected, lyrics loading
│   └── handlers/       #   key dispatch (mod/playlist/library/browser/settings)
├── audio/              # engine.rs (AudioEngine), decoder.rs (Symphonia), output.rs (rodio Sink)
├── metadata/           # lofty tag reader (title/artist/album/cover art)
├── lyrics/             # LRC parser + sync engine + types
├── visualizer/         # fft.rs, processor.rs (daemon side), render.rs (client side)
├── library/            # database.rs (SQLite + FTS5), scanner.rs, playlist_manager.rs (M3U)
├── ui/                 # theme.rs, mod.rs (UiState + render), cover/, views/ (7), widgets/
├── input/              # keymap.rs (bindings), handler.rs (double-key gg/dd), command.rs (:cmd)
├── config.rs           # Config structs + load/ensure + clamps (only ~8 live keys)
├── event.rs            # AppEvent enum (Key, Tick, Quit, JumpTop, RemoveSelected)
├── playlist.rs         # PlaylistData { id, name, songs } — single playlist model
├── paths.rs            # XDG config/data/state/runtime/cache dirs + legacy migration + socket_path
├── constants.rs        # runtime tuning constants (timeouts, buffer sizes, FPS, idle exit)
└── error.rs            # AppError (thiserror)
```

`library/` and `visualizer::{fft,processor}` are daemon-side now (only `player/` imports them); `audio::engine` is the daemon's private implementation but keeps its `pub` surface. `src/app/mod.rs` is the client and was deliberately **not** renamed — the rename would have touched ~150 tests and four documents for cosmetics.

## Key Design Patterns

- **Single writer, read-only readers**: `App` (event loop) mutates `UiState`; `ui::render` only reads it. Render functions take explicit read-only `Params` structs (e.g. `PlayerViewParams`) rather than the whole state. The same rule across processes: `state.json`, `library.db`, `playlists.json` and `library.json` each have exactly one writer (the daemon).
- **One event, one applier**: everything a `PlayerHandle` produces goes through `App::apply_event`. Two handles differ in *when* events arrive, never in what is done with them.
- **Background decode/seek**: heavy work is `spawn_blocking`, results land via shared `Arc` handles; the daemon's loop stays responsive.
- **Immutable-ish updates**: state structs use `Default` + `..Default::default()`; version counters use `Cell` (`cover_gen`, `visible_rows`).
- **Error handling**: `AppResult<T>` / `AppError` (thiserror) for public APIs; background-thread errors go to `tracing` logs (never panics).
- **Config priority**: CLI args > `$XDG_CONFIG_HOME/tmper/config.toml` > compiled-in defaults. Unknown keys are ignored (no `deny_unknown_fields`). A few keys (`num_bars`, `smoothing`) live in the client's file but are consumed by the daemon's FFT thread — the client pushes them with `SetFftParams`.
- **Config reality check**: only `default_volume`, `seek_step_small_secs`, `num_bars`, `frame_rate`, `smoothing`, `theme`, `show_cover_art`, `cell_px` are live. Do not re-add speculative keys without a consuming implementation.

## Known Architectural Debt (do NOT re-litigate without a dedicated plan)

- **Cover art rendering** (`src/ui/cover/mod.rs`): writes Kitty/SIXEL escape sequences directly to stdout outside ratatui's buffer — inherent to native terminal graphics. Now stable: payloads are sent once per change and the protocols are mutually exclusive (see `progress/2026-08-03-cover-refactor.md`). The half-block art and the graphics layer are placed against the *same* aspect-fitted box (`player_view::fit_cover_rect`), and the blocks stand aside while a native image is up. **SIXEL is encoded in-process** by `IcySixelEncoder` (`icy_sixel`), asked for the box's exact pixels — no subprocess, no second program's idea of the cell size. That second cell size was the chafa-era bug: chafa multiplies its `--size` by the cell it reads from the terminal itself (not the probe answer tmper has), so the two had to be reconciled by `chafa_cell_px()`; the encoder now takes the box and nothing else, and the whole class of mismatch is gone (see `progress/2026-10-03-chafa-cell-units.md`, `progress/2026-10-03-icy-sixel-encoder.md`). Cell size — the box shape — comes from a one-shot startup probe (`probe_terminal_once`, falling back to `TIOCGWINSZ` and then 10×20, overridable with `[ui] cell_px`). The same round trip asks **both graphics questions** — the Kitty protocol's own `a=q` APC query and primary device attributes (`CSI c`, attribute 4 = sixel) — and writes a payload only for a protocol the terminal answered for; one that answers neither keeps the block art instead of an empty panel. Selection order is Kitty → SIXEL → blocks. Kitty detection used to read environment variables, which is how the user's own terminal (Konsole 26.08, which speaks the protocol and sets none of them) was missed. A multiplexer (`TMUX`/`STY`/`ZELLIJ`) short-circuits the whole thing to block art: its query answers can still come from the real terminal underneath while the payload gets swallowed. The Kitty payload itself has three rules that all fail *silently* — `f=100` means the bytes must be a real **PNG** (raw RGB was sent under that header for a while: right pixels, right size, image dropped), `p` is a placement id rather than a pixel offset (position with `CSI <row>;<col> H` instead), and only the first chunk of a split transmission carries the command header (later ones are `\x1b_Gm=<more>;<data>\x1b\\`). Every graphics command carries `q=2`: an error reply is an APC sequence, and one arriving after startup is read as *keystrokes*. All of this is verified against a real Konsole by capturing tmper's byte stream under a pty and replaying it into a terminal window — unit tests cannot see it (see `progress/2026-10-03-terminal-graphics-probe.md`). The three tiers are additionally measured end-to-end on real terminals (2026-10-04, `progress/2026-10-04-terminal-compat-matrix.md`): Konsole and kitty take Kitty, foot and `xterm -ti vt340` take SIXEL, default xterm and alacritty take blocks — each confirmed from a screenshot, not from the probe's own log line, since a terminal that answers "yes" and paints nothing is the failure that matters. **xterm compiles sixel in but only advertises it on DA1 at VT340**, so a default xterm's "no" is truthful, not a parse bug.
- `tmper play <directory>` is NOT implemented — the CLI accepts a single file. Directories enter the library through the file browser instead (`a` scans the highlighted directory incrementally; `c` cancels).
- **Library ingestion is index-based**: `scan_incremental` prunes the index on a completed walk, so it is gated on a `complete` flag — a partial walk (unreadable subtree) must never prune, and prefix queries must not use `LIKE` (wildcards + ASCII case-insensitivity over-match, and every over-match deletes a real track).
- **`spawn_detached` refuses under `cfg(test)`** (`src/daemon.rs`): `current_exe()` inside a test binary is the harness, so spawning it would re-run the whole suite, detached. The client's supervisor logs and keeps retrying instead.
- EQ, online lyrics and desktop notifications are not implemented. MPRIS2 is (see above).

## Dependencies (core)

tokio, ratatui, crossterm, rodio, symphonia, lofty, clap, toml, serde, serde_json, tracing, tracing-subscriber, anyhow, thiserror, dirs, rand, walkdir, regex, rustfft, rusqlite (bundled), encoding_rs, unicode-width, image (jpeg/png), base64, mpris-server (zbus)

## Commands

```bash
# Build
cargo build
cargo build --release

# Run
cargo run -- play <path/to/audio>     # opens the TUI on that file
cargo run -- status                   # talks to a player that is already running
cargo run -- daemon                   # run the player in the foreground (debugging)

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
- The default suite is **device-free**: daemon-side tests build a `Player` on a headless engine (`AudioEngine::new_headless` → `Sink::new_idle()`), and the TUI tests drive a `LocalHandle` over it, so `cargo test` passes with no sound card. Exactly 6 tests touch real output; they are named `audio_output_*` and marked `#[ignore]`
- **Socket tests serialize on a process-wide `tokio::sync::Mutex`** (`app::handle::tests::listener()` returns the guard with the listener). There is one socket path per test binary, and parallel tests binding it delete each other's listener — the resulting `ENOENT` reads exactly like a reconnect bug.
- `cfg(test)` redirects every XDG directory into a per-process `test_root()`, including the runtime dir; nothing in the suite touches the real `~/.local/state/tmper`
- Logical modules have `#[cfg(test)] mod tests { ... }` inline
- Tests follow Arrange-Act-Assert pattern; cover normal paths + boundary conditions
- **Line coverage 89.44%** (measured 2026-10-04 via `cargo llvm-cov`; a few timing-sensitive tests
  make this wobble by ~0.3% between runs). What remains is structural, not neglected:
  `audio/engine.rs` (78% — the six `#[ignore]`d device tests count as uncovered, plus `new` and
  `play_file` need a real sound card), `audio/output.rs` (66% — `new` opens a device; the headless
  path is covered), `app/mod.rs` (72% — `TerminalGuard` and the `run` event loop
  need a real tty; the terminal probe is called from there), `paths.rs` (59% — the non-test XDG
  branches are not compiled under `cfg(test)`), `player/mpris.rs` (86% — zbus's interface layer
  needs a real session bus; the mapping, the property diff and the setter mirror are all tested),
  `main.rs` (58% — the entry point and the subscriber setup need a real run; `open_log` is tested
  directly, which is why it was extracted from `init_logging`). The probe's
  read loop is covered anyway: `query_terminal_on` takes its fd, so the tests drive it over a pipe.
  Do not claim "covered" from test counts alone — run `cargo llvm-cov --all-features --workspace`.
