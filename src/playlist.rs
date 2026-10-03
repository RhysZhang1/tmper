use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The single playlist data model used across the app — UI, persistence, and
/// M3U I/O. Songs are stored as plain paths; per-track metadata (title,
/// artist, duration) is read on demand from the audio files when needed, so no
/// duplicate metadata copy is kept here.
///
/// The `id` is assigned by the store that owns the playlist (the daemon's
/// [`crate::player::playlists::Playlists`]) and never changes. Names are not
/// identities: the UI has never stopped a user from creating two playlists
/// called "Mix", and an edit addressed by name would silently land on
/// whichever one the daemon found first.
///
/// Re-exported from `ui::views::playlist_view` for UI call sites.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistData {
    /// Assigned by the store. `0` in a file written before ids existed, and in
    /// a `PlaylistData` that is not stored yet — the store numbers those on
    /// load rather than trusting a zero it cannot address.
    #[serde(default)]
    pub id: u64,
    pub name: String,
    pub songs: Vec<PathBuf>,
}

impl PlaylistData {
    /// A playlist with a name and no songs, for a store that will number it.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            id: 0,
            name: name.into(),
            songs: Vec::new(),
        }
    }
}
