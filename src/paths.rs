use std::path::PathBuf;

const APP_NAME: &str = "tmper";

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn home_fallback(child: &str) -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(child)
        .join(APP_NAME)
}

pub fn config_dir() -> PathBuf {
    env_path("TMPER_CONFIG_DIR")
        .or_else(|| dirs::config_dir().map(|path| path.join(APP_NAME)))
        .unwrap_or_else(|| home_fallback(".config"))
}

pub fn data_dir() -> PathBuf {
    env_path("TMPER_DATA_DIR")
        .or_else(|| dirs::data_dir().map(|path| path.join(APP_NAME)))
        .unwrap_or_else(|| home_fallback(".local/share"))
}

pub fn state_dir() -> PathBuf {
    env_path("TMPER_STATE_DIR")
        .or_else(|| {
            std::env::var_os("XDG_STATE_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|path| path.join(APP_NAME))
        })
        .unwrap_or_else(|| home_fallback(".local/state"))
}

/// Source-tree root used only to migrate pre-XDG installations.
pub fn legacy_project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_paths_have_tmper_suffix() {
        assert_eq!(
            config_dir().file_name().and_then(|name| name.to_str()),
            Some(APP_NAME)
        );
        assert_eq!(
            data_dir().file_name().and_then(|name| name.to_str()),
            Some(APP_NAME)
        );
        assert_eq!(
            state_dir().file_name().and_then(|name| name.to_str()),
            Some(APP_NAME)
        );
    }
}
