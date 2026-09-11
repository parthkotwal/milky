//! Deciding whether, and how well, a query matches a candidate's name.

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
/// Both sides are compared case-insensitively. `query` is expected to be
/// normalized already; `name` is raw, as discovered.
pub fn match_kind(name: &str, query: &str) -> Option<MatchKind> {
    if query.is_empty() {
        return None;
    }

    let name_lower = name.to_lowercase();

    // Exact
    if name_lower == query {
        return Some(MatchKind::Exact);
    }

    // Prefix
    if name_lower.starts_with(query) {
        return Some(MatchKind::Prefix);
    }

    // WordPrefix
    if name_lower
        .split_whitespace()
        .any(|word| word.starts_with(query))
    {
        return Some(MatchKind::WordPrefix);
    }

    // Acronym
    if word_initials(&name_lower).starts_with(query) {
        return Some(MatchKind::Acronym);
    }

    // Substring
    if name_lower.contains(query) {
        return Some(MatchKind::Substring);
    }

    // Subsequence
    if is_subsequence(&name_lower, query) {
        return Some(MatchKind::Subsequence);
    }

    None
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
        .filter_map(|word| word.chars().next())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(match_kind("Visual Studio Code", "code"), Some(MatchKind::WordPrefix));
        assert_eq!(match_kind("Disk Utility", "util"), Some(MatchKind::WordPrefix));
    }

    #[test]
    fn substring_is_the_weakest_match() {
        assert_eq!(match_kind("Visual Studio Code", "ode"), Some(MatchKind::Substring));
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
        assert_eq!(match_kind("Disk Utility", "util"), Some(MatchKind::WordPrefix));
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
        assert_eq!(match_kind("Calculator", "ccu"), Some(MatchKind::Subsequence));
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
        assert_eq!(match_kind("Calculator", "ccu"), Some(MatchKind::Subsequence));
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
}