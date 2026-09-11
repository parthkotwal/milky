//! Discovering installed macOS applications.
//!
//! An app is a directory whose name ends in `.app` — a bundle, not a file. We
//! take the display name from the directory name for now; the real name lives
//! in `Contents/Info.plist`, which needs a plist parser we do not have yet.

use std::path::{Path, PathBuf};

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

/// App bundles directly inside one directory.
///
/// Returns an empty vector if the directory is missing or unreadable: a
/// directory we cannot read has nothing to contribute to the results, and other
/// directories in the scan should still be searched.
pub fn app_bundles_in(dir: &Path) -> Vec<PathBuf> {
    // try to read directory
    // if that fails: return []
    // otherwise iterate entries
    // skip entries that failed to read
    // turn each entry into a path
    // keep only paths that are directories
    // keep only paths where app_name(...) succeeds
    // collect those paths into Vec<PathBuf>
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut apps = Vec::new();

    // flatten: <Item = Result<DirEntry, io::Error> -> <Item = DirEntry>
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && app_name(&path).is_some() {
            apps.push(path);
        }
    }
    apps
}

/// Directories Milky scans for application bundles.
///
/// Each is also scanned one level deep, since `Utilities` and similar folders
/// group apps without being bundles themselves.
pub fn search_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];

    if let Some(home) = std::env::home_dir() {
        roots.push(home.join("Applications"));
    }
    
    roots
}

/// Subdirectories of `dir` that are not themselves app bundles.
fn subdirectories(dir: &Path) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut dirs = Vec::new();

    // flatten: <Item = Result<DirEntry, io::Error> -> <Item = DirEntry>
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && app_name(&path).is_none() {
            dirs.push(path);
        }
    }

    dirs
}

/// Every app bundle Milky can find.
pub fn discover() -> Vec<PathBuf> {
    let mut apps = Vec::new();
    for root in search_roots(){
        for app in app_bundles_in(&root) {
            apps.push(app);
        }

        for subdir in subdirectories(&root) {
            for app in app_bundles_in(&subdir) {
                apps.push(app);
            }
        }
    }

    apps
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

    #[test]
    fn finds_apple_apps_in_the_system_directory() {
        let found = app_bundles_in(Path::new("/System/Applications"));
        assert!(!found.is_empty(), "expected some Apple apps");
        assert!(
            found.iter().all(|path| path.extension() == Some("app".as_ref())),
            "every result should be an app bundle"
        );
    }

    #[test]
    fn missing_directory_is_empty_not_a_panic() {
        let found = app_bundles_in(Path::new("/nope/does/not/exist"));
        assert!(found.is_empty());
    }

    #[test]
    fn finds_safari_or_at_least_something_named() {
        let found = app_bundles_in(Path::new("/System/Applications"));
        let names: Vec<String> = found.iter().filter_map(|p| app_name(p)).collect();
        assert!(names.iter().any(|n| !n.is_empty()));
        println!("found {} apps: {:?}", names.len(), &names[..names.len().min(5)]);
    }

    #[test]
    fn search_roots_include_the_main_applications_folder() {
        let roots = search_roots();
        assert!(roots.contains(&PathBuf::from("/Applications")));
    }

    #[test]
    fn discover_finds_nested_utilities() {
        let found = discover();
        let names: Vec<String> = found.iter().filter_map(|p| app_name(p)).collect();
        assert!(
            names.iter().any(|n| n == "Terminal"),
            "Terminal.app lives in a Utilities subfolder, so finding it proves \
             we scan one level down"
        );
    }

    #[test]
    fn discover_finds_more_than_a_single_root() {
        let one_root = app_bundles_in(Path::new("/System/Applications")).len();
        let everything = discover().len();
        assert!(everything > one_root, "{everything} should exceed {one_root}");
        println!("discovered {everything} apps");
    }
}