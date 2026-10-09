//! Milky's search engine core.

pub mod api;
pub mod apps;
pub mod candidate;
pub mod engine;
pub mod events;
pub mod files;
pub mod launch_services;
pub mod matching;
pub mod places;
pub mod search;
pub mod settings;
pub mod storage;
pub mod usage;

/// Normalize text for matching: fold case, accents, compatibility forms,
/// invisible formatting marks, and dashes (see [`matching::fold`]); treat
/// other punctuation and symbols as spaces; trim; and collapse runs of spaces.
/// Applied to queries and to candidate names alike, so both sides are compared
/// in the same form, and split into words the way names are: `google.pdf` ->
/// `google pdf`, `desktop/customized` -> `desktop customized`.
pub fn normalize_query(input: &str) -> String {
    matching::fold(input)
        .split(|c: char| c.is_whitespace() || matching::is_separator(c))
        .filter(|word| !word.is_empty())
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
    fn punctuation_separates_words_like_spaces() {
        assert_eq!(normalize_query("Parth Kotwal Google.pdf"), "parth kotwal google pdf");
        assert_eq!(normalize_query("~/Desktop/Customized/a_b.txt"), "desktop customized a b txt");
        assert_eq!(normalize_query("Read & Speak (Live)"), "read speak live");
        assert_eq!(normalize_query("wi-fi"), "wifi", "dashes still join");
        assert_eq!(normalize_query("../"), "");
    }

    #[test]
    fn folds_compatibility_forms() {
        // A single "ﬁ" ligature character, and full-width letters.
        assert_eq!(normalize_query("ﬁle"), "file");
        assert_eq!(normalize_query("Ｆｕｌｌ"), "full");
    }
}
