use std::path::PathBuf;

/// Returns the project root directory.
///
/// - **Debug builds** (cargo run / cargo test): uses `CARGO_MANIFEST_DIR` at
///   compile time, which always points to the crate root. This ensures config/
///   data/ and themes/ resolve correctly during development regardless of the
///   working directory or the executable's location.
///
/// - **Release builds**: walks up from the executable looking for the project
///   root — the first ancestor directory containing both `themes/` and
///   `config/`. This handles both the in-tree layout (`target/release/tmper`
///   → repo root) and an installed layout (`$prefix/bin/tmper` → `$prefix`).
///   Falls back to the old `parent().parent()` heuristic if no match is found.
pub fn project_root() -> PathBuf {
    #[cfg(debug_assertions)]
    {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    #[cfg(not(debug_assertions))]
    {
        let mut dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));
        while let Some(d) = dir {
            if d.join("themes").is_dir() && d.join("config").is_dir() {
                return d;
            }
            dir = d.parent().map(|p| p.to_path_buf());
        }
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

pub fn data_dir() -> PathBuf {
    #[cfg(test)]
    {
        test_root().join("data")
    }
    #[cfg(not(test))]
    {
        project_root().join("data")
    }
}

pub fn config_dir() -> PathBuf {
    #[cfg(test)]
    {
        test_root().join("config")
    }
    #[cfg(not(test))]
    {
        project_root().join("config")
    }
}

/// Test-only root for runtime data and config. Handler tests exercise
/// `save_playlists` / `save_library_paths` / `write_config` / M3U export,
/// which would otherwise clobber the developer's real `data/` and
/// `config/config.toml`. Redirecting to a per-process temp dir keeps the
/// suite side-effect-free. `project_root()` is intentionally NOT overridden —
/// theme loading and fixtures still resolve against the real tree.
#[cfg(test)]
fn test_root() -> std::path::PathBuf {
    use std::sync::OnceLock;
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| std::env::temp_dir().join(format!("tmper-tests-{}", std::process::id())))
        .clone()
}
