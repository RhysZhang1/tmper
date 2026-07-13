use std::path::PathBuf;

/// Returns the project root directory.
///
/// - **Debug builds** (cargo run / cargo test): uses `CARGO_MANIFEST_DIR` at
///   compile time, which always points to the crate root. This ensures config/
///   and data/ resolve correctly during development regardless of the working
///   directory or the executable's location.
///
/// - **Release builds**: resolves relative to the executable binary
///   (`current_exe` → parent → parent), which works for the installed layout
///   where `tmper` lives at `$prefix/bin/tmper`.
pub fn project_root() -> PathBuf {
    #[cfg(debug_assertions)]
    {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    #[cfg(not(debug_assertions))]
    {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

pub fn data_dir() -> PathBuf {
    project_root().join("data")
}

pub fn config_dir() -> PathBuf {
    project_root().join("config")
}
