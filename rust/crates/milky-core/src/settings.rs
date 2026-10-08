//! System Settings panes and the sections inside them, as macOS describes them.
//!
//! Each pane is an app extension with extension point
//! `com.apple.Settings.extension.ui`. Its `Info.plist` says whether it accepts
//! `x-apple.systempreferences:` links and names its search-terms file. That file
//! (`<lang>.lproj/<name>.searchTerms`, an XML plist) lists the pane's sections:
//! each section anchor carries titled entries with comma-separated keywords. The
//! anchors are the same ones the links accept, so
//! `x-apple.systempreferences:com.apple.Displays-Settings.extension?nightShiftSection`
//! opens Night Shift directly (verified on macOS 26).
//!
//! English only for now.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use plist::{Dictionary, Value};

use crate::matching::camel_parts;

const EXTENSION_POINT: &str = "com.apple.Settings.extension.ui";
const LANGUAGE: &str = "en";

/// Where settings panes live. The second holds General, which contains panes
/// such as About and Storage.
const PANE_DIRS: [&str; 2] = [
    "/System/Library/ExtensionKit/Extensions",
    "/System/Applications/System Settings.app/Contents/PlugIns",
];

/// One System Settings pane.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsPane {
    /// Bundle identifier; also what the `x-apple.systempreferences:` link takes.
    pub id: String,
    /// The name System Settings shows: "Privacy & Security", "Battery".
    pub name: String,
    /// False for panes reachable only from elsewhere, such as About under General.
    pub in_sidebar: bool,
    /// Searchable entries inside the pane.
    pub items: Vec<SettingsItem>,
}

/// One searchable entry inside a pane. Several entries can share an anchor:
/// Wi-Fi's `Advanced` section has entries titled "Advanced", "Wi‑Fi MAC Address",
/// and others.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsItem {
    /// Section anchor, as accepted after `?` in the link.
    pub anchor: String,
    pub title: String,
    /// Apple's index words for this entry: "mac address", "known networks".
    pub keywords: Vec<String>,
}

impl SettingsPane {
    /// Link that opens the pane.
    pub fn url(&self) -> String {
        format!("x-apple.systempreferences:{}", self.id)
    }

    /// Link that opens the pane at one item's section.
    pub fn item_url(&self, item: &SettingsItem) -> String {
        format!("x-apple.systempreferences:{}?{}", self.id, item.anchor)
    }
}

/// Every linkable System Settings pane on this Mac, with its sections.
///
/// About 50 panes and 700 items; read once, like apps.
pub fn discover_settings() -> Vec<SettingsPane> {
    let dirs: Vec<PathBuf> = PANE_DIRS.iter().map(PathBuf::from).collect();
    discover_settings_in(&dirs, has_internal_battery())
}

/// Like [`discover_settings`], reading panes from `dirs`. Duplicate pane IDs are
/// kept once, first directory wins. Unreadable bundles are skipped: at least one
/// system extension ships a malformed `Info.plist`.
pub fn discover_settings_in(dirs: &[PathBuf], battery: bool) -> Vec<SettingsPane> {
    let mut seen = HashSet::new();
    let mut panes = Vec::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        let mut bundles: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "appex"))
            .collect();
        bundles.sort();
        for bundle in bundles {
            if let Some(pane) = read_pane(&bundle, battery)
                && seen.insert(pane.id.clone())
            {
                panes.push(pane);
            }
        }
    }
    panes
}

fn read_pane(bundle: &Path, battery: bool) -> Option<SettingsPane> {
    let info = Value::from_file(bundle.join("Contents/Info.plist")).ok()?;
    let info = info.as_dictionary()?;
    let extension = info.get("EXAppExtensionAttributes")?.as_dictionary()?;
    if extension.get("EXExtensionPointIdentifier")?.as_string()? != EXTENSION_POINT {
        return None;
    }
    let attributes = extension
        .get("SettingsExtensionAttributes")
        .and_then(Value::as_dictionary);
    let flag = |key: &str| attributes.and_then(|a| a.get(key)).and_then(Value::as_boolean);
    // A pane that refuses links cannot be opened from a search result.
    if flag("allowsXAppleSystemPreferencesURLScheme") != Some(true) {
        return None;
    }
    let id = info.get("CFBundleIdentifier")?.as_string()?.to_string();
    let resources = bundle.join("Contents/Resources");
    let terms_file = attributes
        .and_then(|a| a.get("searchTermsFileName"))
        .and_then(Value::as_string);
    Some(SettingsPane {
        id,
        name: pane_name(info, attributes, &resources, bundle, battery),
        in_sidebar: flag("presentsInSidebar") != Some(false),
        items: read_items(&resources, terms_file),
    })
}

/// The name System Settings shows. Neither `Info.plist` nor LaunchServices is
/// right for every pane, so try, in order:
/// 1. a hardware-dependent sidebar name ("Battery" vs "Energy");
/// 2. the localized `CFBundleDisplayName` in `InfoPlist.loctable`;
/// 3. `Info.plist`'s `CFBundleDisplayName`, or the bundle name, cleaned up if it
///    looks like an identifier.
fn pane_name(
    info: &Dictionary,
    attributes: Option<&Dictionary>,
    resources: &Path,
    bundle: &Path,
    battery: bool,
) -> String {
    let representations = attributes
        .and_then(|a| a.get("representations"))
        .and_then(Value::as_array);
    for representation in representations.into_iter().flatten().filter_map(Value::as_dictionary) {
        let Some(key) = representation.get("sidebar-name").and_then(Value::as_string) else {
            continue; // representations without a name only control visibility
        };
        let predicate = representation
            .get("predicate")
            .and_then(Value::as_string)
            .unwrap_or("");
        if predicate_holds(predicate, battery) {
            // Usually a localization key, sometimes literal text ("Login Password").
            return localized(resources, key).unwrap_or_else(|| key.to_string());
        }
    }
    if let Some(name) = loctable_string(&resources.join("InfoPlist.loctable"), "CFBundleDisplayName")
        .filter(|name| !name.trim().is_empty())
    {
        return name;
    }
    let raw = info
        .get("CFBundleDisplayName")
        .and_then(Value::as_string)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            bundle
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    clean_identifier_name(&raw)
}

/// Evaluate the predicates panes use to choose a sidebar name.
///
/// Only the power source is actually checked. Other capabilities (Touch ID,
/// Apple Intelligence) are assumed present, which is right for current Apple
/// laptops and wrong on some older or desktop Macs, where those panes are named
/// differently. An empty predicate always holds.
fn predicate_holds(predicate: &str, battery: bool) -> bool {
    match predicate.trim() {
        "" => true,
        "powersource.battery == YES" => battery,
        "powersource.battery != YES" => !battery,
        other => other.ends_with("== YES"),
    }
}

/// Look `key` up in every English localization table of a pane.
fn localized(resources: &Path, key: &str) -> Option<String> {
    let mut tables: Vec<PathBuf> = fs::read_dir(resources)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "loctable"))
        .collect();
    tables.sort();
    tables.iter().find_map(|table| loctable_string(table, key))
}

/// A `.loctable` is a plist mapping language codes to string tables.
fn loctable_string(path: &Path, key: &str) -> Option<String> {
    let table = Value::from_file(path).ok()?;
    table
        .as_dictionary()?
        .get(LANGUAGE)?
        .as_dictionary()?
        .get(key)?
        .as_string()
        .map(str::to_string)
}

/// Last resort for a name that is really an identifier:
/// `"HeadphoneSettingsExtension"` -> `"Headphone"`, `"LoginItems"` -> `"Login Items"`.
/// Names that already contain a space are left alone.
fn clean_identifier_name(raw: &str) -> String {
    if raw.contains(' ') {
        return raw.to_string();
    }
    let mut name = raw;
    for suffix in ["SettingsExtension", "PreferenceExtension", "Extension", "Settings", "Ext"] {
        if let Some(stripped) = name.strip_suffix(suffix).filter(|s| !s.is_empty()) {
            name = stripped;
            break;
        }
    }
    camel_parts(name).join(" ")
}

fn read_items(resources: &Path, file_name: Option<&str>) -> Vec<SettingsItem> {
    let dir = resources.join(format!("{LANGUAGE}.lproj"));
    let named = file_name
        .map(|name| dir.join(format!("{name}.searchTerms")))
        .filter(|path| path.exists());
    let Some(path) = named.or_else(|| first_search_terms(&dir)) else {
        return Vec::new();
    };
    fs::read(&path)
        .map(|bytes| parse_search_terms(&bytes))
        .unwrap_or_default()
}

fn first_search_terms(dir: &Path) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "searchTerms"))
        .collect();
    files.sort();
    files.into_iter().next()
}

/// Parse a `.searchTerms` plist: anchor -> `localizableStrings` -> entries with
/// `title` and comma-separated `index` keywords. Entries without a title are
/// skipped; a file that is not a dictionary yields nothing.
pub fn parse_search_terms(bytes: &[u8]) -> Vec<SettingsItem> {
    let Ok(value) = Value::from_reader(std::io::Cursor::new(bytes)) else {
        return Vec::new();
    };
    let Some(anchors) = value.as_dictionary() else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for (anchor, body) in anchors {
        let entries = body
            .as_dictionary()
            .and_then(|b| b.get("localizableStrings"))
            .and_then(Value::as_array);
        for entry in entries.into_iter().flatten().filter_map(Value::as_dictionary) {
            let Some(title) = entry
                .get("title")
                .and_then(Value::as_string)
                .map(str::trim)
                .filter(|title| !title.is_empty())
            else {
                continue;
            };
            let keywords = entry
                .get("index")
                .and_then(Value::as_string)
                .map(|index| {
                    index
                        .split(',')
                        .map(str::trim)
                        .filter(|keyword| !keyword.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            items.push(SettingsItem {
                anchor: anchor.clone(),
                title: title.to_string(),
                keywords,
            });
        }
    }
    items
}

/// Whether this Mac has an internal battery, which decides between the
/// "Battery" and "Energy" pane names. Asks IOKit's power-source API.
#[cfg(target_os = "macos")]
fn has_internal_battery() -> bool {
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::{CFType, CFTypeRef, TCFType};
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::string::CFString;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
        fn IOPSCopyPowerSourcesList(blob: CFTypeRef) -> CFArrayRef;
        fn IOPSGetPowerSourceDescription(blob: CFTypeRef, source: CFTypeRef) -> CFDictionaryRef;
    }

    // SAFETY: the two Copy functions return +1 references, taken over by the
    // `wrap_under_create_rule` wrappers, which release them. The description is
    // a Get (+0) reference owned by `blob`, which outlives every use of it.
    unsafe {
        let blob = IOPSCopyPowerSourcesInfo();
        if blob.is_null() {
            return false;
        }
        let blob = CFType::wrap_under_create_rule(blob);
        let list = IOPSCopyPowerSourcesList(blob.as_CFTypeRef());
        if list.is_null() {
            return false;
        }
        let list: CFArray<CFType> = CFArray::wrap_under_create_rule(list);
        let type_key = CFString::from_static_string("Type");
        list.iter().any(|source| {
            let description = IOPSGetPowerSourceDescription(blob.as_CFTypeRef(), source.as_CFTypeRef());
            if description.is_null() {
                return false;
            }
            let description: CFDictionary<CFString, CFType> =
                CFDictionary::wrap_under_get_rule(description);
            description
                .find(&type_key)
                .and_then(|kind| kind.downcast::<CFString>())
                .is_some_and(|kind| kind == "InternalBattery")
        })
    }
}

#[cfg(not(target_os = "macos"))]
fn has_internal_battery() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Advanced</key>
    <dict>
        <key>localizableStrings</key>
        <array>
            <dict>
                <key>index</key><string>MAC, address, mac address,  , advanced</string>
                <key>title</key><string>Wi-Fi MAC Address</string>
            </dict>
            <dict>
                <key>index</key><string>no title here</string>
            </dict>
        </array>
    </dict>
    <key>General_Join</key>
    <dict>
        <key>localizableStrings</key>
        <array>
            <dict><key>title</key><string>Ask to join networks</string></dict>
        </array>
    </dict>
</dict>
</plist>"#;

    #[test]
    fn search_terms_become_titled_items_with_keywords() {
        let items = parse_search_terms(SAMPLE.as_bytes());
        assert_eq!(items.len(), 2, "the untitled entry is skipped: {items:?}");
        assert_eq!(items[0].anchor, "Advanced");
        assert_eq!(items[0].title, "Wi-Fi MAC Address");
        assert_eq!(items[0].keywords, vec!["MAC", "address", "mac address", "advanced"]);
        assert_eq!(items[1].anchor, "General_Join");
        assert!(items[1].keywords.is_empty());
    }

    #[test]
    fn unparsable_search_terms_yield_nothing() {
        assert!(parse_search_terms(b"not a plist").is_empty());
    }

    #[test]
    fn identifier_names_are_cleaned_up() {
        assert_eq!(clean_identifier_name("HeadphoneSettingsExtension"), "Headphone");
        assert_eq!(clean_identifier_name("LoginItems"), "Login Items");
        assert_eq!(clean_identifier_name("VPN"), "VPN");
        assert_eq!(clean_identifier_name("Apple Account"), "Apple Account");
    }

    #[test]
    fn only_the_power_source_predicate_is_evaluated() {
        assert!(predicate_holds("powersource.battery == YES", true));
        assert!(!predicate_holds("powersource.battery == YES", false));
        assert!(predicate_holds("powersource.battery != YES", false));
        assert!(predicate_holds("device.touchID == YES", false), "assumed present");
        assert!(!predicate_holds("device.touchID != YES", false));
        assert!(predicate_holds("", false));
    }

    #[test]
    fn links_carry_the_pane_and_anchor() {
        let pane = SettingsPane {
            id: "com.apple.Displays-Settings.extension".into(),
            name: "Displays".into(),
            in_sidebar: true,
            items: vec![],
        };
        let item = SettingsItem {
            anchor: "nightShiftSection".into(),
            title: "Night Shift options".into(),
            keywords: vec![],
        };
        assert_eq!(pane.url(), "x-apple.systempreferences:com.apple.Displays-Settings.extension");
        assert_eq!(
            pane.item_url(&item),
            "x-apple.systempreferences:com.apple.Displays-Settings.extension?nightShiftSection"
        );
    }

    #[test]
    fn a_missing_directory_has_no_panes() {
        assert!(discover_settings_in(&[PathBuf::from("/nope/does/not/exist")], false).is_empty());
    }

    // The tests below read this Mac's real System Settings.

    #[test]
    fn finds_the_real_panes_with_their_shown_names() {
        let panes = discover_settings();
        assert!(panes.len() >= 45, "only {} panes", panes.len());
        let names: Vec<&str> = panes.iter().map(|p| p.name.as_str()).collect();
        for expected in ["Displays", "Privacy & Security", "Bluetooth", "Date & Time", "Menu Bar"] {
            assert!(names.contains(&expected), "missing {expected}: {names:?}");
        }
        assert!(
            !names.iter().any(|n| n.ends_with("Extension") || n.ends_with(".appex")),
            "an identifier leaked into a name: {names:?}"
        );
    }

    #[test]
    fn pane_ids_are_unique() {
        let panes = discover_settings();
        let ids: HashSet<&str> = panes.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids.len(), panes.len());
    }

    #[test]
    fn night_shift_and_camera_privacy_are_findable_sections() {
        let panes = discover_settings();
        let find = |anchor: &str| {
            panes.iter().find_map(|pane| {
                pane.items.iter().find(|item| item.anchor == anchor).map(|item| pane.item_url(item))
            })
        };
        assert_eq!(
            find("nightShiftSection").as_deref(),
            Some("x-apple.systempreferences:com.apple.Displays-Settings.extension?nightShiftSection")
        );
        assert!(find("Privacy_Camera").is_some(), "camera privacy section missing");
    }
}
