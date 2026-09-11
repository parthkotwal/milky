//! Discovering installed macOS applications.
//!
//! An app is a directory whose name ends in `.app` — a bundle, not a file. We
//! take the display name from the directory name for now; the real name lives
//! in `Contents/Info.plist`, which needs a plist parser we do not have yet.

use std::path::Path;

/// Display name for an app bundle path.
///
/// `/Applications/Safari.app` becomes `Safari`. Returns `None` if the path is
/// not an app bundle, or if its name is not valid UTF-8.
pub fn app_name(path: &Path) -> Option<String> {
    let extension = path.extension()?;
    if extension != "app" {
        return None;
    }

    // now we know its .app path
    let stem = path.file_stem()?;
    let stem_str = stem.to_str()?;
    let name = stem_str.to_string();
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn takes_the_name_from_the_bundle_directory() {
        let path = PathBuf::from("/Applications/Safari.app");
        assert_eq!(app_name(&path).as_deref(), Some("Safari"));
    }

    #[test]
    fn keeps_spaces_and_punctuation() {
        let path = PathBuf::from("/Applications/Visual Studio Code.app");
        assert_eq!(app_name(&path).as_deref(), Some("Visual Studio Code"));
    }

    #[test]
    fn rejects_a_plain_directory() {
        let path = PathBuf::from("/Applications/Utilities");
        assert_eq!(app_name(&path), None);
    }

    #[test]
    fn rejects_a_non_app_extension() {
        let path = PathBuf::from("/Applications/notes.txt");
        assert_eq!(app_name(&path), None);
    }
}