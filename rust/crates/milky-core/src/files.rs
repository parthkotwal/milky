//! Discovering files and folders.
//!
//! Walks the home folder and iCloud Drive the way Finder shows them: hidden
//! items are skipped, packages (`.app`, `.pages`, `.photoslibrary`) are single
//! results, and folders that tools generate (dependencies, caches, virtual
//! environments) are left out entirely, as are folders the user excludes in
//! `exclusions.txt`.

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::storage::{self, StorageError};

/// One file or folder found by a walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PathBuf,
    /// A folder Finder opens into. Packages are files: they open as documents.
    pub is_folder: bool,
}

/// Everything one or more walks found.
#[derive(Debug, Default)]
pub struct Walk {
    pub entries: Vec<FileEntry>,
    /// Folders that exist but could not be read, such as ones macOS privacy
    /// settings block. Their contents are missing from `entries`.
    pub unreadable: Vec<PathBuf>,
}

/// Entries whose presence marks their folder as generated, so the folder labels
/// itself and needs no configuration. Cargo, pytest, mypy, ruff, and uv write
/// `CACHEDIR.TAG` (a cross-tool convention); every Python virtual environment
/// has `pyvenv.cfg`; every conda environment has a `conda-meta` folder.
const GENERATED_MARKERS: &[&str] = &["CACHEDIR.TAG", "pyvenv.cfg", "conda-meta"];

/// What `exclusions.txt` holds when Milky creates it: output of common tools
/// that carries no marker. Never folder names people also use for their own
/// work, like `build` or `vendor`.
pub const DEFAULT_EXCLUSIONS: &str = "\
# Folders Milky does not index, one per line. Delete a line to index that
# folder again; delete this file to restore the defaults.
#
# A name (node_modules) skips every folder with that name.
# A path (~/go/pkg/mod, /Volumes/Backup) skips that one folder.

# Dependencies installed by package managers.
node_modules
bower_components
Pods

# Python bytecode.
__pycache__

# Go's module cache.
~/go/pkg/mod
";

/// Folders the user does not want indexed: names matched anywhere, and exact
/// paths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exclusions {
    names: Vec<String>,
    paths: Vec<PathBuf>,
}

impl Exclusions {
    /// Rules from exclusion-file text, and the lines that are not rules.
    ///
    /// Blank lines and `#` comments are skipped. A trailing `/` is allowed. A
    /// line starting with `~/` or `/` is a path, with `~` meaning `home`; a line
    /// without `/` is a name. Anything else (`foo/bar`) is returned as ignored
    /// so a typo is visible instead of silently matching nothing.
    pub fn parse(text: &str, home: &Path) -> (Self, Vec<String>) {
        let mut rules = Self::default();
        let mut ignored = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let rule = line.trim_end_matches('/');
            if rule == "~" {
                rules.paths.push(home.to_path_buf());
            } else if let Some(relative) = rule.strip_prefix("~/") {
                rules.paths.push(home.join(relative));
            } else if rule.starts_with('/') {
                rules.paths.push(PathBuf::from(rule));
            } else if !rule.is_empty() && !rule.contains('/') {
                rules.names.push(rule.to_string());
            } else {
                ignored.push(line.to_string());
            }
        }
        (rules, ignored)
    }

    /// [`DEFAULT_EXCLUSIONS`], parsed.
    pub fn defaults(home: &Path) -> Self {
        Self::parse(DEFAULT_EXCLUSIONS, home).0
    }

    /// Whether the folder at `path`, named `name`, is excluded.
    pub fn excludes(&self, path: &Path, name: &OsStr) -> bool {
        self.names.iter().any(|excluded| name == excluded.as_str())
            || self.paths.iter().any(|excluded| excluded == path)
    }

    /// How many rules there are.
    pub fn len(&self) -> usize {
        self.names.len() + self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// `~/Library/Application Support/Milky/exclusions.txt`.
pub fn exclusions_path() -> Result<PathBuf, StorageError> {
    Ok(storage::data_dir()?.join("exclusions.txt"))
}

/// The exclusions in `path`, and its lines that are not rules. A missing file
/// is created with [`DEFAULT_EXCLUSIONS`] first, so the defaults are visible
/// and editable. On error, callers should fall back to
/// [`Exclusions::defaults`]: a broken file must not mean indexing everything.
pub fn load_exclusions(path: &Path, home: &Path) -> Result<(Exclusions, Vec<String>), StorageError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            storage::write_atomic(path, DEFAULT_EXCLUSIONS.as_bytes())?;
            DEFAULT_EXCLUSIONS.to_string()
        }
        Err(error) => return Err(error.into()),
    };
    Ok(Exclusions::parse(&text, home))
}

/// Every file and folder under `root`, except hidden, generated, and excluded
/// folders and what is inside them. `root` itself is not an entry.
/// Symbolic links are not followed or listed: following them can loop and
/// lists the same files twice.
pub fn walk(root: &Path, exclusions: &Exclusions) -> Walk {
    let mut found = Walk::default();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let children = match std::fs::read_dir(&dir) {
            Ok(children) => children,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(_) => {
                found.unreadable.push(dir);
                continue;
            }
        };

        for child in children.flatten() {
            let name = child.file_name();
            if is_dot_name(&name) {
                continue;
            }
            let Ok(file_type) = child.file_type() else {
                continue;
            };
            let path = child.path();

            if file_type.is_dir() {
                let skipped = exclusions.excludes(&path, &name)
                    || has_hidden_flag(&child)
                    || has_generated_marker(&path);
                if skipped {
                    continue;
                }
                let is_folder = !is_package(&path);
                if is_folder {
                    pending.push(path.clone());
                }
                found.entries.push(FileEntry { path, is_folder });
            } else if file_type.is_file() {
                found.entries.push(FileEntry { path, is_folder: false });
            }
        }
    }
    found
}

/// The folders a walk starts from: the home folder and iCloud Drive (which
/// lives inside the hidden `~/Library`, so the home walk never reaches it).
pub fn roots(home: &Path) -> Vec<PathBuf> {
    vec![home.to_path_buf(), home.join("Library/Mobile Documents/com~apple~CloudDocs")]
}

/// Everything under [`roots`], minus `exclusions`.
pub fn discover_files(home: &Path, exclusions: &Exclusions) -> Walk {
    let mut all = Walk::default();
    for root in roots(home) {
        let found = walk(&root, exclusions);
        all.entries.extend(found.entries);
        all.unreadable.extend(found.unreadable);
    }
    all
}

/// Names starting with a dot are hidden by convention.
fn is_dot_name(name: &OsStr) -> bool {
    name.as_encoded_bytes().starts_with(b".")
}

/// Finder also hides items with the `UF_HIDDEN` flag; that is how
/// `~/Library` is hidden. Checked for folders only: hidden files without a
/// dot are rare, and reading flags costs a system call per item.
fn has_hidden_flag(entry: &std::fs::DirEntry) -> bool {
    use std::os::macos::fs::MetadataExt;
    const UF_HIDDEN: u32 = 0x8000;
    entry
        .metadata()
        .is_ok_and(|metadata| metadata.st_flags() & UF_HIDDEN != 0)
}

fn has_generated_marker(dir: &Path) -> bool {
    GENERATED_MARKERS.iter().any(|marker| dir.join(marker).exists())
}

/// Whether Finder shows this folder as a single item: an app, a `.pages`
/// document, a photo library. Only names with an extension can be packages,
/// which skips the LaunchServices call for almost every folder.
pub fn is_package(path: &Path) -> bool {
    path.extension().is_some() && crate::launch_services::is_package(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A fresh, empty folder for one test. Each test passes its own name so
    /// tests running in parallel never share one.
    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("milky-files-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
    }

    /// A walk with the default exclusions, as a fresh install has.
    fn walked(root: &Path) -> Walk {
        walk(root, &Exclusions::defaults(root))
    }

    /// Entry paths relative to `root`, sorted, folders marked with a trailing `/`.
    fn listed(walk: &Walk, root: &Path) -> Vec<String> {
        let mut names: Vec<String> = walk
            .entries
            .iter()
            .map(|entry| {
                let relative = entry.path.strip_prefix(root).unwrap().to_string_lossy();
                if entry.is_folder { format!("{relative}/") } else { relative.into_owned() }
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn finds_nested_files_and_folders() {
        let root = scratch("nested");
        touch(&root.join("notes.txt"));
        touch(&root.join("school/cse332/ex01.pdf"));
        assert_eq!(
            listed(&walked(&root), &root),
            vec!["notes.txt", "school/", "school/cse332/", "school/cse332/ex01.pdf"]
        );
    }

    #[test]
    fn skips_dot_names_and_their_contents() {
        let root = scratch("dots");
        touch(&root.join(".env"));
        touch(&root.join(".git/config"));
        touch(&root.join("kept.md"));
        assert_eq!(listed(&walked(&root), &root), vec!["kept.md"]);
    }

    #[test]
    fn skips_generated_folders_by_name_and_by_marker() {
        let root = scratch("generated");
        touch(&root.join("app/node_modules/react/index.js"));
        touch(&root.join("app/__pycache__/x.pyc"));
        touch(&root.join("app/target/CACHEDIR.TAG"));
        touch(&root.join("app/target/debug/milky"));
        touch(&root.join("app/venv/pyvenv.cfg"));
        touch(&root.join("app/miniconda3/conda-meta/history"));
        touch(&root.join("app/main.rs"));
        assert_eq!(listed(&walked(&root), &root), vec!["app/", "app/main.rs"]);
    }

    #[test]
    fn a_package_is_one_file_and_is_not_entered() {
        let root = scratch("package");
        touch(&root.join("Tool.app/Contents/Info.plist"));
        touch(&root.join("archive.v2/data.txt"));
        assert_eq!(
            listed(&walked(&root), &root),
            vec!["Tool.app", "archive.v2/", "archive.v2/data.txt"],
            "an extension alone does not make a package"
        );
    }

    #[test]
    fn symlinks_are_not_followed() {
        let root = scratch("symlink");
        touch(&root.join("real/file.txt"));
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        std::os::unix::fs::symlink(&root, root.join("real/loop")).unwrap();
        assert_eq!(listed(&walked(&root), &root), vec!["real/", "real/file.txt"]);
    }

    #[test]
    fn unreadable_folders_are_reported_not_fatal() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("unreadable");
        touch(&root.join("locked/secret.txt"));
        touch(&root.join("open.txt"));
        fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
        let found = walked(&root);
        fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(listed(&found, &root), vec!["locked/", "open.txt"]);
        assert_eq!(found.unreadable, vec![root.join("locked")]);
    }

    #[test]
    fn excluded_paths_skip_one_folder_and_names_skip_them_all() {
        let root = scratch("excluded");
        touch(&root.join("go/pkg/mod/golang.org/x.go"));
        touch(&root.join("go/pkg/modules.txt"));
        touch(&root.join("a/Pods/x"));
        touch(&root.join("b/Pods/y"));
        touch(&root.join("keep.txt"));
        assert_eq!(
            listed(&walked(&root), &root),
            vec!["a/", "b/", "go/", "go/pkg/", "go/pkg/modules.txt", "keep.txt"]
        );
    }

    #[test]
    fn with_no_exclusions_tool_folders_are_indexed() {
        let root = scratch("no-exclusions");
        touch(&root.join("node_modules/x.js"));
        assert_eq!(
            listed(&walk(&root, &Exclusions::default()), &root),
            vec!["node_modules/", "node_modules/x.js"]
        );
    }

    #[test]
    fn parses_names_paths_and_comments() {
        let home = Path::new("/Users/someone");
        let text = "# comment\n\n  node_modules  \nbuild/\n~/go/pkg/mod/\n/Volumes/Backup\n~\n";
        let (rules, ignored) = Exclusions::parse(text, home);
        assert!(ignored.is_empty(), "{ignored:?}");
        assert_eq!(rules.len(), 5);
        assert!(rules.excludes(Path::new("/x/node_modules"), OsStr::new("node_modules")));
        assert!(rules.excludes(Path::new("/x/build"), OsStr::new("build")), "a trailing slash is allowed");
        assert!(rules.excludes(&home.join("go/pkg/mod"), OsStr::new("mod")));
        assert!(!rules.excludes(&home.join("go/pkg/modx"), OsStr::new("modx")));
        assert!(!rules.excludes(Path::new("/elsewhere/go/pkg/mod"), OsStr::new("mod")), "paths are exact");
        assert!(rules.excludes(Path::new("/Volumes/Backup"), OsStr::new("Backup")));
        assert!(rules.excludes(home, OsStr::new("someone")));
    }

    #[test]
    fn lines_that_are_not_rules_are_reported() {
        let (rules, ignored) = Exclusions::parse("Projects/old\n/\nnode_modules\n", Path::new("/h"));
        assert_eq!(ignored, vec!["Projects/old", "/"]);
        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn the_defaults_parse_cleanly() {
        let home = Path::new("/Users/someone");
        let (rules, ignored) = Exclusions::parse(DEFAULT_EXCLUSIONS, home);
        assert!(ignored.is_empty(), "{ignored:?}");
        assert_eq!(rules, Exclusions::defaults(home));
        for name in ["node_modules", "bower_components", "Pods", "__pycache__"] {
            assert!(rules.excludes(&Path::new("/p").join(name), OsStr::new(name)), "{name}");
        }
        assert!(rules.excludes(&home.join("go/pkg/mod"), OsStr::new("mod")));
        for kept in ["build", "dist", "vendor", "target"] {
            assert!(!rules.excludes(&Path::new("/p").join(kept), OsStr::new(kept)), "{kept}");
        }
    }

    #[test]
    fn a_missing_exclusions_file_is_created_with_the_defaults() {
        let dir = scratch("load-missing");
        let path = dir.join("Milky/exclusions.txt");
        let (rules, ignored) = load_exclusions(&path, &dir).unwrap();
        assert_eq!(rules, Exclusions::defaults(&dir));
        assert!(ignored.is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap(), DEFAULT_EXCLUSIONS);
    }

    #[test]
    fn an_edited_exclusions_file_is_respected() {
        let dir = scratch("load-edited");
        let path = dir.join("exclusions.txt");
        fs::write(&path, "node_modules\n").unwrap();
        let (rules, _) = load_exclusions(&path, &dir).unwrap();
        assert_eq!(rules.len(), 1, "deleted defaults stay deleted");
        assert_eq!(fs::read_to_string(&path).unwrap(), "node_modules\n", "the file is not rewritten");
    }

    #[test]
    fn a_missing_root_is_empty_not_unreadable() {
        let found = walked(Path::new("/nonexistent/milky"));
        assert!(found.entries.is_empty() && found.unreadable.is_empty());
    }
}