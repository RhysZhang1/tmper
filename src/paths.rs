use std::path::PathBuf;

const APP_NAME: &str = "tmper";

// Only the non-test branches below resolve real XDG locations; under
// `cfg(test)` every runtime dir is redirected to `test_root()`.
#[cfg(not(test))]
fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(not(test))]
fn home_fallback(child: &str) -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(child)
        .join(APP_NAME)
}

/// Append the application directory name to an XDG base directory.
///
/// Kept separate from the `#[cfg(test)]` redirect below so the invariant
/// ("every runtime dir is namespaced under `tmper`") stays directly testable
/// even though tests resolve to a temp root.
fn with_app_name(base: PathBuf) -> PathBuf {
    base.join(APP_NAME)
}

/// Config directory: `$TMPER_CONFIG_DIR`, else `$XDG_CONFIG_HOME/tmper`
/// (usually `~/.config/tmper`).
pub fn config_dir() -> PathBuf {
    #[cfg(test)]
    {
        test_root().join("config")
    }
    #[cfg(not(test))]
    {
        env_path("TMPER_CONFIG_DIR")
            .or_else(|| dirs::config_dir().map(with_app_name))
            .unwrap_or_else(|| home_fallback(".config"))
    }
}

/// Data directory: `$TMPER_DATA_DIR`, else `$XDG_DATA_HOME/tmper`
/// (usually `~/.local/share/tmper`). Holds `library.db`.
pub fn data_dir() -> PathBuf {
    #[cfg(test)]
    {
        test_root().join("data")
    }
    #[cfg(not(test))]
    {
        env_path("TMPER_DATA_DIR")
            .or_else(|| dirs::data_dir().map(with_app_name))
            .unwrap_or_else(|| home_fallback(".local/share"))
    }
}

/// State directory: `$TMPER_STATE_DIR`, else `$XDG_STATE_HOME/tmper`
/// (usually `~/.local/state/tmper`). Holds `state.json`, `playlists.json`,
/// `library.json` and the log.
pub fn state_dir() -> PathBuf {
    #[cfg(test)]
    {
        test_root().join("state")
    }
    #[cfg(not(test))]
    {
        env_path("TMPER_STATE_DIR")
            .or_else(|| {
                std::env::var_os("XDG_STATE_HOME")
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
                    .map(with_app_name)
            })
            .unwrap_or_else(|| home_fallback(".local/state"))
    }
}

/// Source-tree root used only to migrate pre-XDG installations.
///
/// Debug builds resolve to the checkout (`cargo run` / `cargo test`). Release
/// builds walk up from the executable instead: `env!("CARGO_MANIFEST_DIR")` is
/// a compile-time constant pointing at the *build* machine, so an installed
/// binary would silently skip migration. The walk matches the old portable
/// layout, where `config/` and `themes/` sat next to the binary.
pub fn legacy_project_root() -> PathBuf {
    #[cfg(debug_assertions)]
    {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
        PathBuf::from(".")
    }
}

/// Copy old project-local runtime files into XDG locations once. Old files
/// are intentionally retained so migration is reversible.
pub fn migrate_legacy_layout() {
    let legacy = legacy_project_root();
    let mappings = [
        (
            legacy.join("config/config.toml"),
            config_dir().join("config.toml"),
        ),
        (
            legacy.join("config/keybindings.toml"),
            config_dir().join("keybindings.toml"),
        ),
        (
            legacy.join("data/library.db"),
            data_dir().join("library.db"),
        ),
        (
            legacy.join("data/state.json"),
            state_dir().join("state.json"),
        ),
        (
            legacy.join("data/playlists.json"),
            state_dir().join("playlists.json"),
        ),
        (
            legacy.join("data/library.json"),
            state_dir().join("library.json"),
        ),
    ];

    for (old, new) in mappings {
        if old.exists() && !new.exists() {
            if let Some(parent) = new.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::copy(&old, &new) {
                Ok(_) => tracing::info!("Migrated {:?} to {:?}", old, new),
                Err(error) => tracing::warn!("Failed to migrate {:?}: {error}", old),
            }
        }
    }

    migrate_legacy_themes(&legacy);
}

/// User themes lived in `<project>/themes/`. They are not part of the file
/// mapping above because the directory has to be walked, and skipping them
/// would silently drop custom palettes back to the embedded defaults.
fn migrate_legacy_themes(legacy: &std::path::Path) {
    let source = legacy.join("themes");
    let target = config_dir().join("themes");
    let Ok(entries) = std::fs::read_dir(&source) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        let destination = target.join(name);
        if destination.exists() {
            continue;
        }
        if let Err(error) = std::fs::create_dir_all(&target) {
            tracing::warn!("Failed to create {:?}: {error}", target);
            return;
        }
        match std::fs::copy(&path, &destination) {
            Ok(_) => tracing::info!("Migrated theme {:?} to {:?}", path, destination),
            Err(error) => tracing::warn!("Failed to migrate theme {:?}: {error}", path),
        }
    }
}

/// Test-only root for runtime data and config. Handler tests exercise
/// `save_playlists` / `save_library_paths` / `write_config` / M3U export,
/// which would otherwise clobber the developer's real XDG directories
/// (`~/.config/tmper`, `~/.local/share/tmper`, `~/.local/state/tmper`).
/// Redirecting to a per-process temp dir keeps the suite side-effect-free.
///
/// All three directories must be covered: `state_dir()` took over
/// `state.json` / `playlists.json` / `library.json` in the XDG move, so
/// isolating only config+data would still let handler tests write to the
/// developer's real home.
#[cfg(test)]
fn test_root() -> PathBuf {
    use std::sync::OnceLock;
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| std::env::temp_dir().join(format!("tmper-tests-{}", std::process::id())))
        .clone()
}

/// The lock that serialises the tests which use the shared `config.toml`.
///
/// Every runtime dir lands under one [`test_root`] for the whole test
/// *process*, so every test that cycles a setting — each cycle persists through
/// `write_config` — writes the same file, and the one test that reads it back
/// to prove the write happened can observe another test's config instead of its
/// own. That is not hypothetical: `command_theme_switches_and_loads_the_palette`
/// (`:theme nord`) landing between the press and the read turned
/// `test_enter_on_theme_cycles_and_persists` red on a coverage run.
///
/// The config file is the shared resource, so this is its lock: writers take it
/// for the duration of the test, and the reader holds it across both the press
/// and the read. Poisoning is ignored — a panicking test has already failed,
/// and the rest of the suite should not be dragged down with it.
#[cfg(test)]
pub fn config_file_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_paths_append_app_name() {
        assert_eq!(
            with_app_name(PathBuf::from("/base")),
            PathBuf::from("/base").join(APP_NAME)
        );
    }

    /// The suite must never touch the developer's real XDG directories.
    #[test]
    fn runtime_dirs_are_isolated_under_test() {
        let root = test_root();
        assert_eq!(config_dir(), root.join("config"));
        assert_eq!(data_dir(), root.join("data"));
        assert_eq!(state_dir(), root.join("state"));
    }
}
