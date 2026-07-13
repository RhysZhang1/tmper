# Bug Fixes & Code Cleanup — 2026-07-13

## Summary

Fixed 2 critical bugs and cleaned up extensive dead code across the project.

## Changes

### P0 — Bug Fixes

1. **`src/audio/output.rs`**: Fixed dangling handle in `stop_and_replace()` error path.
   - Old code created a local `OutputStream` in `unwrap_or_else` that was immediately dropped, leaving a dangling handle
   - New code logs an error and keeps the old (stopped) sink instead of creating a broken sink

2. **`src/paths.rs`**: Fixed path resolution for debug builds.
   - `project_root()` now uses `env!("CARGO_MANIFEST_DIR")` in debug builds, guaranteeing correct resolution to project root
   - Release builds retain exe-relative resolution (unchanged behavior)

### P1 — Dead Code Cleanup

3. **`src/event.rs`**: Removed unused `JumpBottom` and `VisualizerData` variants (never constructed)
4. **`src/app/handlers/mod.rs`**: Removed corresponding `JumpBottom`/`VisualizerData` match arms
5. **`src/input/handler.rs`**: Removed unused `handle_key()` fallback function
6. **`src/visualizer/render.rs`**: Simplified `CharSet` enum — removed unused `Braille`/`Ascii` variants
7. **`src/lyrics/types.rs`**: Removed unused `LyricMetadata::album`, `author`, `length` fields
8. **`src/lyrics/parser.rs`**: Removed corresponding set code and `parse_length()` helper
9. **`src/library/scanner.rs`**: Removed unused `ScanEvent` enum and `scan_library()` function
10. **`src/input/mod.rs`**: Commented out unused stub modules `command` and `keymap`
11. **`src/playlist.rs`**: Removed `SortKey` enum, moved test-only methods to `#[cfg(test)]` impl
12. **`src/metadata/reader.rs`**: Removed stale `#[allow(dead_code)]` on `read_metadata()`
13. **`src/audio/output.rs`**: Removed unused `len()` method

### P2 — Minor Optimizations

14. **`src/library/database.rs`**: Added `idx_search` composite index on `(title, artist, album)` for search performance
15. **`src/config.rs`**: Added field clamping (`volume`, `frame_rate`, `num_bars`, `smoothing`) to prevent invalid values

## Verification

- `cargo build` — clean, 0 warnings
- `cargo test` — 37/37 passed
- `cargo clippy -- -D warnings` — clean
- `cargo fmt --all` — formatted
