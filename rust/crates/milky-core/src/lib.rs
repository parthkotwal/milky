//! Milky's search engine core.

pub mod api;
pub mod apps;
pub mod engine;
pub mod events;
pub mod matching;
pub mod search;
pub mod storage;
pub mod settings;
pub mod usage;
pub mod candidate;
pub mod files;
pub mod launch_services;
pub mod places;

/// Normalize text for matching: fold case, accents, compatibility forms, and
/// invisible formatting marks (see [`matching::fold`]), trim, and collapse runs
/// of whitespace to single spaces. Applied to queries and to candidate names
/// alike, so both sides are compared in the same form.
pub fn normalize_query(input: &str) -> String {
    matching::fold(input)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercases() {
        assert_eq!(normalize_query("Terminal"), "terminal");
    }

    #[test]
    fn trims_surrounding_whitespaces() {
        assert_eq!(normalize_query("  terminal  "), "terminal");
    }

    #[test]
    fn collapses_internal_whitespace() {
        assert_eq!(normalize_query("seiko    watch"), "seiko watch");
    }

    #[test]
    fn handles_tabs_and_newlines() {
        assert_eq!(normalize_query("seiko\t\nwatch"), "seiko watch");
    }

    #[test]
    fn empty_input_gives_empty_output() {
        assert_eq!(normalize_query("   "), "");
    }

    #[test]
    fn folds_accents() {
        assert_eq!(normalize_query("  Café  Crème "), "cafe creme");
        assert_eq!(normalize_query("Ångström"), "angstrom");
    }

    #[test]
    fn strips_invisible_formatting_marks() {
        // WhatsApp's bundle name starts with an invisible left-to-right mark.
        assert_eq!(normalize_query("\u{200E}WhatsApp"), "whatsapp");
    }

    #[test]
    fn folds_compatibility_forms() {
        // A single "ﬁ" ligature character, and full-width letters.
        assert_eq!(normalize_query("ﬁle"), "file");
        assert_eq!(normalize_query("Ｆｕｌｌ"), "full");
    }
}
