//! The playlist store.
//!
//! `playlists.json` and the playlists in it live on this side of the socket
//! for the same reason the index does: a playlist is the list `Next` walks, so
//! it has to outlive the window that opened it, and two TUIs must not be two
//! writers of one file.
//!
//! Every edit is addressed by **id**, not by name or by position. A name is
//! not unique (the UI has always allowed two playlists called "Mix"), and a
//! position is a cursor into whichever copy of the store the caller happens to
//! hold — which is exactly the copy another client may have just edited.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::library::playlist_manager::{export_m3u, import_m3u};
use crate::playlist::PlaylistData;

pub struct Playlists {
    entries: Vec<PlaylistData>,
    /// Where the store is written. `None` for a store that never touches disk:
    /// every test, and any run whose state directory is unusable.
    file: Option<PathBuf>,
    /// The next id to hand out. Ids are never reused — a client holding a
    /// reference to a deleted playlist must not find it pointing at a new one.
    next_id: u64,
}

impl Playlists {
    /// The store of a real run: `playlists.json` in the state directory.
    pub fn open() -> Self {
        let mut store = Self::with_file(Some(crate::paths::state_dir().join("playlists.json")));
        store.load();
        store
    }

    /// A store that never touches disk, so no two tests share one.
    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::with_file(None)
    }

    fn with_file(file: Option<PathBuf>) -> Self {
        Self {
            entries: Vec::new(),
            file,
            next_id: 1,
        }
    }

    /// A store holding these playlists outright, numbered like a load would
    /// number them. For tests whose keypresses have to name an id.
    #[cfg(test)]
    pub fn with_entries(entries: Vec<PlaylistData>) -> Self {
        let mut store = Self::with_file(None);
        store.entries = entries;
        number(&mut store.entries, &mut store.next_id);
        store
    }

    pub fn entries(&self) -> &[PlaylistData] {
        &self.entries
    }

    pub fn find(&self, id: u64) -> Option<&PlaylistData> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// The songs of one playlist, or `None` if there is no such playlist.
    ///
    /// `Some(&[])` and `None` are different answers: an empty playlist is one
    /// the user opened and has not filled, and walking it means walking
    /// nothing.
    pub fn songs_of(&self, id: u64) -> Option<&[PathBuf]> {
        self.find(id).map(|entry| entry.songs.as_slice())
    }

    /// A new, empty playlist. Returns its id.
    pub fn create(&mut self, name: impl Into<String>) -> u64 {
        let mut playlist = PlaylistData::named(name);
        let id = self.take_id();
        playlist.id = id;
        self.entries.push(playlist);
        self.save();
        id
    }

    /// Append a song. Returns whether the playlist changed — a song it already
    /// has is not added twice, which is what keeps `Next` from walking the
    /// same track three times because the user pressed `Enter` three times.
    pub fn add_song(&mut self, id: u64, path: &Path) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) else {
            return false;
        };
        if entry.songs.iter().any(|song| song == path) {
            return false;
        }
        entry.songs.push(path.to_path_buf());
        self.save();
        true
    }

    /// Drop one entry by position within the playlist.
    pub fn remove_song(&mut self, id: u64, index: usize) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) else {
            return false;
        };
        if index >= entry.songs.len() {
            return false;
        }
        entry.songs.remove(index);
        self.save();
        true
    }

    /// Drop a whole playlist. Returns whether there was one.
    pub fn delete(&mut self, id: u64) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        if self.entries.len() == before {
            return false;
        }
        self.save();
        true
    }

    /// Read an M3U file into the store as a new playlist. Returns it.
    pub fn import(&mut self, path: &Path) -> crate::error::AppResult<PlaylistData> {
        let mut playlist = import_m3u(path)?;
        playlist.id = self.take_id();
        // Only paths that exist: a playlist is a list of things to play, and
        // an entry that cannot be played is a row that does nothing but fail
        // when the user picks it. The loader applies the same rule.
        playlist.songs.retain(|song| song.exists());
        let imported = playlist.clone();
        self.entries.push(playlist);
        self.save();
        Ok(imported)
    }

    /// Write one playlist — or every one, when `id` is `None` — to
    /// `{name}.m3u` in the data directory.
    ///
    /// Returns the files actually written, in store order, plus the number
    /// that could not be. The caller composes whatever sentence it likes from
    /// that; a count is a fact, and "2 of 3" is two facts.
    pub fn export(&self, id: Option<u64>) -> (Vec<PathBuf>, usize) {
        let selected: Vec<&PlaylistData> = match id {
            Some(id) => self.entries.iter().filter(|entry| entry.id == id).collect(),
            None => self.entries.iter().collect(),
        };
        let mut written = Vec::new();
        let mut failed = 0;
        for playlist in selected {
            let path = crate::paths::data_dir().join(format!("{}.m3u", playlist.name));
            match export_m3u(playlist, &path) {
                Ok(()) => written.push(path),
                Err(error) => {
                    tracing::warn!("Failed to export {:?}: {error}", path);
                    failed += 1;
                }
            }
        }
        (written, failed)
    }

    // ── Disk ──

    fn load(&mut self) {
        let Some(file) = self.file.clone() else {
            return;
        };
        let Ok(content) = std::fs::read_to_string(&file) else {
            return;
        };
        match serde_json::from_str::<Vec<PlaylistData>>(&content) {
            Ok(mut entries) => {
                let renumbered = number(&mut entries, &mut self.next_id);
                for entry in &mut entries {
                    entry.songs.retain(|song| song.exists());
                }
                self.entries = entries;
                // A file written before ids existed comes back numbered, and
                // writing it now makes the migration one-way instead of
                // something every load has to redo.
                if renumbered {
                    self.save();
                }
            }
            Err(error) => tracing::warn!("Ignoring {}: {error}", file.display()),
        }
    }

    fn save(&self) {
        let Some(file) = self.file.as_ref() else {
            return;
        };
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&self.entries) {
            Ok(json) => {
                if let Err(error) = std::fs::write(file, json) {
                    tracing::warn!("Failed to write {}: {error}", file.display());
                }
            }
            Err(error) => tracing::warn!("Failed to encode the playlists: {error}"),
        }
    }

    /// A fresh id, and the counter past it.
    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

/// Give every entry an id it can be addressed by, bumping `next_id` past them
/// all. Returns whether anything had to be renumbered.
///
/// Two ways in: a file written before ids existed (every entry zero), and a
/// hand-edited one that repeats a number. Both would otherwise leave an edit
/// landing on the wrong playlist — the exact failure the id exists to rule
/// out — so a duplicate is renumbered rather than rejected.
fn number(entries: &mut [PlaylistData], next_id: &mut u64) -> bool {
    let mut used: HashSet<u64> = HashSet::new();
    let mut next = 1u64;
    let mut renumbered = false;
    for entry in entries.iter_mut() {
        if entry.id == 0 || !used.insert(entry.id) {
            while used.contains(&next) {
                next += 1;
            }
            entry.id = next;
            used.insert(next);
            renumbered = true;
        }
        next = next.max(entry.id + 1);
    }
    *next_id = next;
    renumbered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tmper-playlists-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{name}.json"))
    }

    /// A path that really exists, since the loader drops the ones that do not.
    fn fixture(name: &str) -> PathBuf {
        std::fs::canonicalize(format!("tests/fixtures/{name}")).expect("fixture file")
    }

    #[test]
    fn a_playlist_round_trips_with_its_id() {
        let store = file("round-trip");
        let _ = std::fs::remove_file(&store);
        let mut playlists = Playlists::with_file(Some(store.clone()));
        let song = fixture("test.flac");
        let id = playlists.create("Mix");
        assert!(playlists.add_song(id, &song));

        let mut reloaded = Playlists::with_file(Some(store));
        reloaded.load();
        assert_eq!(reloaded.entries().len(), 1);
        assert_eq!(reloaded.entries()[0].name, "Mix");
        assert_eq!(reloaded.entries()[0].id, id);
        assert_eq!(reloaded.songs_of(id), Some([song].as_slice()));
    }

    /// A song that has left the disk is dropped on load: the row would only
    /// fail when picked, and the same rule already applies to library paths.
    #[test]
    fn songs_that_no_longer_exist_are_dropped_on_load() {
        let store = file("missing-song");
        let song = fixture("test.flac");
        let mut playlists = Playlists::with_file(Some(store.clone()));
        let id = playlists.create("Keep");
        playlists.add_song(id, &song);
        playlists.add_song(id, Path::new("/definitely/missing.flac"));

        let mut reloaded = Playlists::with_file(Some(store));
        reloaded.load();
        assert_eq!(reloaded.songs_of(id), Some([song].as_slice()));
    }

    /// A file from before ids existed has zeros throughout; every entry has to
    /// come back addressable, and the numbering has to be written down.
    #[test]
    fn a_file_without_ids_is_numbered_and_rewritten() {
        let store = file("legacy");
        std::fs::write(
            &store,
            r#"[{"name":"One","songs":[]},{"name":"Two","songs":[]}]"#,
        )
        .unwrap();

        let mut playlists = Playlists::with_file(Some(store.clone()));
        playlists.load();
        let ids: Vec<u64> = playlists.entries().iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![1, 2]);

        let written: Vec<PlaylistData> =
            serde_json::from_str(&std::fs::read_to_string(&store).unwrap()).unwrap();
        assert_eq!(
            written.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![1, 2],
            "the renumbering has to reach the disk, or the next load redoes it"
        );

        // And a new playlist must not collide with the ones that came back.
        let mut playlists = Playlists::with_file(Some(store));
        playlists.load();
        let fresh = playlists.create("Three");
        assert_eq!(fresh, 3);
    }

    #[test]
    fn a_duplicate_id_is_renumbered() {
        let mut entries = vec![
            PlaylistData {
                id: 7,
                name: "One".into(),
                songs: Vec::new(),
            },
            PlaylistData {
                id: 7,
                name: "Two".into(),
                songs: Vec::new(),
            },
        ];
        let mut next = 1;
        assert!(number(&mut entries, &mut next));
        assert_ne!(entries[0].id, entries[1].id);
        assert_eq!(next, entries.iter().map(|e| e.id).max().unwrap() + 1);
    }

    #[test]
    fn adding_the_same_song_twice_keeps_one() {
        let mut playlists = Playlists::in_memory();
        let song = fixture("test.flac");
        let id = playlists.create("Mix");
        assert!(playlists.add_song(id, &song));
        assert!(!playlists.add_song(id, &song));
        assert_eq!(playlists.songs_of(id), Some([song].as_slice()));
    }

    #[test]
    fn removing_a_song_removes_it_by_position() {
        let mut playlists = Playlists::in_memory();
        let a = fixture("test.flac");
        let b = fixture("test.wav");
        let id = playlists.create("Mix");
        playlists.add_song(id, &a);
        playlists.add_song(id, &b);
        assert!(playlists.remove_song(id, 0));
        assert_eq!(playlists.songs_of(id), Some([b].as_slice()));
        assert!(!playlists.remove_song(id, 5), "past the end is not an edit");
    }

    /// Every edit names a playlist by id, so an edit to one that is gone has
    /// to be a no-op rather than an edit to whatever is in its place.
    #[test]
    fn an_edit_to_a_missing_playlist_changes_nothing() {
        let mut playlists = Playlists::in_memory();
        let id = playlists.create("Mix");
        assert!(playlists.delete(id));
        assert!(!playlists.delete(id));
        assert!(!playlists.add_song(id, &fixture("test.flac")));
        assert!(!playlists.remove_song(id, 0));
        assert!(playlists.entries().is_empty());
    }

    /// The store is the one that stops a re-used id: a client holding a
    /// reference to a deleted playlist must not find it pointing at a new one.
    #[test]
    fn ids_are_never_reused() {
        let mut playlists = Playlists::in_memory();
        let first = playlists.create("One");
        playlists.delete(first);
        let second = playlists.create("Two");
        assert_ne!(first, second);
        assert!(playlists.find(first).is_none());
    }

    #[test]
    fn an_import_lands_as_a_numbered_playlist() {
        let dir = std::env::temp_dir().join(format!("tmper-m3u-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("Imported.m3u");
        let song = fixture("test.flac");
        std::fs::write(
            &source,
            format!("#EXTM3U\n{}\n/missing.flac\n", song.display()),
        )
        .unwrap();

        let mut playlists = Playlists::in_memory();
        let imported = playlists.import(&source).expect("import");
        assert_eq!(imported.name, "Imported");
        assert_ne!(imported.id, 0);
        assert_eq!(
            playlists.songs_of(imported.id),
            Some([song].as_slice()),
            "a path that is not on disk does not come in from an M3U either"
        );

        let _ = std::fs::remove_file(&source);
    }

    #[test]
    fn an_import_that_cannot_be_read_reports_it() {
        let mut playlists = Playlists::in_memory();
        assert!(playlists
            .import(Path::new("/definitely/not/here.m3u"))
            .is_err());
        assert!(playlists.entries().is_empty());
    }

    #[test]
    fn a_playlist_that_names_no_songs_is_still_a_playlist() {
        let mut playlists = Playlists::in_memory();
        let id = playlists.create("Empty");
        assert_eq!(playlists.songs_of(id), Some([].as_slice()));
        assert_eq!(playlists.songs_of(id + 99), None);
    }

    #[test]
    fn a_corrupt_file_is_ignored() {
        let store = file("corrupt");
        std::fs::write(&store, "[[[not json").unwrap();
        let mut playlists = Playlists::with_file(Some(store));
        playlists.load();
        assert!(playlists.entries().is_empty());
    }
}
