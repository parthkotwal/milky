//! What a search result is: something Milky can find, describe, and act on.
//!
//! One candidate per *destination*. Identity names where a result goes, not
//! what it is called, so usage and selections attach to the place the user
//! went. See DECISIONS 2026-10-08 (contract v3).

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use crate::apps::App;
use crate::files::FileEntry;
use crate::matching::{NameKey, fold, words_of};
use crate::places::Place;
use crate::settings::SettingsPane;

/// What happens when the user picks a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Open an application bundle.
    Launch(PathBuf),
    /// Open a URL, such as a System Settings deep link.
    OpenUrl(String),
    /// Open a file or folder in its default app; folders open in Finder.
    Open(PathBuf),
}

/// Which kind of thing a result is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    App,
    Setting,
    /// A well-known folder: Downloads, Applications, Trash.
    Place,
    /// Any other folder found by the file walk.
    Folder,
    /// A file, including packages Finder shows as one item (`.pages`).
    File,
}

/// What kind of destination a candidate is, ordered by how likely it is to be
/// wanted when nothing learned says otherwise: apps, places, settings panes
/// and sections, then folders, then files. See search's ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prior {
    File,
    Folder,
    Section,
    Pane,
    /// Just below apps: only an app and a place sharing a name and usage
    /// differ, and then the app wins (`music`).
    Place,
    App,
}

/// One name a candidate answers to, with its precomputed matching key.
#[derive(Debug, Clone, PartialEq)]
pub struct Name {
    pub text: String,
    pub(crate) key: NameKey,
    /// Matches like any name but is never shown: the Trash answers to "Bin",
    /// yet a result titled "Bin" would name a folder that does not exist.
    pub alias: bool,
}

impl Name {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            key: NameKey::new(text),
            alias: false,
        }
    }

    /// A name to match on but never display.
    pub fn alias(text: &str) -> Self {
        Self {
            alias: true,
            ..Self::new(text)
        }
    }
}

/// Something search can return. One candidate per destination.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// `app:/Applications/Safari.app`, `settings:<pane id>`, or
    /// `settings:<pane id>#<anchor>`.
    pub id: String,
    pub kind: Kind,
    /// Shown under the title: an app's folder, a section's pane, or
    /// "System Settings" for a pane.
    pub subtitle: String,
    pub action: Action,
    /// Every name this destination answers to; the first is its default title.
    /// Never empty.
    pub names: Vec<Name>,
    /// Folded words describing the destination, from Apple's search index:
    /// sorted, no duplicates. Empty for apps.
    pub keywords: Vec<String>,
    /// What only walked files and folders have. `None` for everything else.
    pub file: Option<FileFacts>,
}

/// What file matching and ranking need beyond a file's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFacts {
    /// Folded words of the folders it sits in, below iCloud Drive or the home
    /// folder: `~/school/cse332/ex01.pdf` -> `["332", "cse", "cse332",
    /// "school"]`. Sorted, no duplicates.
    pub folder_words: Vec<String>,
    /// Folded extension without the dot (`pdf`); `None` for folders and
    /// files without one.
    pub extension: Option<String>,
    pub modified: Option<SystemTime>,
}

impl Candidate {
    /// The title to show when no particular name matched.
    pub fn title(&self) -> &str {
        self.names.first().map_or("", |name| name.text.as_str())
    }

    /// Apps, then places, then panes, then sections. A section's ID has an
    /// anchor.
    pub fn prior(&self) -> Prior {
        match self.kind {
            Kind::App => Prior::App,
            Kind::Place => Prior::Place,
            Kind::Folder => Prior::Folder,
            Kind::File => Prior::File,
            Kind::Setting if self.id.contains('#') => Prior::Section,
            Kind::Setting => Prior::Pane,
        }
    }
}

/// `app:<path>`, or `None` if the path is not UTF-8. A lossy ID would be a
/// broken identity.
pub fn app_id(path: &Path) -> Option<String> {
    Some(format!("app:{}", path.to_str()?))
}

/// `folder:<path>`, or `None` if the path is not UTF-8. The same ID a file
/// walk will give the same folder, so a place and a walked folder are one
/// destination with one usage history.
pub fn folder_id(path: &Path) -> Option<String> {
    Some(format!("folder:{}", path.to_str()?))
}

/// Where iCloud Drive keeps its files, relative to the home folder.
pub const ICLOUD_DRIVE: &str = "Library/Mobile Documents/com~apple~CloudDocs";

/// `path` as people read it: inside iCloud Drive as `iCloud Drive/Notes`, as
/// Finder names it; inside the home folder as `~/Downloads` (`~` for the home
/// folder itself); anywhere else, the full path.
pub fn display_path(path: &Path, home: &Path) -> String {
    let under = |root: &Path, label: &str| {
        let relative = path.strip_prefix(root).ok()?;
        Some(if relative.as_os_str().is_empty() {
            label.to_string()
        } else {
            format!("{label}/{}", relative.display())
        })
    };
    under(&home.join(ICLOUD_DRIVE), "iCloud Drive")
        .or_else(|| under(home, "~"))
        .unwrap_or_else(|| path.display().to_string())
}

/// A place as a candidate: named as Finder names it, also answering to its
/// aliases, subtitled by where it is, opened in Finder.
pub fn from_place(place: &Place, home: &Path) -> Option<Candidate> {
    let mut names = vec![Name::new(&place.name)];
    for alias in &place.aliases {
        if !names
            .iter()
            .any(|name| name.text.eq_ignore_ascii_case(alias))
        {
            names.push(Name::alias(alias));
        }
    }
    Some(Candidate {
        id: folder_id(&place.path)?,
        kind: Kind::Place,
        subtitle: display_path(&place.path, home),
        action: Action::Open(place.path.clone()),
        names,
        keywords: Vec::new(),
        file: None,
    })
}

/// `file:<path>`, or `None` if the path is not UTF-8.
pub fn file_id(path: &Path) -> Option<String> {
    Some(format!("file:{}", path.to_str()?))
}

/// A walked file or folder as a candidate: titled by its full name
/// (`resume.pdf`), subtitled by the folder it is in, opened in its default
/// app. A file's name is matched without its extension, so `resume` is an
/// exact match for `resume.pdf`; the extension is kept apart in
/// [`FileFacts`]. Folders keep their whole name (`archive.v2`).
pub fn from_file(entry: &FileEntry, home: &Path) -> Option<Candidate> {
    let text = entry.path.file_name()?.to_str()?;
    let (kind, id, matched, extension) = if entry.is_folder {
        (Kind::Folder, folder_id(&entry.path)?, text, None)
    } else {
        let stem = entry
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(text);
        let extension = entry
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(fold);
        (Kind::File, file_id(&entry.path)?, stem, extension)
    };
    let parent = entry.path.parent()?;
    let mut folder_words: Vec<String> = location_below_root(parent, home)
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .flat_map(words_of)
        .collect();
    folder_words.sort();
    folder_words.dedup();
    Some(Candidate {
        id,
        kind,
        subtitle: display_path(parent, home),
        action: Action::Open(entry.path.clone()),
        names: vec![Name {
            text: text.to_string(),
            key: NameKey::new(matched),
            alias: false,
        }],
        keywords: Vec::new(),
        file: Some(FileFacts {
            folder_words,
            extension,
            modified: entry.modified,
        }),
    })
}

/// `path` relative to the root people think of it from: iCloud Drive, the
/// home folder, or else the whole path. Folder words come from this, so
/// `Library`, `Mobile Documents`, and `Users` never count as context.
fn location_below_root<'a>(path: &'a Path, home: &Path) -> &'a Path {
    path.strip_prefix(home.join(ICLOUD_DRIVE))
        .or_else(|_| path.strip_prefix(home))
        .unwrap_or(path)
}

/// An app as a candidate: identified by path, subtitled by its folder
/// ("Python 3.12"), launched on pick.
pub fn from_app(app: &App) -> Option<Candidate> {
    let subtitle = app
        .path
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Some(Candidate {
        id: app_id(&app.path)?,
        kind: Kind::App,
        subtitle,
        action: Action::Launch(app.path.clone()),
        names: vec![Name {
            text: app.name.clone(),
            key: app.key.clone(),
            alias: false,
        }],
        keywords: Vec::new(),
        file: None,
    })
}

/// A pane as candidates: the pane itself first, then one candidate per section
/// anchor in first-appearance order, each answering to all of its items'
/// titles and keywords. Items titled exactly like the pane are covered by the
/// pane, which takes their keywords.
pub fn from_settings(pane: &SettingsPane) -> Vec<Candidate> {
    let mut candidates = vec![Candidate {
        id: format!("settings:{}", pane.id),
        kind: Kind::Setting,
        subtitle: "System Settings".to_string(),
        action: Action::OpenUrl(pane.url()),
        names: vec![Name::new(&pane.name)],
        keywords: Vec::new(),
        file: None,
    }];
    let mut by_anchor: HashMap<&str, usize> = HashMap::new();

    for item in &pane.items {
        if item.title.eq_ignore_ascii_case(&pane.name) {
            candidates[0]
                .keywords
                .extend(item.keywords.iter().flat_map(|k| words_of(k)));
            continue;
        }
        let index = *by_anchor.entry(item.anchor.as_str()).or_insert_with(|| {
            candidates.push(Candidate {
                id: format!("settings:{}#{}", pane.id, item.anchor),
                kind: Kind::Setting,
                subtitle: pane.name.clone(),
                action: Action::OpenUrl(pane.item_url(item)),
                names: Vec::new(),
                keywords: Vec::new(),
                file: None,
            });
            candidates.len() - 1
        });
        let candidate = &mut candidates[index];
        if !candidate.names.iter().any(|name| name.text == item.title) {
            candidate.names.push(Name::new(&item.title));
        }
        candidate
            .keywords
            .extend(item.keywords.iter().flat_map(|k| words_of(k)));
    }
    for candidate in &mut candidates {
        candidate.keywords.sort();
        candidate.keywords.dedup();
    }
    candidates
}

/// Every candidate: apps, then places, then settings, then files and folders.
/// A walked entry that is already an app or a place is left out, so
/// `~/Downloads` is one result and an app in the home folder is never also a
/// file.
pub fn build(
    apps: &[App],
    places: &[Place],
    panes: &[SettingsPane],
    files: &[FileEntry],
    home: &Path,
) -> Vec<Candidate> {
    let covered: HashSet<&Path> = apps
        .iter()
        .map(|app| app.path.as_path())
        .chain(places.iter().map(|place| place.path.as_path()))
        .collect();
    apps.iter()
        .filter_map(from_app)
        .chain(places.iter().filter_map(|place| from_place(place, home)))
        .chain(panes.iter().flat_map(from_settings))
        .chain(
            files
                .iter()
                .filter(|entry| !covered.contains(entry.path.as_path()))
                .filter_map(|entry| from_file(entry, home)),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SettingsItem;
    use std::collections::HashSet;

    fn idle(version: &str) -> App {
        App::new(
            PathBuf::from(format!("/Applications/Python {version}/IDLE.app")),
            "IDLE".to_string(),
        )
    }

    fn item(anchor: &str, title: &str) -> SettingsItem {
        SettingsItem {
            anchor: anchor.to_string(),
            title: title.to_string(),
            keywords: vec![],
        }
    }

    fn item_with(anchor: &str, title: &str, keywords: &[&str]) -> SettingsItem {
        SettingsItem {
            anchor: anchor.to_string(),
            title: title.to_string(),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
        }
    }

    fn wifi() -> SettingsPane {
        SettingsPane {
            id: "com.apple.wifi-settings-extension".to_string(),
            name: "Wi-Fi".to_string(),
            in_sidebar: true,
            items: vec![
                item("Advanced", "Advanced"),
                item("General_Join", "Ask to join networks"),
                item("Advanced", "Wi-Fi MAC Address"),
                item("Advanced", "Advanced"),
                item("General_Main", "Wi-Fi"),
            ],
        }
    }

    fn titles(candidate: &Candidate) -> Vec<&str> {
        candidate
            .names
            .iter()
            .map(|name| name.text.as_str())
            .collect()
    }

    #[test]
    fn an_app_is_identified_by_its_path_and_subtitled_by_its_folder() {
        let candidate = from_app(&idle("3.12")).expect("utf-8 path");
        assert_eq!(candidate.id, "app:/Applications/Python 3.12/IDLE.app");
        assert_eq!(candidate.kind, Kind::App);
        assert_eq!(candidate.title(), "IDLE");
        assert_eq!(candidate.subtitle, "Python 3.12");
        assert_eq!(
            candidate.action,
            Action::Launch(PathBuf::from("/Applications/Python 3.12/IDLE.app"))
        );
    }

    #[test]
    fn same_named_apps_get_different_ids_and_subtitles() {
        let a = from_app(&idle("3.12")).unwrap();
        let b = from_app(&idle("3.11")).unwrap();
        assert_ne!(a.id, b.id);
        assert_ne!(a.subtitle, b.subtitle);
    }

    #[test]
    fn a_pane_comes_first_and_opens_the_pane() {
        let candidates = from_settings(&wifi());
        let pane = &candidates[0];
        assert_eq!(pane.id, "settings:com.apple.wifi-settings-extension");
        assert_eq!(pane.title(), "Wi-Fi");
        assert_eq!(pane.subtitle, "System Settings");
        assert_eq!(
            pane.action,
            Action::OpenUrl(
                "x-apple.systempreferences:com.apple.wifi-settings-extension".to_string()
            )
        );
    }

    #[test]
    fn items_sharing_an_anchor_become_one_destination() {
        let candidates = from_settings(&wifi());
        let advanced = candidates
            .iter()
            .find(|c| c.id == "settings:com.apple.wifi-settings-extension#Advanced")
            .expect("advanced section");
        assert_eq!(titles(advanced), vec!["Advanced", "Wi-Fi MAC Address"]);
        assert_eq!(advanced.subtitle, "Wi-Fi");
        assert_eq!(
            advanced.action,
            Action::OpenUrl(
                "x-apple.systempreferences:com.apple.wifi-settings-extension?Advanced".to_string()
            )
        );
    }

    #[test]
    fn an_item_titled_like_its_pane_is_covered_by_the_pane() {
        let candidates = from_settings(&wifi());
        assert!(!candidates.iter().any(|c| c.id.ends_with("#General_Main")));
        assert_eq!(candidates.len(), 3, "pane, Advanced, General_Join");
    }

    #[test]
    fn sections_keep_the_order_they_first_appear_in() {
        let ids: Vec<String> = from_settings(&wifi()).into_iter().map(|c| c.id).collect();
        assert_eq!(
            ids,
            vec![
                "settings:com.apple.wifi-settings-extension",
                "settings:com.apple.wifi-settings-extension#Advanced",
                "settings:com.apple.wifi-settings-extension#General_Join",
            ]
        );
    }

    #[test]
    fn sections_collect_their_items_keywords() {
        let pane = SettingsPane {
            id: "com.apple.wifi-settings-extension".to_string(),
            name: "Wi-Fi".to_string(),
            in_sidebar: true,
            items: vec![
                item_with("Advanced", "Advanced", &["advanced", "MAC"]),
                item_with(
                    "Advanced",
                    "Wi-Fi MAC Address",
                    &["MAC address", "advanced"],
                ),
                item_with("General_Main", "Wi-Fi", &["wireless", "internet"]),
            ],
        };
        let candidates = from_settings(&pane);
        assert_eq!(
            candidates[1].keywords,
            vec!["address", "advanced", "mac"],
            "sorted, no duplicates"
        );
        assert_eq!(
            candidates[0].keywords,
            vec!["internet", "wireless"],
            "the pane takes keywords of items titled like it"
        );
    }

    #[test]
    fn apps_come_before_panes_before_sections() {
        let candidates = build(
            &[idle("3.12")],
            &[],
            &[wifi()],
            &[],
            Path::new("/Users/someone"),
        );
        assert_eq!(candidates[0].prior(), Prior::App);
        assert_eq!(candidates[1].prior(), Prior::Pane);
        assert_eq!(candidates[2].prior(), Prior::Section);
        assert!(Prior::App > Prior::Pane && Prior::Pane > Prior::Section);
        assert!(candidates[0].keywords.is_empty());
    }

    fn trash() -> Place {
        Place {
            path: PathBuf::from("/Users/someone/.Trash"),
            name: "Trash".to_string(),
            aliases: vec![
                "Bin".to_string(),
                "Recycle Bin".to_string(),
                "trash".to_string(),
            ],
        }
    }

    #[test]
    fn a_place_is_a_folder_opened_in_finder() {
        let home = Path::new("/Users/someone");
        let candidate = from_place(&trash(), home).unwrap();
        assert_eq!(candidate.id, "folder:/Users/someone/.Trash");
        assert_eq!(candidate.kind, Kind::Place);
        assert_eq!(candidate.prior(), Prior::Place);
        assert_eq!(candidate.subtitle, "~/.Trash");
        assert_eq!(
            candidate.action,
            Action::Open(PathBuf::from("/Users/someone/.Trash"))
        );
        assert_eq!(
            titles(&candidate),
            vec!["Trash", "Bin", "Recycle Bin"],
            "an alias repeating the name is dropped"
        );
        assert!(!candidate.names[0].alias);
        assert!(candidate.names[1..].iter().all(|name| name.alias));
    }

    #[test]
    fn paths_display_relative_to_home() {
        let home = Path::new("/Users/someone");
        assert_eq!(display_path(home, home), "~");
        assert_eq!(display_path(&home.join("Downloads"), home), "~/Downloads");
        assert_eq!(
            display_path(Path::new("/Applications"), home),
            "/Applications"
        );
        assert_eq!(
            display_path(Path::new("/Users/someoneelse"), home),
            "/Users/someoneelse"
        );
        let icloud = home.join(ICLOUD_DRIVE);
        assert_eq!(display_path(&icloud, home), "iCloud Drive");
        assert_eq!(
            display_path(&icloud.join("Notes/a.txt"), home),
            "iCloud Drive/Notes/a.txt"
        );
    }

    fn entry(path: &str, is_folder: bool) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            is_folder,
            modified: Some(SystemTime::UNIX_EPOCH),
        }
    }

    fn facts(candidate: &Candidate) -> &FileFacts {
        candidate.file.as_ref().expect("a file or folder")
    }

    #[test]
    fn a_file_is_titled_by_its_name_and_matched_without_its_extension() {
        let home = Path::new("/Users/someone");
        let file = from_file(
            &entry("/Users/someone/school/cse332/Resume.PDF", false),
            home,
        )
        .unwrap();
        assert_eq!(file.id, "file:/Users/someone/school/cse332/Resume.PDF");
        assert_eq!(file.kind, Kind::File);
        assert_eq!(file.prior(), Prior::File);
        assert_eq!(file.title(), "Resume.PDF");
        assert_eq!(file.subtitle, "~/school/cse332");
        assert_eq!(
            file.action,
            Action::Open(PathBuf::from("/Users/someone/school/cse332/Resume.PDF"))
        );
        assert_eq!(facts(&file).extension.as_deref(), Some("pdf"));
        assert_eq!(
            facts(&file).folder_words,
            vec!["332", "cse", "cse332", "school"]
        );
        assert_eq!(facts(&file).modified, Some(SystemTime::UNIX_EPOCH));
        assert_eq!(
            crate::matching::match_key(&file.names[0].key, "resume"),
            Some(crate::matching::MatchKind::Exact)
        );
    }

    #[test]
    fn a_folder_keeps_its_whole_name() {
        let home = Path::new("/Users/someone");
        let folder = from_file(&entry("/Users/someone/archive.v2", true), home).unwrap();
        assert_eq!(folder.id, "folder:/Users/someone/archive.v2");
        assert_eq!(folder.kind, Kind::Folder);
        assert_eq!(folder.prior(), Prior::Folder);
        assert_eq!(facts(&folder).extension, None);
        assert_eq!(folder.subtitle, "~");
        assert!(
            facts(&folder).folder_words.is_empty(),
            "directly in the home folder"
        );
        assert_eq!(
            crate::matching::match_key(&folder.names[0].key, "archive.v2"),
            Some(crate::matching::MatchKind::Exact)
        );
    }

    #[test]
    fn icloud_files_get_their_folder_words_from_inside_icloud_drive() {
        let home = Path::new("/Users/someone");
        let path = home.join(ICLOUD_DRIVE).join("Taxes/2025/w2.pdf");
        let file = from_file(
            &FileEntry {
                path,
                is_folder: false,
                modified: None,
            },
            home,
        )
        .unwrap();
        assert_eq!(file.subtitle, "iCloud Drive/Taxes/2025");
        assert_eq!(
            facts(&file).folder_words,
            vec!["2025", "taxes"],
            "not library or mobile documents"
        );
    }

    #[test]
    fn walked_entries_that_are_apps_or_places_are_not_repeated() {
        let home = Path::new("/Users/someone");
        let app = App::new(
            PathBuf::from("/Users/someone/Applications/Tool.app"),
            "Tool".to_string(),
        );
        let downloads = Place {
            path: home.join("Downloads"),
            name: "Downloads".to_string(),
            aliases: vec![],
        };
        let walked = [
            entry("/Users/someone/Applications/Tool.app", false),
            entry("/Users/someone/Downloads", true),
            entry("/Users/someone/Downloads/a.zip", false),
        ];
        let ids: Vec<String> = build(&[app], &[downloads], &[], &walked, home)
            .into_iter()
            .map(|candidate| candidate.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                "app:/Users/someone/Applications/Tool.app",
                "folder:/Users/someone/Downloads",
                "file:/Users/someone/Downloads/a.zip",
            ]
        );
    }

    #[test]
    fn folders_and_files_rank_below_settings() {
        assert!(Prior::Section > Prior::Folder && Prior::Folder > Prior::File);
    }

    #[test]
    fn places_rank_between_apps_and_panes() {
        assert!(Prior::App > Prior::Place && Prior::Place > Prior::Pane);
    }

    #[test]
    fn build_lists_apps_then_settings() {
        let candidates = build(
            &[idle("3.12")],
            &[],
            &[wifi()],
            &[],
            Path::new("/Users/someone"),
        );
        assert_eq!(candidates.len(), 4);
        assert_eq!(candidates[0].kind, Kind::App);
        assert!(candidates[1..].iter().all(|c| c.kind == Kind::Setting));
    }

    #[test]
    fn every_real_destination_has_a_unique_id() {
        let home = std::env::home_dir().unwrap();
        let candidates = build(
            &crate::apps::discover_apps(),
            &crate::places::discover_places(&home),
            &crate::settings::discover_settings(),
            &[],
            &home,
        );
        let ids: HashSet<&str> = candidates.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids.len(), candidates.len(), "duplicate ids");
        assert!(ids.contains("settings:com.apple.Displays-Settings.extension#nightShiftSection"));
        assert!(ids.contains("folder:/Applications"));
        assert!(candidates.iter().all(|c| !c.names.is_empty()));
    }
}
