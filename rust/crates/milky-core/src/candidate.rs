//! What a search result is: something Milky can find, describe, and act on.
//!
//! One candidate per *destination*. Identity names where a result goes, not
//! what it is called, so usage and selections attach to the place the user
//! went. See DECISIONS 2026-10-08 (contract v3).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::apps::App;
use crate::matching::{NameKey, words_of};
use crate::settings::SettingsPane;

/// What happens when the user picks a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Open an application bundle.
    Launch(PathBuf),
    /// Open a URL, such as a System Settings deep link.
    OpenUrl(String),
}

/// Which kind of thing a result is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    App,
    Setting,
}

/// What kind of destination a candidate is, ordered by how likely it is to be
/// wanted when nothing learned says otherwise: apps before panes before
/// sections. See search's ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prior {
    Section,
    Pane,
    App,
}

/// One name a candidate answers to, with its precomputed matching key.
#[derive(Debug, Clone, PartialEq)]
pub struct Name {
    pub text: String,
    pub(crate) key: NameKey,
}

impl Name {
    pub fn new(text: &str) -> Self {
        Self { text: text.to_string(), key: NameKey::new(text) }
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
}

impl Candidate {
    /// The title to show when no particular name matched.
    pub fn title(&self) -> &str {
        self.names.first().map_or("", |name| name.text.as_str())
    }

    /// Apps, then panes, then sections. A section's ID has an anchor.
    pub fn prior(&self) -> Prior {
        match self.kind {
            Kind::App => Prior::App,
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
        names: vec![Name { text: app.name.clone(), key: app.key.clone() }],
        keywords: Vec::new(),
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
    }];
    let mut by_anchor: HashMap<&str, usize> = HashMap::new();

    for item in &pane.items {
        if item.title.eq_ignore_ascii_case(&pane.name) {
            candidates[0].keywords.extend(item.keywords.iter().flat_map(|k| words_of(k)));
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
            });
            candidates.len() - 1
        });
        let candidate = &mut candidates[index];
        if !candidate.names.iter().any(|name| name.text == item.title) {
            candidate.names.push(Name::new(&item.title));
        }
        candidate.keywords.extend(item.keywords.iter().flat_map(|k| words_of(k)));
    }
    for candidate in &mut candidates {
        candidate.keywords.sort();
        candidate.keywords.dedup();
    }
    candidates
}

/// Every candidate: apps first, then settings.
pub fn build(apps: &[App], panes: &[SettingsPane]) -> Vec<Candidate> {
    apps.iter()
        .filter_map(from_app)
        .chain(panes.iter().flat_map(from_settings))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SettingsItem;
    use std::collections::HashSet;

    fn idle(version: &str) -> App {
        App::new(PathBuf::from(format!("/Applications/Python {version}/IDLE.app")), "IDLE".to_string())
    }

    fn item(anchor: &str, title: &str) -> SettingsItem {
        SettingsItem { anchor: anchor.to_string(), title: title.to_string(), keywords: vec![] }
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
        candidate.names.iter().map(|name| name.text.as_str()).collect()
    }

    #[test]
    fn an_app_is_identified_by_its_path_and_subtitled_by_its_folder() {
        let candidate = from_app(&idle("3.12")).expect("utf-8 path");
        assert_eq!(candidate.id, "app:/Applications/Python 3.12/IDLE.app");
        assert_eq!(candidate.kind, Kind::App);
        assert_eq!(candidate.title(), "IDLE");
        assert_eq!(candidate.subtitle, "Python 3.12");
        assert_eq!(candidate.action, Action::Launch(PathBuf::from("/Applications/Python 3.12/IDLE.app")));
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
        assert_eq!(pane.action, Action::OpenUrl("x-apple.systempreferences:com.apple.wifi-settings-extension".to_string()));
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
            Action::OpenUrl("x-apple.systempreferences:com.apple.wifi-settings-extension?Advanced".to_string())
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
                item_with("Advanced", "Wi-Fi MAC Address", &["MAC address", "advanced"]),
                item_with("General_Main", "Wi-Fi", &["wireless", "internet"]),
            ],
        };
        let candidates = from_settings(&pane);
        assert_eq!(candidates[1].keywords, vec!["address", "advanced", "mac"], "sorted, no duplicates");
        assert_eq!(candidates[0].keywords, vec!["internet", "wireless"], "the pane takes keywords of items titled like it");
    }

    #[test]
    fn apps_come_before_panes_before_sections() {
        let candidates = build(&[idle("3.12")], &[wifi()]);
        assert_eq!(candidates[0].prior(), Prior::App);
        assert_eq!(candidates[1].prior(), Prior::Pane);
        assert_eq!(candidates[2].prior(), Prior::Section);
        assert!(Prior::App > Prior::Pane && Prior::Pane > Prior::Section);
        assert!(candidates[0].keywords.is_empty());
    }

    #[test]
    fn build_lists_apps_then_settings() {
        let candidates = build(&[idle("3.12")], &[wifi()]);
        assert_eq!(candidates.len(), 4);
        assert_eq!(candidates[0].kind, Kind::App);
        assert!(candidates[1..].iter().all(|c| c.kind == Kind::Setting));
    }

    #[test]
    fn every_real_destination_has_a_unique_id() {
        let candidates = build(&crate::apps::discover_apps(), &crate::settings::discover_settings());
        let ids: HashSet<&str> = candidates.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids.len(), candidates.len(), "duplicate ids");
        assert!(ids.contains("settings:com.apple.Displays-Settings.extension#nightShiftSection"));
        assert!(candidates.iter().all(|c| !c.names.is_empty()));
    }
}