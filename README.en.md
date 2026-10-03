# tmper

**A music player for your terminal — close the window, the music keeps playing.**

[![CI](https://github.com/RhysZhang1/tmper/actions/workflows/ci.yml/badge.svg)](https://github.com/RhysZhang1/tmper/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/RhysZhang1/tmper?label=release)](https://github.com/RhysZhang1/tmper/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.90%2B-orange.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-Linux-lightgrey.svg)](#requirements)

[中文](README.md) | English

---

## What this is

tmper is a local music player that lives in your terminal: Vim-style keys, scrolling LRC lyrics,
a live spectrum, native cover art, and a full-text searchable SQLite library — all in one binary,
with **no ffmpeg and no external programs**.

What sets it apart from most terminal players is that **it is two processes.** A long-lived
player daemon owns the sound, the queue and the library; the TUI is just a view attached to it.
So:

- Close the terminal and the music does not stop. Reopen the TUI and you are back on the same
  track, at the same second.
- It publishes itself on the session bus as a standard MPRIS2 player, so **Plasma's media widget,
  your keyboard's media keys and `playerctl` all drive the same state the TUI shows.**
- Library scanning, playlist editing and playback all belong to the daemon. Several views can
  attach at once and see the same truth.

The daemon doesn't linger forever once you leave: with no client attached and nothing sounding
for five minutes it writes the queue and playback position to `state.json` and exits. The next
`tmper play` picks up from that second.

```
┌ Now Playing ─────────────────────┐┌ Lyrics ────────────────────────────────────────────────────────────────┐
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││Sends shivers down my spine                                             │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││Body's aching all the time                                              │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││Goodbye, everybody, I've got to go                                      │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││Gotta leave you all behind and face the truth                           │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││Mama, ooh (any way the wind blows)                                      │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││I don't wanna die                                                       │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││I sometimes wish I'd never been born at all                             │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││I see a little silhouetto of a man                                      │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││Scaramouche, Scaramouche, will you do the Fandango?                     │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││Thunderbolt and lightning, very, very frightening me                    │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││(Galileo) Galileo, (Galileo) Galileo, Galileo Figaro                    │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││                                                                        │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││                                                                        │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  ││                                                                        │
│  ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄  │└────────────────────────────────────────────────────────────────────────┘
└──────────────────────────────────┘┌ Spectrum ──────────────────────────────────────────────────────────────┐
┌ Playlists ───────────────────────┐│            ▂▂                                                          │
│▼ Late Night                      ││            ██                                                          │
│◄ Bohemian Rhapsody               ││            ██      ▇▇        ▃▃                                        │
│   Love of My Life                ││          ▆▆██▂▂    ██        ██        ▃▃                              │
│   Don't Stop Me Now              ││          ██████    ██      ▅▅██        ██                              │
│   Somebody to Love               ││          ██████  ████▁▁    ████      ▆▆██                              │
│   Under Pressure                 ││        ▅▅██████▁▁██████    ████▄▄    ████        ▇▇                    │
│▶ Focus                           ││        ████████████████  ▆▆██████  ▁▁████▇▇      ██▃▃      ▂▂          │
│                                  ││        ████████████████▃▃████████▁▁████████    ▄▄████      ██▂▂        │
│                                  ││      ██████████████████████████████████████▄▄  ██████▃▃  ▆▆████▂▂      │
│                                  ││    ▄▄██████████████████████████████████████████████████▅▅████████▄▄    │
│                                  │└────────────────────────────────────────────────────────────────────────┘
│                                  │ 🎵  Late Night  💿  A Night at the Opera  ♪ Rock  📅  1975  FLAC
│                                  │▶ 03:12 / 05:55 Vol:80% [███████████████░░░░░░░░░░░░░] ⟳ 顺 序 循 环
│                                  │
│                                  │
└──────────────────────────────────┘
```

> The image above is real renderer output
> (`cargo test -- --ignored --nocapture print_the_player_view`), not a drawing. A drawing is a
> second implementation, and a second implementation goes wrong quietly.
>
> It arrives here without colour: GitHub's Markdown has no ```` ```ansi ```` support, and the
> coloured version renders as noise. So the cover is a field of half-blocks and the spectrum is
> bare bars — in a terminal both are drawn in the theme's colours. Run the command above to see
> that version.

---

## Contents

- [Features](#features)
- [Requirements](#requirements)
- [Install](#install)
- [Quick start](#quick-start)
- [Usage](#usage)
- [Keybindings](#keybindings)
- [Command mode](#command-mode)
- [Configuration](#configuration)
- [FAQ](#faq)
- [How it works](#how-it-works)
- [Project layout](#project-layout)
- [Development](#development)
- [Stack](#stack)
- [License](#license)

---

## Features

### Playback and the process model
- MP3, FLAC, OGG Vorbis, Opus, WAV, AAC, ALAC, M4A, WMA, APE, WavPack, AIFF
- Decoded by [Symphonia](https://github.com/pdeljanov/Symphonia) in pure Rust — **no ffmpeg needed**
- Bounded streaming decode with a ~2 s prebuffer; skipping and seeking retire the old session, so
  a long file is never read into memory
- Real audio seek, volume, and sequential / shuffle / repeat-one
- **Closing the TUI does not stop the music**, and reopening it rejoins the same track and position
- **A player that dies mid-session does not take the TUI with it**: a banner appears, a
  replacement is dialled in the background, and keys pressed meanwhile queue up and are delivered
  in order once the connection is back

### Desktop integration (MPRIS2)
- Owns `org.mpris.MediaPlayer2.tmper` on the session bus, so **Plasma's media widget, media keys
  and `playerctl` work out of the box**
- Playback status, track metadata (with microsecond length), loop/shuffle and volume are all
  bidirectional: change it on the desktop and the TUI follows
- Cover art is exposed as a `file://` URL into `$XDG_CACHE_HOME/tmper/` (the newest 8 are kept)
- With no session bus at all (SSH, plain console) it logs one line and plays on — a missing bus
  is not a reason to refuse to start

### Cover art
- **Three tiers, chosen by a single startup probe:**
  1. **Kitty graphics protocol** — native pixels (Kitty, WezTerm, Ghostty, Konsole 26.08+)
  2. **SIXEL** — encoded in-process (Konsole on Plasma 6+, xterm, foot…), no external program
  3. **Half-blocks** — Lanczos3 resize plus Floyd–Steinberg dithering, the universal fallback
- Payloads are sent once per change, and the raster is sized to the exact pixel box of the cover
  rectangle — the same box the block art uses
- The probe asks the terminal two questions — the Kitty protocol's own `a=q` query and `CSI c`
  (DA1, attribute 4 = sixel) — and writes a payload **only for a protocol the terminal answered
  for**. VTE terminals (GNOME Terminal, xfce4-terminal) and Alacritty support neither, and get
  block art instead of an empty panel
- The cover box is aspect-fitted to the artwork and aligned using the terminal's reported cell
  size
- Embedded art is read from ID3v2 APIC / Vorbis Comments / MP4

### Library and metadata
- ID3v1/v2, Vorbis Comments, APE and MP4 tags, with fallbacks for missing fields
- Incremental background scanning that skips files whose fingerprint is unchanged; cancellable
- SQLite + FTS5 full-text search across title, artist, album and genre
- M3U import/export. A playlist's identity is an id — never a name, never a position

### Lyrics
- Standard and enhanced (word-level) LRC
- Encoding detection: UTF-8 → GBK → Shift-JIS
- A same-named `.lrc` next to the audio file loads automatically
- Live highlighting and fine offset control (`[` `]` ±0.5 s, `{` `}` ±2 s), full-screen view

### Spectrum
- Live FFT (2048-point Hann window, ~30 FPS), log-spaced buckets, green → yellow → red
- Decays when paused. The FFT **only runs while a client is subscribed** — nobody watching,
  no CPU and no bandwidth

### Interface
- **Seven views**: player `1` / library `2` / lyrics `3` / visualizer `4` / playlists `5` /
  file browser `6` / settings `7`
- Two-column player: cover and playlists on the left, lyrics, spectrum, info and controls on the right
- Vim-style modal keys, `gg` / `dd` sequences, `/` live search, `:` command mode, help on `8`
- Five built-in themes: Tokyo Night, Dracula, Nord, Solarized Dark, Catppuccin Mocha
- Custom keybindings in `$XDG_CONFIG_HOME/tmper/keybindings.toml`
- State is saved on exit: volume, loop mode, lyrics offset, **the queue and the position**. Next
  launch the track is cued and the progress bar sits at the second you left — pressing play
  carries on from there (starting up itself is silent)

---

## Requirements

- **Linux** (Arch, Ubuntu, Debian, Fedora…; macOS and Windows are untested)
- PulseAudio / PipeWire / ALSA
- **Rust 1.90 or newer** (SIXEL encoding happens in-process, so no `chafa`; the floor comes
  from the dependency chain rather than from tmper's own code — `quantette`, the quantiser
  behind `icy_sixel`, declares 1.90, and cargo refuses anything older)
- ALSA development headers at build time (`alsa-lib-devel` / `libasound2-dev`) — rodio links
  `libasound` through cpal

---

## Install

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

git clone https://github.com/RhysZhang1/tmper.git
cd tmper
cargo build --release
```

The result is `target/release/tmper`. The default config and all five themes are compiled into
the binary, so **copying that one file to another machine is enough to run it**; your config,
library and state still live in the XDG directories.

Or install it into Cargo's bin directory:

```bash
cargo install --path .
```

Upgrading from an old version needs nothing: the first run copies the old project-local
`config/` and `data/` layout into the XDG directories and leaves the originals alone.

---

## Quick start

```bash
tmper                          # open the interface
tmper play ~/Music/song.flac   # open it on a file
tmper status                   # ask the running player what it is doing
```

`j` / `k` move, `Enter` plays the selection, `Space` pauses, `q` closes the interface.

**To load a whole library**: press `6` for the file browser, navigate to your music directory and
press `a` to add it and scan in the background. When the scan finishes, press `2` for the library
view, `/` to search, `Enter` to play.

---

## Usage

### Background playback

The player and the interface are separate processes. Closing the interface leaves the daemon
playing; reopening the TUI rejoins the same track at the same position.

Without a file, these verbs are **messages to a player that is already running** — they never
start one. `tmper pause` on a machine where nothing is playing says so, rather than quietly
starting a player in order to pause it.

| Command | Effect |
|---------|--------|
| `tmper play` | Carry on (what a media key sends; after `stop`, starts from the beginning) |
| `tmper pause` | Pause, keeping the track loaded |
| `tmper next` / `tmper prev` | Next / previous track |
| `tmper stop` | Silence, release the device, rewind (the track stays loaded) |
| `tmper volume <0-100>` | Set the volume |
| `tmper status` | Print status, track, position, volume and queue length |
| `tmper quit` | Stop the player and exit the daemon |

`q` and `:quit` **only close the interface** — the music keeps playing. `:quit!` (or
`tmper quit`) stops the player too.

Run `tmper daemon` in the foreground to watch the player directly when something is wrong; its
log is `~/.local/state/tmper/tmper-daemon.log`.

### Lyrics

Put a same-named `.lrc` beside the audio file:

```
~/Music/
├── song.flac
└── song.lrc        ← loaded automatically
```

### Themes

```toml
# ~/.config/tmper/config.toml
[ui]
theme = "dracula"
```

Valid values: `tokyo-night`, `dracula`, `nord`, `solarized-dark`, `catppuccin-mocha`. You can
also change it live in the settings view (`7`) or with `:theme dracula`.

---

## Keybindings

| Key | Action |
|-----|--------|
| `Space` | Play / pause |
| `n` / `p` | Next / previous track |
| `Enter` | Play the selection |
| `-` / `=` | Volume down / up |
| `j` / `k` / `↓` / `↑` | Move down / up |
| `←` / `→` | Seek back / forward 5 s (real audio seek) |
| `g` `g` / `G` | Jump to top / bottom |
| `Ctrl+d` / `Ctrl+u` | Half-page down / up |
| `d` `d` | Remove the current track |
| `r` | Cycle repeat (sequential / shuffle / single) |
| `/` | Filter the queue (live; `j`/`k` pick, `Enter` plays) |
| `1`–`7` | Switch view |
| `8` | Help panel |
| `a` / `c` | In the browser: add / rescan a directory, cancel a scan |
| `[` `]` `{` `}` | Lyrics offset (±0.5 s / ±2 s) |
| `Ctrl+r` | Reset the lyrics offset |
| `:` | Command mode |
| `q` | Close the interface (music continues) |

---

## Command mode

| Command | Description |
|---------|-------------|
| `:q` / `:quit` | Close the interface (music continues) |
| `:q!` / `:quit!` | Stop playback and exit the daemon |
| `:help` | Show the help panel |
| `:version` | Print the version |
| `:theme <name>` | Switch theme |
| `:seek <secs>` | Seek (negative goes back) |
| `:volume <0-100>` | Set the volume |
| `:repeat <mode>` | sequential / shuffle / single |
| `:view <name>` | player / library / lyrics / visualizer / playlists / browser / settings |
| `:import <path>` | Import an M3U playlist |
| `:export <name>` | Export a playlist to M3U |

---

## Configuration

Everything follows the XDG spec:

- Config: `$XDG_CONFIG_HOME/tmper/` (usually `~/.config/tmper/`)
- Library and exported M3U: `$XDG_DATA_HOME/tmper/`
- State, playlists and logs: `$XDG_STATE_HOME/tmper/` (`tmper.log` is the client's,
  `tmper-daemon.log` the player's)
- Cover cache: `$XDG_CACHE_HOME/tmper/` (read by the desktop widget; newest 8 kept)
- Socket: `$XDG_RUNTIME_DIR/tmper/socket` (mode 0600, removed on exit)
- Override any of them with `TMPER_CONFIG_DIR`, `TMPER_DATA_DIR`, `TMPER_STATE_DIR`,
  `TMPER_RUNTIME_DIR` — useful for testing and portable installs

### ~/.config/tmper/config.toml

Generated from an embedded template on first run.

```toml
[playback]
default_volume = 0.8
seek_step_small_secs = 5

[visualizer]
num_bars = 32
frame_rate = 30
smoothing = 0.35

[ui]
theme = "tokyo-night"
show_cover_art = true
# Pixel size of one terminal cell. Leave unset to probe: CSI 16 t first
# (what Konsole answers), then CSI 14 t ÷ CSI 18 t. A terminal that answers
# neither gets a 10×20 assumption, which keeps the aspect right but the art
# a little small — set it by hand in that case, e.g. cell_px = [10, 20]
# cell_px = [10, 20]
```

Unknown keys are ignored, so comments and stale fields never stop the program from starting.

---

## FAQ

### Keys do nothing after startup?

Check `~/.local/state/tmper/tmper.log`. A terminal smaller than 30×8 shows a size notice. Also
check the interface actually reached the player: when it cannot, a banner appears at the top and
every retry is logged.

### No sound?

Check that PulseAudio / PipeWire / ALSA works with another player. The player's own log is
`~/.local/state/tmper/tmper-daemon.log`; a device that fails to open is reported there.

### The desktop widget / media keys don't see tmper?

```bash
playerctl -p tmper status                    # prints Playing/Paused/Stopped if this layer is fine
busctl --user list | grep tmper              # does it own org.mpris.MediaPlayer2.tmper?
```

No output means the daemon isn't running (`tmper play` first) or there is no session bus
(`echo $DBUS_SESSION_BUS_ADDRESS` is empty — SSH and plain consoles; playback still works, the
desktop just can't see it). If the bus layer is fine but Plasma still doesn't show it, the cause
is on the desktop side (the widget hidden, or another program holding the media keys); the
daemon's log says whether it reached the bus at startup.

### Why is the cover a block of pixels rather than the real image?

One of these applies:

- **Kitty / WezTerm / Ghostty / Konsole 26.08+** — native pixels via the Kitty protocol
- **Konsole on Plasma 6, xterm, foot…** — SIXEL, encoded in-process, nothing to install
- **Anything else** — half-block characters (`▄` with foreground/background doubling the
  vertical resolution)

The box is aspect-fitted and aligned using the cell size the terminal reports. That size is
probed once at startup and logged: `grep "cell size" ~/.local/state/tmper/tmper.log`. Terminals
that report neither source fall back to a 10×20 assumption — the aspect stays right but the art
may be smaller; set `cell_px = [w, h]` in `config.toml` to override.

The same probe asks whether the terminal supports a graphics protocol at all: the Kitty query for
Kitty, `CSI c` (DA1, attribute 4) for SIXEL. **Only a terminal that answers for a protocol gets
that payload.** Under `tmux` / `screen` / `zellij` the probe is skipped entirely and block art is
used: a multiplexer swallows the payload while the query may still be answered by the real
terminal underneath — which is exactly the "it says it's supported and the panel is blank"
combination.

This is also why Konsole 26.08 gets native pixels: it implements the Kitty protocol but sets none
of the environment variables, so detecting it by environment was never going to work.

### `q` left the music playing — is that a bug?

No, it is the design. `q` closes the interface only. Use `:quit!` or `tmper quit` to stop the
player as well.

### Which formats are supported?

MP3, FLAC, OGG Vorbis, Opus, WAV, AAC (.aac/.m4a), ALAC (.m4a), WavPack (.wv), WMA, AIFF, APE.

### macOS / Windows?

Linux only for now. macOS would probably compile (rodio supports CoreAudio) but is untested;
Windows is not supported.

---

## How it works

One binary, **two roles**:

```
$ tmper                    $ tmper daemon
   TUI client                  player daemon
   ├ event loop / render       ├ audio engine (rodio + symphonia)
   ├ key dispatch / lyrics     ├ queue and continuation policy
   └ connection / reconnect    ├ SQLite library / scanner / playlists
        │                      ├ state.json (its only writer)
        │                      ├ FFT thread
        │                      └ MPRIS2 (session bus)
        └────── unix socket ───┘
          $XDG_RUNTIME_DIR/tmper/socket
          line-delimited JSON, both ways
```

- **State is pushed whole, never as deltas.** A full snapshot per tick means a reconnecting
  client needs no catch-up protocol — the next snapshot *is* the catch-up.
- **The protocol is asymmetric.** Clients send `Request`s; everything the daemon says is an
  `Event`, *including the answers*. The TUI sits in a `select!` and must never block waiting for
  a reply.
- **The daemon never waits for a client.** Each connection has a bounded mailbox; when it fills,
  snapshots and spectrum frames are dropped (the next one supersedes them) and a client that
  missed something unrecoverable is dropped. Playback is never blocked by a slow peer.
- **One trait, two delivery timings.** The local handle used by tests answers synchronously; the
  socket handle's events arrive a tick later. Both feed the same `apply_event`, so the timings
  differ and nothing else does — there is no second synchronous path that only tests take.
- **One writer per file.** `state.json`, `library.db`, `playlists.json` and `library.json` are
  written by the daemon and nobody else.

The full architecture, its invariants and the bugs that shaped them are in
[DESIGN.md](DESIGN.md) (Chinese); current capabilities and limits are in [STATUS.md](STATUS.md).

---

## Project layout

```
tmper/
├── Cargo.toml                    # crate manifest
├── DESIGN.md                     # architecture and implementation constraints
├── STATUS.md                     # current capabilities, limits, near-term plan
├── README.md / README.en.md      # Chinese / this file
│
├── config/default.toml           # default config, compiled into the binary
├── themes/                       # five palettes, compiled into the binary
├── progress/                     # historical development notes; not current truth
│
├── src/                          # ~24,750 lines of Rust, one binary
│   ├── main.rs                   #   entry: dispatch on the verb
│   ├── cli.rs                    #   clap verbs
│   ├── client.rs                 #   one-shot verbs: connect, send, print, exit
│   ├── daemon.rs                 #   the player process
│   ├── ipc/                      #   mod.rs framing, proto.rs the wire contract
│   ├── player/                   #   the daemon's kernel
│   │   ├── mod.rs                #     engine + queue + continuation policy
│   │   ├── library.rs            #     index and scan tasks
│   │   ├── playlists.rs          #     playlists (identity is an id)
│   │   ├── persistence.rs        #     state.json
│   │   ├── cover.rs              #     cover cache for mpris:artUrl
│   │   ├── fft.rs                #     spectrum thread and subscriptions
│   │   └── mpris.rs              #     MPRIS2 interface
│   ├── app/                      #   the TUI client
│   │   ├── mod.rs                #     App + event loop
│   │   ├── handle.rs             #     PlayerHandle: socket and local
│   │   ├── playback.rs           #     selection and lyrics loading
│   │   └── handlers/             #     key dispatch
│   ├── audio/                    #   decoder, rodio output, engine
│   ├── metadata/ lyrics/ visualizer/ library/
│   ├── ui/                       #   theme, cover, views (7), widgets
│   └── input/                    #   keymap, two-key sequences, `:` commands
│
└── tests/fixtures/               # test audio
```

---

## Development

```bash
cargo build                        # debug
cargo build --release              # release (a single self-contained file)
cargo test                         # default: device-free tests (521)
cargo test audio_output_ -- --ignored --test-threads=1  # needs a real/virtual device (6)
cargo clippy --all-targets -- -D warnings
cargo fmt --all

# Full pre-commit check
cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test

# Line coverage (one-time setup, then reusable)
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --locked
cargo llvm-cov --all-features --workspace
```

**The default suite is device-free**: daemon-side tests build a `Player` on a headless engine and
the TUI tests drive a local handle, so `cargo test` is green with no sound card. Exactly six tests
touch real output; they are named `audio_output_*` and marked `#[ignore]`.

Coverage is **89.96% lines / 90.61% regions / 88.63% functions** (measured 2026-10-03 across 528 tests
= 521 default + 7 ignored: 6 device-gated and the screenshot tool above). What remains uncovered is structural rather than
neglected — see [DESIGN.md §10](DESIGN.md).

### Regenerating the screenshot

The screenshot at the top is real renderer output, not a drawing:

```bash
cargo test -- --ignored --nocapture print_the_player_view
```

It prints a plain-text block; paste it over the one in the READMEs. Do **not** add colour to
it: GitHub has no ```` ```ansi ```` support — it replaces every ESC with U+FFFD and leaves
`[0;38;2;…m` sitting in the picture, turning the screenshot into a couple of thousand
replacement characters. That mistake has been made once already.

---

## Stack

| Area | Crate | Notes |
|------|-------|-------|
| Async | tokio | event loops, timers, background tasks |
| TUI | ratatui + crossterm | the terminal framework |
| Audio | symphonia + rodio | decoding and output |
| Tags | lofty | ID3 / Vorbis / APE / MP4 |
| FFT | rustfft | 2048-point spectrum analysis |
| Database | rusqlite (bundled) | SQLite + FTS5 library index |
| Images | image + icy_sixel | cover decode, Lanczos3 resize, in-process SIXEL |
| Desktop | mpris-server (zbus) | MPRIS2: Plasma widget, media keys, playerctl |
| IPC | serde_json + unix socket | line-delimited JSON, no extra protocol crate |
| Config | toml + serde + clap | config file and command line |
| Encoding | encoding_rs | lyrics encoding detection |

---

## License

[MIT License](LICENSE). Copyright (c) 2026 Rhys Zhang
