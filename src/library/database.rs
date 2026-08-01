use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
/// Database row — stores ALL columns even though only `path`/`title`/`artist`
/// are consumed in the current UI.  The rest are reserved for future features
/// (sorting by year, filtering by genre/bitrate, etc.).
#[expect(
    dead_code,
    reason = "DB schema — most fields reserved for future queries"
)]
pub struct TrackRow {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_num: Option<u32>,
    pub disc_num: Option<u32>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub duration_s: f64,
    pub bitrate: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub codec: String,
    pub file_size: u64,
    pub file_mtime: i64,
    pub added_at: i64,
}

pub struct LibraryDb {
    conn: Connection,
}

impl LibraryDb {
    pub fn open(path: &Path) -> AppResult<Self> {
        let conn = Connection::open(path)
            .map_err(|e| AppError::Config(format!("Failed to open database: {e}")))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tracks (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                path        TEXT NOT NULL UNIQUE,
                title       TEXT,
                artist      TEXT,
                album       TEXT,
                album_artist TEXT,
                track_num   INTEGER,
                disc_num    INTEGER,
                genre       TEXT,
                year        INTEGER,
                duration_s  REAL,
                bitrate     INTEGER,
                sample_rate INTEGER,
                channels    INTEGER,
                codec       TEXT,
                file_size   INTEGER,
                file_mtime  INTEGER,
                added_at    INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_artist ON tracks(artist);
            CREATE INDEX IF NOT EXISTS idx_album  ON tracks(album);
            CREATE INDEX IF NOT EXISTS idx_genre  ON tracks(genre);
            CREATE INDEX IF NOT EXISTS idx_search ON tracks(title, artist, album);",
        )
        .map_err(|e| AppError::Config(format!("Failed to create schema: {e}")))?;

        Ok(Self { conn })
    }

    pub fn open_memory() -> AppResult<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| AppError::Config(format!("Failed to open in-memory database: {e}")))?;

        conn.execute_batch(
            "CREATE TABLE tracks (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                path        TEXT NOT NULL UNIQUE,
                title       TEXT,
                artist      TEXT,
                album       TEXT,
                album_artist TEXT,
                track_num   INTEGER,
                disc_num    INTEGER,
                genre       TEXT,
                year        INTEGER,
                duration_s  REAL,
                bitrate     INTEGER,
                sample_rate INTEGER,
                channels    INTEGER,
                codec       TEXT,
                file_size   INTEGER,
                file_mtime  INTEGER,
                added_at    INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_search ON tracks(title, artist, album);",
        )
        .map_err(|e| AppError::Config(format!("Failed to create schema: {e}")))?;

        Ok(Self { conn })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert(
        &self,
        path: &str,
        title: &str,
        artist: Option<&str>,
        album: Option<&str>,
        album_artist: Option<&str>,
        track_num: Option<u32>,
        disc_num: Option<u32>,
        genre: Option<&str>,
        year: Option<u32>,
        duration_s: f64,
        bitrate: u32,
        sample_rate: u32,
        channels: u32,
        codec: &str,
        file_size: u64,
        file_mtime: i64,
    ) -> AppResult<i64> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        self.conn
            .execute(
                "INSERT INTO tracks (path, title, artist, album, album_artist,
                 track_num, disc_num, genre, year, duration_s, bitrate,
                 sample_rate, channels, codec, file_size, file_mtime, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                         ?12, ?13, ?14, ?15, ?16, ?17)
                 ON CONFLICT(path) DO UPDATE SET
                    title = excluded.title,
                    artist = excluded.artist,
                    album = excluded.album,
                    album_artist = excluded.album_artist,
                    track_num = excluded.track_num,
                    disc_num = excluded.disc_num,
                    genre = excluded.genre,
                    year = excluded.year,
                    duration_s = excluded.duration_s,
                    bitrate = excluded.bitrate,
                    sample_rate = excluded.sample_rate,
                    channels = excluded.channels,
                    codec = excluded.codec,
                    file_size = excluded.file_size,
                    file_mtime = excluded.file_mtime,
                    added_at = excluded.added_at",
                params![
                    path,
                    title,
                    artist,
                    album,
                    album_artist,
                    track_num,
                    disc_num,
                    genre,
                    year,
                    duration_s,
                    bitrate,
                    sample_rate,
                    channels,
                    codec,
                    file_size as i64,
                    file_mtime,
                    now
                ],
            )
            .map_err(|e| AppError::Config(format!("Failed to upsert track: {e}")))?;

        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_by_path(&self, path: &str) -> AppResult<Option<TrackRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT * FROM tracks WHERE path = ?1")
            .map_err(|e| AppError::Config(format!("Failed to prepare query: {e}")))?;

        let row = stmt
            .query_row(params![path], row_from_db)
            .optional()
            .map_err(|e| AppError::Config(format!("Failed to query: {e}")))?;

        Ok(row)
    }

    pub fn search(&self, query: &str) -> AppResult<Vec<TrackRow>> {
        let pattern = format!("%{query}%");
        let mut stmt = self
            .conn
            .prepare(
                "SELECT * FROM tracks WHERE title LIKE ?1 OR artist LIKE ?1 OR album LIKE ?1
                 ORDER BY artist, album, track_num",
            )
            .map_err(|e| AppError::Config(format!("Failed to prepare search: {e}")))?;

        let rows = stmt
            .query_map(params![pattern], row_from_db)
            .map_err(|e| AppError::Config(format!("Failed to search: {e}")))?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| AppError::Config(format!("Failed to read row: {e}")))?);
        }
        Ok(result)
    }

    pub fn get_artists(&self) -> AppResult<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT artist FROM tracks WHERE artist IS NOT NULL ORDER BY artist")
            .map_err(|e| AppError::Config(format!("Failed to prepare query: {e}")))?;

        let rows = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| AppError::Config(format!("Failed to query artists: {e}")))?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| AppError::Config(format!("Failed to read artist: {e}")))?);
        }
        Ok(result)
    }

    pub fn get_albums_by_artist(&self, artist: &str) -> AppResult<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT DISTINCT album FROM tracks WHERE artist = ?1 AND album IS NOT NULL ORDER BY album",
            )
            .map_err(|e| AppError::Config(format!("Failed to prepare query: {e}")))?;

        let rows = stmt
            .query_map(params![artist], |row| row.get(0))
            .map_err(|e| AppError::Config(format!("Failed to query albums: {e}")))?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| AppError::Config(format!("Failed to read album: {e}")))?);
        }
        Ok(result)
    }

    pub fn get_tracks_by_album(&self, artist: &str, album: &str) -> AppResult<Vec<TrackRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT * FROM tracks WHERE artist = ?1 AND album = ?2 ORDER BY track_num")
            .map_err(|e| AppError::Config(format!("Failed to prepare query: {e}")))?;

        let rows = stmt
            .query_map(params![artist, album], row_from_db)
            .map_err(|e| AppError::Config(format!("Failed to query tracks: {e}")))?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| AppError::Config(format!("Failed to read track: {e}")))?);
        }
        Ok(result)
    }

    #[cfg(test)]
    pub fn delete_by_path(&self, path: &str) -> AppResult<()> {
        self.conn
            .execute("DELETE FROM tracks WHERE path = ?1", params![path])
            .map_err(|e| AppError::Config(format!("Failed to delete: {e}")))?;
        Ok(())
    }

    #[cfg(test)]
    pub fn count(&self) -> AppResult<usize> {
        let count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))
            .map_err(|e| AppError::Config(format!("Failed to count: {e}")))?;
        Ok(count)
    }
}

fn row_from_db(row: &rusqlite::Row) -> rusqlite::Result<TrackRow> {
    Ok(TrackRow {
        id: row.get(0)?,
        path: row.get(1)?,
        title: row.get(2)?,
        artist: row.get(3)?,
        album: row.get(4)?,
        album_artist: row.get(5)?,
        track_num: row.get(6)?,
        disc_num: row.get(7)?,
        genre: row.get(8)?,
        year: row.get(9)?,
        duration_s: row.get(10)?,
        bitrate: row.get::<_, i64>(11)? as u32,
        sample_rate: row.get::<_, i64>(12)? as u32,
        channels: row.get::<_, i64>(13)? as u32,
        codec: row.get(14)?,
        file_size: row.get::<_, i64>(15)? as u64,
        file_mtime: row.get(16)?,
        added_at: row.get(17)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_test_track(db: &LibraryDb, path: &str, title: &str, artist: &str, album: &str) {
        db.upsert(
            path,
            title,
            Some(artist),
            Some(album),
            None,
            Some(1),
            Some(1),
            Some("Rock"),
            Some(2024),
            200.0,
            320,
            44100,
            2,
            "FLAC",
            10000,
            1000,
        )
        .expect("upsert failed");
    }

    #[test]
    fn test_upsert_and_query() {
        let db = LibraryDb::open_memory().unwrap();
        insert_test_track(&db, "/music/a.flac", "Song A", "Artist 1", "Album 1");

        let row = db.get_by_path("/music/a.flac").unwrap().unwrap();
        assert_eq!(row.title, "Song A");
        assert_eq!(row.artist.as_deref(), Some("Artist 1"));
        assert_eq!(row.duration_s, 200.0);
    }

    #[test]
    fn test_upsert_duplicate_path_updates() {
        let db = LibraryDb::open_memory().unwrap();
        insert_test_track(&db, "/music/a.flac", "Song A", "Artist 1", "Album 1");
        // Update same path
        db.upsert(
            "/music/a.flac",
            "Song A Updated",
            Some("Artist 1"),
            None,
            None,
            None,
            None,
            None,
            None,
            250.0,
            256,
            48000,
            2,
            "MP3",
            5000,
            2000,
        )
        .unwrap();

        let row = db.get_by_path("/music/a.flac").unwrap().unwrap();
        assert_eq!(row.title, "Song A Updated");
        assert_eq!(row.duration_s, 250.0);
        assert_eq!(row.sample_rate, 48000);
        assert_eq!(db.count().unwrap(), 1); // No duplicate
    }

    #[test]
    fn test_search_finds_match() {
        let db = LibraryDb::open_memory().unwrap();
        insert_test_track(&db, "/music/a.flac", "Hello World", "John", "Debut");
        insert_test_track(&db, "/music/b.flac", "Goodbye", "Jane", "Farewell");

        let results = db.search("hello").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Hello World");

        let results = db.search("jane").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].artist.as_deref(), Some("Jane"));
    }

    #[test]
    fn test_get_artists() {
        let db = LibraryDb::open_memory().unwrap();
        insert_test_track(&db, "/music/a.flac", "A", "Artist 1", "Album 1");
        insert_test_track(&db, "/music/b.flac", "B", "Artist 2", "Album 2");
        insert_test_track(&db, "/music/c.flac", "C", "Artist 1", "Album 3");

        let artists = db.get_artists().unwrap();
        assert_eq!(artists.len(), 2);
        assert_eq!(artists[0], "Artist 1");
    }

    #[test]
    fn test_get_albums_by_artist() {
        let db = LibraryDb::open_memory().unwrap();
        insert_test_track(&db, "/music/a.flac", "A", "Artist 1", "Album 1");
        insert_test_track(&db, "/music/b.flac", "B", "Artist 1", "Album 2");

        let albums = db.get_albums_by_artist("Artist 1").unwrap();
        assert_eq!(albums.len(), 2);
    }

    #[test]
    fn test_delete() {
        let db = LibraryDb::open_memory().unwrap();
        insert_test_track(&db, "/music/a.flac", "A", "Artist 1", "Album 1");
        assert_eq!(db.count().unwrap(), 1);
        db.delete_by_path("/music/a.flac").unwrap();
        assert_eq!(db.count().unwrap(), 0);
    }
}
