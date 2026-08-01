use std::path::PathBuf;

/// The single playlist data model used across the app — UI, persistence, and
/// M3U I/O. Songs are stored as plain paths; per-track metadata (title,
/// artist, duration) is read on demand from the audio files when needed, so no
/// duplicate metadata copy is kept here.
///
/// Re-exported from `ui::views::playlist_view` for UI call sites.
#[derive(Debug, Clone)]
pub struct PlaylistData {
    pub name: String,
    pub songs: Vec<PathBuf>,
}
