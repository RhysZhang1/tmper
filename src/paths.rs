use std::path::PathBuf;

pub fn project_root() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn data_dir() -> PathBuf {
    project_root().join("data")
}

pub fn config_dir() -> PathBuf {
    project_root().join("config")
}
