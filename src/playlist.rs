use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct TrackEntry {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub duration_secs: f64,
}

impl TrackEntry {
    pub fn new(path: PathBuf, title: String, artist: String, duration_secs: f64) -> Self {
        Self {
            path,
            title,
            artist,
            duration_secs,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Playlist {
    pub name: String,
    pub tracks: Vec<TrackEntry>,
    pub current_index: Option<usize>,
}

impl Playlist {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            tracks: Vec::new(),
            current_index: None,
        }
    }

    pub fn push(&mut self, entry: TrackEntry) {
        self.tracks.push(entry);
        if self.current_index.is_none() && !self.tracks.is_empty() {
            self.current_index = Some(0);
        }
    }
}

#[cfg(test)]
impl Playlist {
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn remove(&mut self, index: usize) {
        if index >= self.tracks.len() {
            return;
        }
        self.tracks.remove(index);
        match self.current_index {
            Some(cur) if cur == index => {
                if self.tracks.is_empty() {
                    self.current_index = None;
                } else if cur >= self.tracks.len() {
                    self.current_index = Some(self.tracks.len().saturating_sub(1));
                }
            }
            Some(cur) if cur > index => {
                self.current_index = Some(cur - 1);
            }
            Some(_) => {}
            None => {}
        }
    }

    pub fn next(&mut self) -> Option<usize> {
        match self.current_index {
            Some(idx) if idx + 1 < self.tracks.len() => {
                self.current_index = Some(idx + 1);
                self.current_index
            }
            _ => None,
        }
    }

    pub fn prev(&mut self) -> Option<usize> {
        match self.current_index {
            Some(idx) if idx > 0 => {
                self.current_index = Some(idx - 1);
                self.current_index
            }
            _ => None,
        }
    }

    pub fn shuffle(&mut self) {
        use rand::seq::SliceRandom;
        use rand::thread_rng;

        if self.tracks.len() < 2 {
            return;
        }

        if let Some(cur) = self.current_index {
            let current = self.tracks.remove(cur);
            let mut rng = thread_rng();
            self.tracks.shuffle(&mut rng);
            self.tracks.insert(0, current);
            self.current_index = Some(0);
        } else {
            let mut rng = thread_rng();
            self.tracks.shuffle(&mut rng);
        }
    }

    pub fn insert_after_current(&mut self, entry: TrackEntry) {
        match self.current_index {
            Some(idx) if idx < self.tracks.len() => {
                let insert_pos = idx + 1;
                self.tracks.insert(insert_pos, entry);
            }
            _ => {
                self.tracks.push(entry);
                if self.current_index.is_none() {
                    self.current_index = Some(0);
                }
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(name: &str) -> TrackEntry {
        TrackEntry::new(
            PathBuf::from(format!("/music/{name}.mp3")),
            name.to_string(),
            "Artist".to_string(),
            200.0,
        )
    }

    #[test]
    fn test_push_and_len() {
        let mut pl = Playlist::new("test");
        assert!(pl.is_empty());
        pl.push(make_entry("A"));
        assert_eq!(pl.len(), 1);
        assert_eq!(pl.current_index, Some(0));
    }

    #[test]
    fn test_next_prev() {
        let mut pl = Playlist::new("test");
        pl.push(make_entry("A"));
        pl.push(make_entry("B"));
        pl.push(make_entry("C"));

        assert_eq!(pl.current_index, Some(0));
        assert_eq!(pl.next(), Some(1));
        assert_eq!(pl.next(), Some(2));
        assert_eq!(pl.next(), None);
        assert_eq!(pl.prev(), Some(1));
        assert_eq!(pl.prev(), Some(0));
        assert_eq!(pl.prev(), None);
    }

    #[test]
    fn test_remove_adjusts_index() {
        let mut pl = Playlist::new("test");
        pl.push(make_entry("A"));
        pl.push(make_entry("B"));
        pl.push(make_entry("C"));
        pl.next(); // index = 1

        pl.remove(0); // remove A, B becomes index 0
        assert_eq!(pl.len(), 2);
        assert_eq!(pl.current_index, Some(0));
        assert_eq!(pl.tracks[0].title, "B");
    }

    #[test]
    fn test_remove_empty() {
        let mut pl = Playlist::new("test");
        pl.push(make_entry("A"));
        pl.remove(0);
        assert!(pl.is_empty());
        assert_eq!(pl.current_index, None);
    }

    #[test]
    fn test_shuffle_preserves_current() {
        let mut pl = Playlist::new("test");
        for name in &["A", "B", "C", "D", "E"] {
            pl.push(make_entry(name));
        }
        pl.next(); // index = 1 (B)
        pl.next(); // index = 2 (C)

        pl.shuffle();
        assert_eq!(pl.current_index, Some(0));
        assert_eq!(pl.tracks[0].title, "C");
        assert_eq!(pl.len(), 5);
    }

    #[test]
    fn test_insert_after_current() {
        let mut pl = Playlist::new("test");
        pl.push(make_entry("A"));
        pl.push(make_entry("C"));
        // current_index = 0 (A)

        pl.insert_after_current(make_entry("B"));
        assert_eq!(pl.len(), 3);
        assert_eq!(pl.tracks[1].title, "B");
    }

    #[test]
    fn test_empty_list_boundary() {
        let mut pl: Playlist = Playlist::new("empty");
        assert!(pl.is_empty());
        assert_eq!(pl.next(), None);
        assert_eq!(pl.prev(), None);
        pl.remove(0); // should not panic
        pl.shuffle(); // should not panic
    }
}
