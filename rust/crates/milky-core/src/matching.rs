//! Deciding whether, and how well, a query matches a candidate's name.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// How a query matched, weakest to strongest.
///
/// Declaration order defines the ordering, so `Exact` compares greater than
/// `Prefix`, and so on. Ordering variants rather than assigning numbers keeps
/// us from inventing precision we do not have; scores come later, when these
/// become features alongside recency and usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    /// The query's characters appear in order, but not adjacently
    Subsequence,
    /// The query appears somewhere in the name.
    Substring,
    /// The query matches the leading initials of the name's words
    Acronym,
    /// Some word in the name starts with the query.
    WordPrefix,
    /// The name starts with the query.
    Prefix,
    /// The name is the query.
    Exact,
}

/// How `query` matches `name`, or `None` if it does not.
///
/// `query` must already be normalized by [`crate::normalize_query`]. Builds a
/// [`NameKey`] on every call; code matching the same name repeatedly should
/// build the key once and call [`match_key`].
pub fn match_kind(name: &str, query: &str) -> Option<MatchKind> {
    match_key(&NameKey::new(name), query)
}

/// Everything matching needs from a candidate's name, derived once.
///
/// Folding accents and splitting words costs about 0.4 us per name, which adds
/// up across every candidate on every keystroke, and names do not change
/// between keystrokes.
#[derive(Debug, Clone, PartialEq)]
pub struct NameKey {
    /// The whole name, normalized: `"Café Bar"` -> `"cafe bar"`.
    folded: String,
    /// Folded words: whitespace-separated words, plus their camelCase parts
    /// when there are several. `"ColorSync Utility"` ->
    /// `["color", "sync", "colorsync", "utility"]`.
    words: Vec<String>,
    /// First letter of each whitespace-separated word: `"ColorSync Utility"` -> `"cu"`.
    initials: String,
    /// First letter of each camelCase-aware word: `"ColorSync Utility"` -> `"csu"`.
    camel_initials: String,
}

impl NameKey {
    pub fn new(name: &str) -> Self {
        let mut words = Vec::new();
        let mut camel_initials = String::new();
        for token in name.split_whitespace() {
            let parts = camel_parts(token);
            for part in &parts {
                let folded = fold(part);
                if let Some(first) = folded.chars().next() {
                    camel_initials.push(first);
                }
                if parts.len() > 1 && !folded.is_empty() {
                    words.push(folded);
                }
            }
            let folded = fold(token);
            if !folded.is_empty() {
                words.push(folded);
            }
        }
        Self {
            folded: crate::normalize_query(name),
            words,
            initials: word_initials(name),
            camel_initials,
        }
    }
}

/// How `query` matches a precomputed name key, or `None`. Strongest kind wins.
pub fn match_key(key: &NameKey, query: &str) -> Option<MatchKind> {
    if query.is_empty() {
        return None;
    }
    if key.folded == query {
        return Some(MatchKind::Exact);
    }
    if key.folded.starts_with(query) {
        return Some(MatchKind::Prefix);
    }
    if all_words_start(&key.words, query) {
        return Some(MatchKind::WordPrefix);
    }
    // Either reading of the initials counts, so camelCase splitting never
    // breaks an acronym that worked on whole words: "ColorSync Utility"
    // answers to both "cu" and "csu".
    if key.initials.starts_with(query) || key.camel_initials.starts_with(query) {
        return Some(MatchKind::Acronym);
    }
    if key.folded.contains(query) {
        return Some(MatchKind::Substring);
    }
    if is_subsequence(&key.folded, query) {
        return Some(MatchKind::Subsequence);
    }
    None
}

/// How a query matched an entry's keywords, weaker to stronger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeywordMatch {
    /// Every query word starts some keyword, but at least one is unfinished:
    /// `"camer"`.
    Partial,
    /// Every query word is a whole keyword: `"camera"`, `"mac address"`.
    Complete,
}

/// How `query` matches a set of keyword words, or `None`.
///
/// `words` come from [`words_of`]; `query` must already be normalized by
/// [`crate::normalize_query`]. Query words may match in any order.
pub fn keyword_match(words: &[String], query: &str) -> Option<KeywordMatch> {
    if query.is_empty() {
        return None;
    }
    if query.split(' ').all(|part| words.iter().any(|word| word == part)) {
        return Some(KeywordMatch::Complete);
    }
    if all_words_start(words, query) {
        return Some(KeywordMatch::Partial);
    }
    None
}

/// Does every word of `query` start at least one of `words`?
///
/// `"mac addr"` against `["wifi", "mac", "address"]` is true, in any order.
/// A one-word query asks whether any word starts with it.
pub fn all_words_start(words: &[String], query: &str) -> bool {
    query.split(' ').all(|part| words.iter().any(|word| word.starts_with(part)))
}

/// The folded words of `text`, as matching sees them, camelCase parts
/// included: `"AirDrop"` -> `["air", "drop", "airdrop"]`.
pub fn words_of(text: &str) -> Vec<String> {
    NameKey::new(text).words
}

/// Is `needle` a subsequence of `haystack` — all its characters present, in
/// order, not necessarily adjacent?
///
/// Compares characters as given; the caller lowercases. An empty needle is
/// trivially a subsequence, so callers must reject empty queries first.
pub fn is_subsequence(haystack: &str, needle: &str) -> bool {
    let mut needle_chars = needle.chars();
    let mut next_needle = needle_chars.next();
    for ch in haystack.chars() {
        if Some(ch) == next_needle {
            next_needle = needle_chars.next();
        }
    }
    next_needle.is_none()
}

/// The first character of each whitespace-separated word.
///
/// `"Claude Code URL Handler"` becomes `"ccuh"`. Characters are taken as given,
/// so the caller lowercases.
pub fn word_initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|word| fold(word).chars().next())
        .collect()
}

/// Split one whitespace-free token at camelCase boundaries.
///
/// A boundary falls before an uppercase letter that follows a lowercase one
/// (`ColorSync` -> `Color`, `Sync`; `iMovie` -> `i`, `Movie`), and before the
/// last capital of an uppercase run followed by lowercase (`PDFExpert` ->
/// `PDF`, `Expert`). All-caps tokens stay whole (`VLC`).
pub(crate) fn camel_parts(token: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = token.char_indices().collect();
    let mut parts = Vec::new();
    let mut start = 0;
    for i in 1..chars.len() {
        let (index, current) = chars[i];
        let previous = chars[i - 1].1;
        let next = chars.get(i + 1).map(|&(_, c)| c);
        let lower_to_upper = previous.is_lowercase() && current.is_uppercase();
        let acronym_ends = previous.is_uppercase()
            && current.is_uppercase()
            && next.is_some_and(char::is_lowercase);
        if lower_to_upper || acronym_ends {
            parts.push(&token[start..index]);
            start = index;
        }
    }
    parts.push(&token[start..]);
    parts
}

/// Fold text for comparison: compatibility decomposition (NFKD), then drop
/// combining accent marks, invisible formatting characters, and dashes, then
/// lowercase. Dropping dashes makes `wifi`, `wi-fi`, and Apple's `Wi‑Fi` (a
/// non-breaking hyphen) the same text.
///
/// `"Café"` -> `"cafe"`, `"ﬁle"` -> `"file"`, `"Ｆｕｌｌ"` -> `"full"`. Not full
/// Unicode case folding: `"ß"` stays `"ß"`. Lenient for some scripts by design,
/// since both query and name are folded the same way: Japanese voiced marks are
/// dropped, so `"が"` matches `"か"`.
pub fn fold(text: &str) -> String {
    text.nfkd()
        .filter(|&c| !is_combining_mark(c) && !is_invisible_format(c) && !is_dash(c))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Hyphens and dashes, which people type or omit inconsistently: `-`, the
/// Unicode hyphens and dashes U+2010 to U+2015 (Apple writes "Wi‑Fi" with
/// U+2011), and the minus sign.
fn is_dash(c: char) -> bool {
    matches!(c, '-' | '\u{2010}'..='\u{2015}' | '\u{2212}')
}

/// Zero-width and bidirectional formatting characters: present in some app
/// names (WhatsApp's bundle name starts with a left-to-right mark), never
/// typed on purpose.
fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_query_word_must_start_a_word() {
        assert_eq!(match_kind("Wi-Fi MAC Address", "mac address"), Some(MatchKind::WordPrefix));
        assert_eq!(match_kind("Wi-Fi MAC Address", "address mac"), Some(MatchKind::WordPrefix));
        assert_eq!(match_kind("Wi-Fi MAC Address", "mac addr"), Some(MatchKind::WordPrefix));
        assert_eq!(match_kind("Turn off display when inactive", "turn display"), Some(MatchKind::WordPrefix));
        assert_ne!(match_kind("Wi-Fi MAC Address", "mac phone"), Some(MatchKind::WordPrefix));
    }

    #[test]
    fn whole_keywords_are_complete_matches() {
        let words: Vec<String> = ["MAC address", "Dark Mode"].iter().flat_map(|k| words_of(k)).collect();
        assert_eq!(keyword_match(&words, "mac address"), Some(KeywordMatch::Complete));
        assert_eq!(keyword_match(&words, "dark"), Some(KeywordMatch::Complete));
        assert_eq!(keyword_match(&words, "mode dark"), Some(KeywordMatch::Complete));
    }

    #[test]
    fn unfinished_keywords_are_partial_matches() {
        let words = words_of("camera");
        assert_eq!(keyword_match(&words, "camer"), Some(KeywordMatch::Partial));
        assert!(KeywordMatch::Complete > KeywordMatch::Partial);
    }

    #[test]
    fn every_query_word_must_be_a_keyword() {
        let words: Vec<String> = ["camera", "privacy"].iter().flat_map(|k| words_of(k)).collect();
        assert_eq!(keyword_match(&words, "camera microphone"), None);
        assert_eq!(keyword_match(&words, "amera"), None, "keywords match from the start of a word");
        assert_eq!(keyword_match(&words, ""), None);
    }

    #[test]
    fn keyword_words_are_folded_and_split() {
        assert_eq!(words_of("AirDrop"), vec!["air", "drop", "airdrop"]);
        assert_eq!(words_of("Wi‑Fi"), vec!["wifi"]);
        assert_eq!(keyword_match(&words_of("Wi‑Fi"), "wifi"), Some(KeywordMatch::Complete));
    }

    #[test]
    fn exact_beats_everything() {
        assert_eq!(match_kind("Safari", "safari"), Some(MatchKind::Exact));
    }

    #[test]
    fn prefix_of_the_whole_name() {
        assert_eq!(match_kind("Terminal", "term"), Some(MatchKind::Prefix));
    }

    #[test]
    fn prefix_of_a_later_word() {
        assert_eq!(
            match_kind("Visual Studio Code", "code"),
            Some(MatchKind::WordPrefix)
        );
        assert_eq!(
            match_kind("Disk Utility", "util"),
            Some(MatchKind::WordPrefix)
        );
    }

    #[test]
    fn substring_is_the_weakest_match() {
        assert_eq!(
            match_kind("Visual Studio Code", "ode"),
            Some(MatchKind::Substring)
        );
    }

    #[test]
    fn no_match_is_none() {
        assert_eq!(match_kind("Terminal", "photoshop"), None);
    }

    #[test]
    fn empty_query_matches_nothing() {
        assert_eq!(match_kind("Terminal", ""), None);
    }

    #[test]
    fn ordering_runs_weakest_to_strongest() {
        assert!(MatchKind::Exact > MatchKind::Prefix);
        assert!(MatchKind::Prefix > MatchKind::WordPrefix);
        assert!(MatchKind::WordPrefix > MatchKind::Substring);
    }

    #[test]
    fn a_word_prefix_is_not_downgraded_to_substring() {
        // "Utility" starts with "util", so this must not fall through to Substring.
        assert_eq!(
            match_kind("Disk Utility", "util"),
            Some(MatchKind::WordPrefix)
        );
    }

    #[test]
    fn subsequence_matches_scattered_characters() {
        assert!(is_subsequence("visual studio code", "vsc"));
        assert!(is_subsequence("activity monitor", "actmon"));
    }

    #[test]
    fn subsequence_requires_correct_order() {
        assert!(!is_subsequence("visual studio code", "csv"));
    }

    #[test]
    fn subsequence_needs_every_character() {
        assert!(!is_subsequence("visual studio code", "vscx"));
    }

    #[test]
    fn a_contiguous_match_is_also_a_subsequence() {
        assert!(is_subsequence("terminal", "term"));
    }

    #[test]
    fn empty_needle_is_trivially_a_subsequence() {
        assert!(is_subsequence("terminal", ""));
    }

    #[test]
    fn nothing_is_a_subsequence_of_an_empty_haystack() {
        assert!(!is_subsequence("", "a"));
    }

    #[test]
    fn subsequence_is_the_weakest_kind() {
        // A genuine subsequence-only case: initials are "c", it contains no
        // "ccu", but the letters do appear in order.
        assert_eq!(
            match_kind("Calculator", "ccu"),
            Some(MatchKind::Subsequence)
        );
        assert!(MatchKind::Substring > MatchKind::Subsequence);
    }

    #[test]
    fn stronger_kinds_still_win_over_subsequence() {
        // "term" is a subsequence of "Terminal" too, but it is a prefix first.
        assert_eq!(match_kind("Terminal", "term"), Some(MatchKind::Prefix));
    }

    #[test]
    fn initials_are_the_first_letter_of_each_word() {
        assert_eq!(word_initials("claude code url handler"), "ccuh");
        assert_eq!(word_initials("terminal"), "t");
        assert_eq!(word_initials(""), "");
    }

    #[test]
    fn initials_ignore_extra_whitespace() {
        assert_eq!(word_initials("  visual   studio  code "), "vsc");
    }

    #[test]
    fn acronym_beats_a_scattered_subsequence() {
        // Initials are "cu", so this is not an acronym match — but c-o-l-o-r-
        // s-y-n-C ... U-tility does contain c, c, u in order, so it still
        // matches, just at the weakest kind.
        assert_eq!(
            match_kind("ColorSync Utility", "ccu"),
            Some(MatchKind::Subsequence)
        );
        assert_eq!(
            match_kind("Claude Code URL Handler", "ccu"),
            Some(MatchKind::Acronym)
        );
        assert_eq!(
            match_kind("Calculator", "ccu"),
            Some(MatchKind::Subsequence)
        );
    }

    #[test]
    fn acronym_must_match_consecutive_words_from_the_start() {
        // "ch" would be Claude ... Handler, skipping two words. Not an acronym.
        assert_ne!(
            match_kind("Claude Code URL Handler", "ch"),
            Some(MatchKind::Acronym)
        );
    }

    #[test]
    fn stronger_kinds_still_win_over_acronym() {
        // "c" is the initial of "Calculator", but it is a prefix first.
        assert_eq!(match_kind("Calculator", "c"), Some(MatchKind::Prefix));
    }

    #[test]
    fn acronym_ranks_above_substring_below_word_prefix() {
        assert!(MatchKind::Acronym > MatchKind::Substring);
        assert!(MatchKind::WordPrefix > MatchKind::Acronym);
    }

    #[test]
    fn camel_case_splits_where_capitals_begin_words() {
        assert_eq!(camel_parts("ColorSync"), vec!["Color", "Sync"]);
        assert_eq!(camel_parts("iMovie"), vec!["i", "Movie"]);
        assert_eq!(camel_parts("PDFExpert"), vec!["PDF", "Expert"]);
        assert_eq!(camel_parts("VLC"), vec!["VLC"]);
        assert_eq!(camel_parts("zoom.us"), vec!["zoom.us"]);
    }

    #[test]
    fn camel_case_initials_form_acronyms() {
        assert_eq!(
            match_kind("ColorSync Utility", "csu"),
            Some(MatchKind::Acronym)
        );
        assert_eq!(
            match_kind("ColorSync Utility", "cs"),
            Some(MatchKind::Acronym)
        );
    }

    #[test]
    fn whole_word_acronyms_still_work_alongside_camel_case() {
        assert_eq!(
            match_kind("ColorSync Utility", "cu"),
            Some(MatchKind::Acronym)
        );
    }

    #[test]
    fn camel_case_parts_are_words() {
        assert_eq!(
            match_kind("ColorSync Utility", "sync"),
            Some(MatchKind::WordPrefix)
        );
        assert_eq!(match_kind("iMovie", "movie"), Some(MatchKind::WordPrefix));
    }

    #[test]
    fn accents_do_not_block_a_match() {
        assert_eq!(match_kind("Café Bar", "cafe"), Some(MatchKind::Prefix));
        assert_eq!(match_kind("Cafe Bar", "cafe"), Some(MatchKind::Prefix));
    }

    #[test]
    fn match_key_agrees_with_match_kind() {
        let key = NameKey::new("Visual Studio Code");
        for query in [
            "visual studio code",
            "vis",
            "code",
            "vsc",
            "dio",
            "vsd",
            "zzz",
        ] {
            assert_eq!(
                match_key(&key, query),
                match_kind("Visual Studio Code", query),
                "{query}"
            );
        }
    }

    #[test]
    fn dashes_do_not_split_or_block_a_match() {
        // Apple's "Wi‑Fi" uses U+2011, a non-breaking hyphen.
        let apple = "Wi\u{2011}Fi";
        assert_eq!(match_kind(apple, "wifi"), Some(MatchKind::Exact));
        assert_eq!(match_kind(apple, &crate::normalize_query("wi-fi")), Some(MatchKind::Exact));
        assert_eq!(match_kind("Wi-Fi MAC Address", "wifi"), Some(MatchKind::Prefix));
    }

    #[test]
    fn every_dash_form_folds_away() {
        for dash in ["-", "\u{2010}", "\u{2011}", "\u{2013}", "\u{2014}", "\u{2212}", "\u{FF0D}"] {
            assert_eq!(fold(&format!("e{dash}mail")), "email", "{dash:?}");
        }
    }
}
