#!/usr/bin/env bash
# Reproducible complete local validation without changing user configuration.
set -euo pipefail
cd "$(dirname "$0")/.."
tmper_check_dir=$(mktemp -d -t tmper-validation-XXXXXX)
trap 'rm -rf "$tmper_check_dir"' EXIT
printf 'pcm.!default { type null }\nctl.!default { type null }\n' > "$tmper_check_dir/alsa.conf"
cargo fmt --all -- --check
cargo clippy --locked --all-targets --workspace --all-features -- -D warnings
ALSA_CONFIG_PATH="$tmper_check_dir/alsa.conf" cargo test --locked --workspace --all-features -- --include-ignored --test-threads=1
cargo build --locked --release
dbus-run-session -- python3 scripts/smoke_test.py target/release/tmper
