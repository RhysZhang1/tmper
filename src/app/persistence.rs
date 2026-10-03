//! What the *client* persists.
//!
//! `state.json` and `library.json` are gone from here — the player owns them
//! now, because volume, repeat mode, the lyric offset and the collection's
//! paths outlive any one TUI. What remains is the playlist store, which the
//! client still owns until the next slice hands it to the daemon.

use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::ipc::proto::Request;

impl App {
    pub(super) fn save_playlists(&mut self) {
        #[derive(Serialize)]
        struct SavePlaylist {
            name: String,
            songs: Vec<String>,
        }
        let path = crate::paths::state_dir().join("playlists.json");
        // Create the parent rather than assuming startup made it: the same
        // guard `save_state` has, so every writer in this file stands alone.
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let save: Vec<SavePlaylist> = self
            .ui_state
            .playlist_state
            .playlists
            .iter()
            .map(|pl| SavePlaylist {
                name: pl.name.clone(),
                songs: pl
                    .songs
                    .iter()
                    .map(|s| s.to_string_lossy().to_string())
                    .collect(),
            })
            .collect();
        if let Ok(json) = serde_json::to_string_pretty(&save) {
            let _ = std::fs::write(&path, json);
        }
        // Every playlist edit lands here, which makes this the one place that
        // has to tell the player what `Next` should walk. Anything that
        // reshapes the store and forgets to call this would leave the player
        // advancing through a list the user has already edited.
        self.sync_active_list();
    }

    pub(super) fn load_playlists(&mut self) {
        #[derive(Deserialize)]
        struct SavePlaylist {
            name: String,
            songs: Vec<String>,
        }
        let path = crate::paths::state_dir().join("playlists.json");
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(save) = serde_json::from_str::<Vec<SavePlaylist>>(&content) {
                for sp in save {
                    let songs: Vec<std::path::PathBuf> = sp
                        .songs
                        .iter()
                        .filter(|s| std::path::Path::new(s).exists())
                        .map(std::path::PathBuf::from)
                        .collect();
                    self.ui_state.playlist_state.playlists.push(
                        crate::ui::views::playlist_view::PlaylistData {
                            name: sp.name,
                            songs,
                        },
                    );
                }
            }
        }
        self.sync_active_list();
    }

    /// Push the songs of the open playlist to the player.
    ///
    /// An index, not the songs, would be the smaller message — and the wrong
    /// one: the player and the client would then share a cursor into a list
    /// only one of them can edit. Sending the songs means the player's idea of
    /// "the list" is a copy taken at a moment the client chose.
    ///
    /// An empty list is meaningful: it means no playlist is open, and the
    /// player falls back to the queue.
    pub(super) fn sync_active_list(&mut self) {
        let songs = self
            .ui_state
            .active_playlist
            .and_then(|index| self.ui_state.playlist_state.playlists.get(index))
            .map(|playlist| playlist.songs.clone())
            .unwrap_or_default();
        self.dispatch(Request::SetActiveList { songs });
    }
}

#[cfg(test)]
mod tests {
    use crate::app::handlers::test_support::test_app;
    use crate::paths;
    use crate::ui::views::playlist_view::PlaylistData;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// The runtime directories are per-process, not per-test, so tests that
    /// write the same file must not run concurrently.
    static FILE_LOCK: Mutex<()> = Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        FILE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write_state_file(name: &str, contents: &str) {
        let dir = paths::state_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), contents).unwrap();
    }

    /// A path that really exists, since both loaders drop entries that do not.
    fn fixture(name: &str) -> PathBuf {
        std::fs::canonicalize(format!("tests/fixtures/{name}")).expect("fixture file")
    }

    // ── playlists.json ──

    #[test]
    fn playlists_round_trip_and_drop_songs_that_no_longer_exist() {
        let _guard = lock();
        let song = fixture("test.flac");
        let mut app = test_app();
        app.ui_state.playlist_state.playlists = vec![PlaylistData {
            name: "Keep".into(),
            songs: vec![song.clone(), PathBuf::from("/definitely/missing.flac")],
        }];
        app.save_playlists();

        let mut reloaded = test_app();
        reloaded.load_playlists();

        let loaded = &reloaded.ui_state.playlist_state.playlists;
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "Keep");
        assert_eq!(
            loaded[0].songs,
            vec![song],
            "a song that no longer exists must not be restored"
        );
    }

    #[test]
    fn a_corrupt_playlists_file_is_ignored() {
        let _guard = lock();
        write_state_file("playlists.json", "[[[not json");

        let mut app = test_app();
        app.load_playlists();

        assert!(app.ui_state.playlist_state.playlists.is_empty());
    }

    #[test]
    fn a_missing_playlists_file_is_a_no_op() {
        let _guard = lock();
        let _ = std::fs::remove_file(paths::state_dir().join("playlists.json"));

        let mut app = test_app();
        app.load_playlists();

        assert!(app.ui_state.playlist_state.playlists.is_empty());
    }

    // ── the active list ──

    /// The player is told which songs to walk, not which playlist index is
    /// open: an index means nothing on the other side of the socket.
    #[test]
    fn the_open_playlist_is_pushed_to_the_player() {
        let _guard = lock();
        let song = fixture("test.flac");
        let mut app = test_app();
        app.ui_state.playlist_state.playlists = vec![PlaylistData {
            name: "Mix".into(),
            songs: vec![song.clone()],
        }];
        app.ui_state.active_playlist = Some(0);
        app.save_playlists();

        assert_eq!(app.player().active_list(), vec![song]);
    }

    /// Closing the playlist must *clear* the list on the player, or `Next`
    /// would keep walking a list the user has left.
    #[test]
    fn closing_the_playlist_clears_it_on_the_player() {
        let _guard = lock();
        let song = fixture("test.flac");
        let mut app = test_app();
        app.ui_state.playlist_state.playlists = vec![PlaylistData {
            name: "Mix".into(),
            songs: vec![song],
        }];
        app.ui_state.active_playlist = Some(0);
        app.save_playlists();
        assert_eq!(app.player().active_list().len(), 1);

        app.ui_state.active_playlist = None;
        app.save_playlists();
        assert!(app.player().active_list().is_empty());
    }
}
