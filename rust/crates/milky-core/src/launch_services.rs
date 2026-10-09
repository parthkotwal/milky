//! What macOS LaunchServices knows about a path: the name Finder shows, and
//! whether a folder is a package.
//!
//! Both come from one CoreFoundation call, `CFURLCopyResourcePropertyForKey`,
//! so the unsafe code lives here once. Off macOS every answer is "unknown".

use std::path::Path;

/// The name Finder shows for `path`: "Find My.app" for `FindMy.app`,
/// "iCloud Drive" for `~/Library/Mobile Documents/com~apple~CloudDocs`,
/// "Trash" for `~/.Trash`. Extensions Finder hides are kept, so callers strip
/// what they need. `None` if LaunchServices cannot answer or the name is empty.
pub fn localized_name(path: &Path) -> Option<String> {
    let name = imp::localized_name(path)?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Whether Finder shows this folder as a single item: an app, a `.pages`
/// document, a photo library. `false` if LaunchServices cannot answer.
pub fn is_package(path: &Path) -> bool {
    imp::is_package(path)
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;
    use std::path::Path;

    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::string::CFString;
    use core_foundation::url::CFURL;
    use core_foundation_sys::base::{Boolean, CFTypeRef};
    use core_foundation_sys::error::CFErrorRef;
    use core_foundation_sys::string::CFStringRef;
    use core_foundation_sys::url::{CFURLRef, kCFURLIsPackageKey, kCFURLLocalizedNameKey};

    // Not exported by core-foundation-sys, which only binds the plural form.
    unsafe extern "C" {
        fn CFURLCopyResourcePropertyForKey(
            url: CFURLRef,
            key: CFStringRef,
            property_value_type_ref_ptr: *mut c_void,
            error: *mut CFErrorRef,
        ) -> Boolean;
    }

    /// One resource property of the file at `path`, or `None`.
    fn property(path: &Path, key: CFStringRef) -> Option<CFType> {
        let url = CFURL::from_path(path, true)?;
        let mut value: CFTypeRef = std::ptr::null();
        // SAFETY: `url` is a live CFURL for the call's duration, `key` is one
        // of the framework's constant keys, and `value` is a valid out-pointer.
        // A null error pointer is permitted.
        let ok = unsafe {
            CFURLCopyResourcePropertyForKey(
                url.as_concrete_TypeRef(),
                key,
                &mut value as *mut CFTypeRef as *mut c_void,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || value.is_null() {
            return None;
        }
        // SAFETY: a "Copy" function returns a +1 reference we now own; the
        // wrapper releases it when dropped.
        Some(unsafe { CFType::wrap_under_create_rule(value) })
    }

    pub fn localized_name(path: &Path) -> Option<String> {
        // SAFETY: the constant is defined by CoreFoundation for the process lifetime.
        let key = unsafe { kCFURLLocalizedNameKey };
        Some(property(path, key)?.downcast::<CFString>()?.to_string())
    }

    pub fn is_package(path: &Path) -> bool {
        // SAFETY: the constant is defined by CoreFoundation for the process lifetime.
        let key = unsafe { kCFURLIsPackageKey };
        property(path, key)
            .and_then(|value| value.downcast::<CFBoolean>())
            .is_some_and(bool::from)
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::path::Path;

    pub fn localized_name(_path: &Path) -> Option<String> {
        None
    }

    pub fn is_package(_path: &Path) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_finder() {
        assert_eq!(
            localized_name(Path::new("/Applications")).as_deref(),
            Some("Applications")
        );
        let home = std::env::home_dir().unwrap();
        assert_eq!(
            localized_name(&home.join(".Trash")).as_deref(),
            Some("Trash")
        );
    }

    #[test]
    fn app_names_keep_their_extension() {
        assert_eq!(
            localized_name(Path::new("/System/Applications/Calendar.app")).as_deref(),
            Some("Calendar.app")
        );
    }

    #[test]
    fn a_missing_path_has_no_name() {
        assert_eq!(localized_name(Path::new("/nonexistent/milky")), None);
    }

    #[test]
    fn apps_are_packages_and_plain_folders_are_not() {
        assert!(is_package(Path::new("/System/Applications/Calendar.app")));
        assert!(!is_package(Path::new("/Applications")));
        assert!(!is_package(Path::new("/nonexistent/milky.app")));
    }
}
