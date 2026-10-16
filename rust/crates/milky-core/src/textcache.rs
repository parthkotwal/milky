//! The cache of text that is expensive to extract (DECISIONS 2026-10-16,
//! document search, beat D1).
//!
//! `content.sqlite3` in Milky's data folder holds the text of PDFs and Word
//! documents (and, later, OCR), so the index can be rebuilt at startup without
//! running PDFKit again. It is a cache, not history: deleting it costs one
//! re-extraction. Plain text and code are never cached; reading them again
//! takes microseconds.
//!
//! An entry is valid only while the file's modified date and size are what
//! they were when it was extracted. A file with no text (a scanned PDF) is
//! cached as such, so it is not retried until it changes.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::extract::{TextKind, extract, text_kind};
use crate::store::StoreError;

/// File name of the cache inside the data folder.
pub const FILE_NAME: &str = "content.sqlite3";

/// Bump when extraction changes in a way that should redo cached entries.
pub const EXTRACTOR_VERSION: i64 = 1;

/// What identifies one version of a file: when it last changed, and its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub modified_ms: i64,
    pub size: i64,
}

impl Stamp {
    /// The stamp of the file at `path` now, or `None` if it cannot be read.
    pub fn of(path: &Path) -> Option<Stamp> {
        let metadata = std::fs::metadata(path).ok()?;
        let modified_ms = metadata
            .modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as i64);
        Some(Stamp { modified_ms, size: metadata.len() as i64 })
    }
}

#[derive(Debug)]
pub struct TextCache {
    conn: Connection,
}

impl TextCache {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::prepare(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS texts (
                 path TEXT PRIMARY KEY,
                 modified_ms INTEGER NOT NULL,
                 size INTEGER NOT NULL,
                 extractor TEXT NOT NULL,
                 extractor_version INTEGER NOT NULL,
                 extracted_at_ms INTEGER NOT NULL,
                 text TEXT
             );",
        )?;
        Ok(Self { conn })
    }

    /// The cached text of `path` as of `stamp`: `Some(Some(text))`,
    /// `Some(None)` for a file known to have no text, `None` if nothing valid
    /// is cached (never extracted, changed since, or older extractor).
    pub fn get(&self, path: &Path, stamp: Stamp) -> Result<Option<Option<String>>, StoreError> {
        let Some(key) = path.to_str() else {
            return Ok(None);
        };
        let row: Option<(i64, i64, i64, Option<String>)> = self
            .conn
            .query_row(
                "SELECT modified_ms, size, extractor_version, text FROM texts WHERE path = ?1",
                params![key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        Ok(row.and_then(|(modified_ms, size, version, text)| {
            (modified_ms == stamp.modified_ms && size == stamp.size && version == EXTRACTOR_VERSION).then_some(text)
        }))
    }

    /// Remember `text` (or that there is none) for `path` as of `stamp`.
    pub fn put(&mut self, path: &Path, stamp: Stamp, kind: TextKind, text: Option<&str>) -> Result<(), StoreError> {
        let Some(key) = path.to_str() else {
            return Ok(());
        };
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis() as i64);
        self.conn.execute(
            "INSERT INTO texts (path, modified_ms, size, extractor, extractor_version, extracted_at_ms, text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (path) DO UPDATE SET modified_ms = excluded.modified_ms, size = excluded.size,
                 extractor = excluded.extractor, extractor_version = excluded.extractor_version,
                 extracted_at_ms = excluded.extracted_at_ms, text = excluded.text",
            params![key, stamp.modified_ms, stamp.size, kind.name(), EXTRACTOR_VERSION, now, text],
        )?;
        Ok(())
    }

    /// Forget `path` (it was deleted or excluded).
    pub fn remove(&mut self, path: &Path) -> Result<(), StoreError> {
        if let Some(key) = path.to_str() {
            self.conn.execute("DELETE FROM texts WHERE path = ?1", params![key])?;
        }
        Ok(())
    }

    /// How many entries, and how many bytes of text, the cache holds.
    pub fn size(&self) -> Result<(u64, u64), StoreError> {
        let (entries, bytes): (i64, i64) = self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(text)), 0) FROM texts",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok((entries as u64, bytes as u64))
    }
}

/// The indexable text of the file at `path`, or `None` if it has none or its
/// contents are not indexed. Expensive kinds come from `cache` when the file
/// is unchanged, and are extracted and cached otherwise; plain files are read
/// directly. Caching errors never hide text: they only cost a later
/// re-extraction.
pub fn document_text(path: &Path, cache: &mut TextCache) -> Option<String> {
    let kind = text_kind(path)?;
    if !kind.is_expensive() {
        return extract(path, kind);
    }
    let stamp = Stamp::of(path)?;
    if let Ok(Some(cached)) = cache.get(path, stamp) {
        return cached;
    }
    let text = extract(path, kind);
    let _ = cache.put(path, stamp, kind, text.as_deref());
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn folder(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("milky-textcache-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An RTF file saying `words`, written directly (RTF is plain markup).
    fn rtf(path: &Path, words: &str) {
        std::fs::write(path, format!("{{\\rtf1\\ansi {words}}}")).unwrap();
    }

    #[test]
    fn expensive_text_is_cached_and_reused_until_the_file_changes() {
        let dir = folder("reuse");
        let path = dir.join("letter.rtf");
        rtf(&path, "kafka streams");
        let mut cache = TextCache::open_in_memory().unwrap();
        assert!(document_text(&path, &mut cache).unwrap().contains("kafka"));
        assert_eq!(cache.size().unwrap().0, 1);
        let stamp = Stamp::of(&path).unwrap();
        assert!(cache.get(&path, stamp).unwrap().unwrap().unwrap().contains("kafka"), "served from the cache");

        std::thread::sleep(std::time::Duration::from_millis(20));
        rtf(&path, "zookeeper ensemble members");
        let changed = Stamp::of(&path).unwrap();
        assert_eq!(cache.get(&path, changed).unwrap(), None, "a changed file is not served stale");
        assert!(document_text(&path, &mut cache).unwrap().contains("zookeeper"));
    }

    #[test]
    fn plain_files_are_never_cached() {
        let dir = folder("plain");
        std::fs::write(dir.join("notes.md"), "attention").unwrap();
        let mut cache = TextCache::open_in_memory().unwrap();
        assert_eq!(document_text(&dir.join("notes.md"), &mut cache).as_deref(), Some("attention"));
        assert_eq!(cache.size().unwrap(), (0, 0));
    }

    #[test]
    fn a_file_without_text_is_remembered_as_such() {
        let dir = folder("scan");
        let path = dir.join("scan.pdf");
        std::fs::write(&path, b"not really a pdf").unwrap();
        let mut cache = TextCache::open_in_memory().unwrap();
        assert_eq!(document_text(&path, &mut cache), None);
        let stamp = Stamp::of(&path).unwrap();
        assert_eq!(cache.get(&path, stamp).unwrap(), Some(None), "known to have no text: not retried");
    }

    #[test]
    fn removed_files_are_forgotten_and_the_cache_survives_reopening() {
        let dir = folder("reopen");
        let path = dir.join("letter.rtf");
        rtf(&path, "kafka");
        let db = dir.join(FILE_NAME);
        {
            let mut cache = TextCache::open(&db).unwrap();
            document_text(&path, &mut cache).unwrap();
        }
        let mut cache = TextCache::open(&db).unwrap();
        assert_eq!(cache.size().unwrap().0, 1);
        cache.remove(&path).unwrap();
        assert_eq!(cache.size().unwrap().0, 0);
    }
}
