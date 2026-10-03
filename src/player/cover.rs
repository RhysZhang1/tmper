//! Cover art on disk, for the desktop to fetch.
//!
//! MPRIS asks for a *URL*, not for bytes: the media widget in the panel
//! downloads the picture itself, out of process, and keeps its own copy. So
//! the player writes the image it already read off the tags into the cache
//! directory and hands out the path.
//!
//! The file is named after the **track's path**, not after its bytes. The
//! desktop fetches the URL when the metadata changes, so a name that changed
//! every time the same track came round would be a fresh URL for a picture the
//! widget already has — and a fresh download to go with it.

use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// How many covers stay in the cache.
///
/// The widget only ever asks about the track on the deck, so this is already
/// generous — it is here so that stepping back and forth through a playlist
/// does not rewrite the same two files, while a machine that has played ten
/// thousand tracks keeps ten covers instead of ten thousand.
const KEEP: usize = 8;

/// Write `bytes` as the cached cover of `track`, and return where they landed.
///
/// `None` when there is nothing worth writing: no bytes, a format no widget
/// will draw, or a cache directory that cannot be written. None of those is an
/// error — a missing cache costs a picture in the panel and nothing else.
pub fn store(dir: &Path, track: &Path, bytes: &[u8]) -> Option<PathBuf> {
    let extension = extension(bytes)?;
    // Spelled the way the MPRIS track id spells the same number: one track is
    // one name, and a fixed width reads as an identifier rather than a count.
    let path = dir.join(format!("{:016x}.{extension}", fingerprint(track)));
    // Already there: the same track, the same picture, and no reason to spend
    // a write on it. `prune` is deliberately inside this branch too — the
    // steady state of replaying a track is no filesystem work at all.
    if path.exists() {
        return Some(path);
    }
    if let Err(error) = std::fs::create_dir_all(dir) {
        tracing::warn!("Failed to create the cover cache {:?}: {error}", dir);
        return None;
    }
    if let Err(error) = std::fs::write(&path, bytes) {
        tracing::warn!("Failed to cache cover art in {:?}: {error}", dir);
        return None;
    }
    prune(dir);
    Some(path)
}

/// Delete the cached covers past the newest [`KEEP`].
///
/// Best-effort: a file that will not go is a file that will be considered
/// again next time, and losing an argument with the filesystem is not a reason
/// to fail the write that just succeeded.
fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .collect();
    if files.len() <= KEEP {
        return;
    }
    files.sort_by_key(|(modified, _)| *modified);
    for (_, path) in &files[..files.len() - KEEP] {
        let _ = std::fs::remove_file(path);
    }
}

/// The extension the bytes want, from their magic number.
///
/// The tag reader hands over bytes and no MIME type, and the extension is not
/// cosmetic: a JPEG written as `.png` is a picture Plasma refuses to draw. The
/// two formats here are exactly the ones this program can also *render*
/// (`image`, jpeg+png); anything else keeps its bytes out of the cache rather
/// than being mislabelled.
fn extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else {
        None
    }
}

/// A stable name for a track's path.
///
/// FNV-1a over the path's bytes: not a security hash, just a short name that
/// is the same on every run and different for every track. The MPRIS track id
/// is derived from the same function, so a track is one identity in both
/// places.
pub fn fingerprint(path: &Path) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.as_os_str().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tmper-cover-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];

    #[test]
    fn a_png_lands_with_a_png_name() {
        let dir = dir("png");
        let path = store(&dir, Path::new("/music/a.flac"), PNG).expect("stored");
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("png"));
        assert_eq!(std::fs::read(&path).unwrap(), PNG);
    }

    #[test]
    fn a_jpeg_lands_with_a_jpeg_name() {
        let dir = dir("jpeg");
        let path = store(&dir, Path::new("/music/a.flac"), JPEG).expect("stored");
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("jpg"));
    }

    /// A mislabelled file is worse than no file: the widget would try to
    /// decode it and give up. Bytes in a format this program cannot name are
    /// not cached at all.
    #[test]
    fn bytes_in_an_unknown_format_are_not_cached() {
        let dir = dir("unknown");
        assert_eq!(
            store(&dir, Path::new("/music/a.flac"), b"GIF89a......"),
            None
        );
        assert_eq!(store(&dir, Path::new("/music/a.flac"), &[]), None);
        assert!(!dir.exists(), "nothing was written, not even the directory");
    }

    /// The name is the track's, so replaying a track reuses its file instead
    /// of leaving the last one behind.
    #[test]
    fn the_same_track_reuses_the_same_file() {
        let dir = dir("reuse");
        let first = store(&dir, Path::new("/music/a.flac"), PNG).expect("stored");
        let second = store(&dir, Path::new("/music/a.flac"), PNG).expect("stored");
        assert_eq!(first, second);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn different_tracks_get_different_files() {
        let dir = dir("distinct");
        let a = store(&dir, Path::new("/music/a.flac"), PNG).expect("stored");
        let b = store(&dir, Path::new("/music/b.flac"), PNG).expect("stored");
        assert_ne!(a, b);
    }

    /// The cache is allowed to be a cache: it holds a handful of covers, not
    /// one per track the user has ever played.
    #[test]
    fn the_cache_does_not_grow_with_the_listening_history() {
        let dir = dir("prune");
        let written = KEEP + 5;
        // Written in a tight loop, so every mtime would be "just now" and
        // "newest" would be a coin flip between them. Say when instead.
        let base = std::time::SystemTime::now();
        for index in 0..written {
            let track = PathBuf::from(format!("/music/{index}.flac"));
            let path = store(&dir, &track, PNG).expect("stored");
            let when = base - std::time::Duration::from_secs((written - index) as u64);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .expect("reopen")
                .set_modified(when)
                .expect("stamp");
        }

        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), KEEP);
        // And the ones that stayed are the newest ones: the cover the desktop
        // is about to ask for must not be the one that was thrown away.
        for index in KEEP + 5 - KEEP..written {
            let track = PathBuf::from(format!("/music/{index}.flac"));
            assert!(
                dir.join(format!("{:016x}.png", fingerprint(&track)))
                    .exists(),
                "track {index} is newer than the others and should have survived"
            );
        }
        let oldest = PathBuf::from("/music/0.flac");
        assert!(
            !dir.join(format!("{:016x}.png", fingerprint(&oldest)))
                .exists(),
            "the oldest cover should have gone"
        );
    }

    #[test]
    fn a_fingerprint_is_stable_and_distinct() {
        assert_eq!(
            fingerprint(Path::new("/music/a.flac")),
            fingerprint(Path::new("/music/a.flac"))
        );
        assert_ne!(
            fingerprint(Path::new("/music/a.flac")),
            fingerprint(Path::new("/music/b.flac"))
        );
    }

    /// The path is hashed as bytes, so two files whose names differ only in
    /// how they are spelled are two files.
    #[test]
    fn a_non_utf8_path_still_gets_a_name() {
        let odd = PathBuf::from(std::ffi::OsStr::from_bytes(b"/music/\xff\xfe.flac"));
        assert_ne!(fingerprint(&odd), fingerprint(Path::new("/music//.flac")));
    }
}
