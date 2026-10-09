//! Well-known places: the folders Finder lists in its sidebar and Go menu,
//! including ones outside the home folder (Applications) and hidden ones
//! (Trash, iCloud Drive). They are results in their own right, so `downloads`
//! finds the Downloads folder first.

use std::path::{Path, PathBuf};

/// Where a place lives, independent of whose Mac this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    /// The home folder itself.
    Home,
    /// A path inside the home folder: `InHome("Downloads")`.
    InHome(&'static str),
    /// An absolute path: `Absolute("/Applications")`.
    Absolute(&'static str),
}

impl Location {
    /// The path this location names on a Mac whose home folder is `home`.
    pub fn resolve(self, home: &Path) -> PathBuf {
        match self {
            Location::Home => home.to_path_buf(),
            Location::InHome(relative) => home.join(relative),
            Location::Absolute(path) => PathBuf::from(path),
        }
    }
}

/// A well-known place: where it is, and names it answers to beyond the one
/// Finder shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaceSpec {
    pub location: Location,
    pub aliases: &'static [&'static str],
}

/// Every well-known place. `~/Applications` and `/System/Applications` are left
/// out: Finder's Applications already shows their apps, and three results named
/// "Applications" would be noise.
pub const PLACES: &[PlaceSpec] = &[
    PlaceSpec { location: Location::Home, aliases: &["Home"] },
    PlaceSpec { location: Location::InHome("Desktop"), aliases: &[] },
    PlaceSpec { location: Location::InHome("Documents"), aliases: &[] },
    PlaceSpec { location: Location::InHome("Downloads"), aliases: &[] },
    PlaceSpec { location: Location::InHome("Movies"), aliases: &[] },
    PlaceSpec { location: Location::InHome("Music"), aliases: &[] },
    PlaceSpec { location: Location::InHome("Pictures"), aliases: &[] },
    PlaceSpec { location: Location::InHome("Public"), aliases: &[] },
    PlaceSpec { location: Location::InHome("Library"), aliases: &[] },
    PlaceSpec { location: Location::InHome(".Trash"), aliases: &["Bin", "Recycle Bin"] },
    PlaceSpec {
        location: Location::InHome(crate::candidate::ICLOUD_DRIVE),
        aliases: &["iCloud"],
    },
    PlaceSpec { location: Location::Absolute("/Applications"), aliases: &[] },
    PlaceSpec { location: Location::Absolute("/Applications/Utilities"), aliases: &[] },
];

/// A well-known place that exists on this Mac.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub path: PathBuf,
    /// The name Finder shows: "Trash", "iCloud Drive", the user's short name
    /// for the home folder. Localized on Macs set to other languages.
    pub name: String,
    pub aliases: Vec<String>,
}

/// `spec` on this Mac, or `None` if its folder does not exist. The name comes
/// from LaunchServices, falling back to the folder's own name.
pub fn place(spec: &PlaceSpec, home: &Path) -> Option<Place> {
    let path = spec.location.resolve(home);
    if !path.is_dir() {
        return None;
    }
    let name = crate::launch_services::localized_name(&path).unwrap_or_else(|| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let aliases = spec.aliases.iter().map(|alias| alias.to_string()).collect();
    Some(Place { path, name, aliases })
}

/// Every place in [`PLACES`] that exists on this Mac, in table order.
pub fn discover_places(home: &Path) -> Vec<Place> {
    PLACES.iter().filter_map(|spec| place(spec, home)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn locations_resolve_against_the_home_folder() {
        let home = Path::new("/Users/someone");
        assert_eq!(Location::Home.resolve(home), PathBuf::from("/Users/someone"));
        assert_eq!(Location::InHome("Downloads").resolve(home), PathBuf::from("/Users/someone/Downloads"));
        assert_eq!(Location::Absolute("/Applications").resolve(home), PathBuf::from("/Applications"));
    }

    #[test]
    fn the_home_folder_has_no_trailing_slash() {
        // `home.join("")` would give "/Users/someone/", a different string, and
        // so a different result ID from the same folder found by a file walk.
        let home = Path::new("/Users/someone");
        assert_eq!(Location::Home.resolve(home).to_str(), Some("/Users/someone"));
    }

    #[test]
    fn missing_places_are_left_out() {
        let home = std::env::temp_dir().join(format!("milky-places-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("Downloads")).unwrap();
        let paths: Vec<PathBuf> = discover_places(&home).into_iter().map(|place| place.path).collect();
        assert!(paths.contains(&home.join("Downloads")));
        assert!(!paths.contains(&home.join("Movies")), "no Movies folder here");
        assert!(paths.contains(&PathBuf::from("/Applications")));
    }

    #[test]
    fn every_place_is_a_different_folder() {
        let home = Path::new("/Users/someone");
        let paths: HashSet<PathBuf> = PLACES.iter().map(|spec| spec.location.resolve(home)).collect();
        assert_eq!(paths.len(), PLACES.len());
    }

    #[test]
    fn real_places_are_named_by_finder() {
        let home = std::env::home_dir().unwrap();
        let places = discover_places(&home);
        let named = |path: PathBuf| places.iter().find(|place| place.path == path).map(|place| place.name.as_str());
        assert_eq!(named(home.join(".Trash")), Some("Trash"));
        assert_eq!(named(home.join("Downloads")), Some("Downloads"));
        assert_eq!(named(PathBuf::from("/Applications/Utilities")), Some("Utilities"));
        let trash = places.iter().find(|place| place.name == "Trash").unwrap();
        assert_eq!(trash.aliases, vec!["Bin", "Recycle Bin"]);
    }
}