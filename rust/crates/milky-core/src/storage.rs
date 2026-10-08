//! Where Milky keeps durable local state, and how it writes it safely.
//!
//! Everything here stays on this Mac. Nothing in this module sends data anywhere.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// `~/Library/Application Support/Milky`: Apple's convention for app state the
/// user should not have to manage.
pub fn data_dir() -> Result<PathBuf, StorageError> {
    let home = std::env::home_dir().ok_or(StorageError::NoHomeDirectory)?;
    Ok(home.join("Library/Application Support/Milky"))
}

/// Replace the contents of `path` with `bytes`, crash-safely.
///
/// Writes a sibling temporary file, forces it to disk, then renames it over the
/// target. `rename` is atomic within one filesystem, so a reader sees either the
/// old complete file or the new complete file, never a truncated one. Without
/// the sync, the rename can reach disk before the data does, and a power loss
/// leaves an empty file under the real name. Costs about 5 ms on this machine.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = sibling(path, ".tmp");
    let mut file = File::create(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    Ok(())
}

/// Append `line` plus a newline to `path`, creating it if needed, and force it
/// to disk. One `write` call per line; callers that may run concurrently must
/// serialize appends themselves. Costs about 4 ms on this machine.
pub fn append_line(path: &Path, line: &[u8]) -> Result<(), StorageError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut record = Vec::with_capacity(line.len() + 1);
    record.extend_from_slice(line);
    record.push(b'\n');

    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&record)?;
    file.sync_all()?;
    Ok(())
}

/// `path` with `suffix` appended to its file name: `usage.json` -> `usage.json.tmp`.
pub(crate) fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// Something that went wrong reading or writing durable state.
#[derive(Debug)]
pub enum StorageError {
    /// Reading, writing, renaming, or creating directories failed.
    Io(io::Error),
    /// A file existed but did not contain what we expected.
    Format(serde_json::Error),
    /// No home directory, so there is nowhere standard to keep state.
    NoHomeDirectory,
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::Io(err) => write!(f, "storage i/o failed: {err}"),
            StorageError::Format(err) => write!(f, "stored data was malformed: {err}"),
            StorageError::NoHomeDirectory => write!(f, "could not locate a home directory"),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StorageError::Io(err) => Some(err),
            StorageError::Format(err) => Some(err),
            StorageError::NoHomeDirectory => None,
        }
    }
}

impl From<io::Error> for StorageError {
    fn from(err: io::Error) -> Self {
        StorageError::Io(err)
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(err: serde_json::Error) -> Self {
        StorageError::Format(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("milky-storage-{label}-{}", std::process::id()))
    }

    #[test]
    fn data_dir_follows_the_macos_convention() {
        let dir = data_dir().expect("a home directory");
        assert!(dir.ends_with("Library/Application Support/Milky"));
    }

    #[test]
    fn write_atomic_replaces_contents_and_leaves_no_temp_file() {
        let dir = temp_dir("atomic");
        let path = dir.join("state.json");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        assert!(!sibling(&path, ".tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn append_line_adds_one_line_per_call() {
        let dir = temp_dir("append");
        let path = dir.join("log.jsonl");
        append_line(&path, b"{\"a\":1}").unwrap();
        append_line(&path, b"{\"a\":2}").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"a\":1}\n{\"a\":2}\n");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sibling_appends_to_the_file_name() {
        let path = Path::new("/tmp/milky/usage.json");
        assert_eq!(
            sibling(path, ".tmp"),
            Path::new("/tmp/milky/usage.json.tmp")
        );
    }
}
