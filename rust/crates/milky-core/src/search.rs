//! Turning a query and a set of candidates into an ordered list of results.

use std::cmp::Reverse;

use crate::candidate::{Candidate, FileFacts, Kind};
use crate::matching::{KeywordMatch, MatchKind, NameKey, keyword_match, match_key};

/// How strong the evidence for a hit is, weakest to strongest; the first sort
/// key. Title kinds and keyword matches interleave: a whole keyword is better
/// evidence than initials, an unfinished one better than a substring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Subsequence,
    Substring,
    KeywordPartial,
    Acronym,
    KeywordComplete,
    /// Files and folders only: a word in the name starts with part of the
    /// query, and the rest was found in the folders above it or its
    /// extension. What a `WordPrefix` name match drops to when it needed that
    /// help (see [`file_tier`]).
    PathSupported,
    WordPrefix,
    Prefix,
    Exact,
}

impl From<MatchKind> for Tier {
    fn from(kind: MatchKind) -> Self {
        match kind {
            MatchKind::Subsequence => Tier::Subsequence,
            MatchKind::Substring => Tier::Substring,
            MatchKind::Acronym => Tier::Acronym,
            MatchKind::WordPrefix => Tier::WordPrefix,
            MatchKind::Prefix => Tier::Prefix,
            MatchKind::Exact => Tier::Exact,
        }
    }
}

/// The tier for a title match and a keyword match, or `None` if neither
/// matched. A word in the title that Apple also lists as a keyword counts as
/// much as the title starting with it.
pub fn tier(kind: Option<MatchKind>, keyword: Option<KeywordMatch>) -> Option<Tier> {
    if kind == Some(MatchKind::WordPrefix) && keyword == Some(KeywordMatch::Complete) {
        return Some(Tier::Prefix);
    }
    let from_title = kind.map(Tier::from);
    let from_keywords = keyword.map(|keyword| match keyword {
        KeywordMatch::Partial => Tier::KeywordPartial,
        KeywordMatch::Complete => Tier::KeywordComplete,
    });
    from_title.max(from_keywords)
}

/// How strongly a file or folder matches `query`, or `None` (DECISIONS
/// 2026-10-08, file matching option C).
///
/// - Every query word must start a word of the name (`name.words()`), of a
///   folder above it (`facts.folder_words`), or its extension
///   (`facts.extension`). Check the name first: a word found there counts as
///   a name word even if a folder has it too.
/// - At least one query word must come from the name. Folders and the
///   extension only support a match; they never make one alone.
/// - Score the name against just its own words, in query order, with
///   [`match_key`] and [`Tier::from`]. Only strong matches count: anything
///   below `WordPrefix` is no match (no substrings or scattered letters for
///   files).
/// - If any word needed a folder or the extension, drop one step: `Exact` ->
///   `Prefix` -> `WordPrefix` -> `PathSupported`.
///
/// `query` is normalized ([`crate::normalize_query`]): lowercase, single
/// spaces. An empty query matches nothing.
pub fn file_tier(name: &NameKey, facts: &FileFacts, query: &str) -> Option<Tier> {
    if query.is_empty() {
        return None;
    }

    let mut name_words: Vec<&str> = Vec::new();
    let mut needed_support = false;

    for word in query.split(' ') {
        if name.words().iter().any(|w| w.starts_with(word)) {
            name_words.push(word);
        } else if facts.folder_words.iter().any(|w| w.starts_with(word))
            || facts
                .extension
                .as_deref()
                .is_some_and(|ext| ext.starts_with(word))
        {
            needed_support = true;
        } else {
            return None;
        }
    }

    if name_words.is_empty() {
        return None;
    }

    let tier = Tier::from(match_key(name, &name_words.join(" "))?);
    if tier < Tier::WordPrefix {
        return None;
    }
    if !needed_support {
        return Some(tier);
    }
    Some(match tier {
        Tier::Exact => Tier::Prefix,
        Tier::Prefix => Tier::WordPrefix,
        _ => Tier::PathSupported,
    })
}

/// One candidate that matched a query. Borrows from the candidate list
/// instead of copying it, so a search allocates nothing per candidate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit<'a> {
    pub candidate: &'a Candidate,
    /// The name that matched best — shown as the result's title. The default
    /// title when only keywords or an alias matched.
    pub title: &'a str,
    /// How the title matched, if it did.
    pub kind: Option<MatchKind>,
    /// How the keywords matched, if they did.
    pub keyword: Option<KeywordMatch>,
    pub tier: Tier,
    pub usage: f64,
}

/// Candidates matching `query`, best first, capped at `limit`. One hit per
/// candidate, titled by its best-matching name (the earlier name wins a tie).
/// Sorted by tier, then usage, then prior (apps, panes, sections), then keyword
/// match, then title length, then title, then ID, so the order is fully
/// deterministic. Only apps and places match as subsequences: their names are
/// short, while on sentence-long setting titles nearly every short query is one.
///
/// `query` must already be normalized by [`crate::normalize_query`].
pub fn search<'a>(
    candidates: &'a [Candidate],
    query: &str,
    limit: usize,
    usage_of: impl Fn(&str) -> f64,
) -> Vec<Hit<'a>> {
    let mut hits: Vec<Hit<'a>> = candidates
        .iter()
        .filter_map(|candidate| {
            // Files and folders match by their own rule: strong name matches
            // only, with folders and the extension as support.
            if let Some(facts) = &candidate.file {
                let name = candidate.names.first()?;
                let tier = file_tier(&name.key, facts, query)?;
                return Some(Hit {
                    candidate,
                    title: &name.text,
                    kind: match_key(&name.key, query).filter(|kind| *kind >= MatchKind::WordPrefix),
                    keyword: None,
                    tier,
                    usage: usage_of(&candidate.id),
                });
            }
            let best = candidate
                .names
                .iter()
                .filter_map(|name| Some((match_key(&name.key, query)?, name)))
                .filter(|(kind, _)| {
                    *kind != MatchKind::Subsequence
                        || matches!(candidate.kind, Kind::App | Kind::Place)
                })
                .min_by_key(|(kind, _)| Reverse(*kind));
            let keyword = keyword_match(&candidate.keywords, query);
            let tier = tier(best.map(|(kind, _)| kind), keyword)?;
            Some(Hit {
                candidate,
                title: match best {
                    Some((_, name)) if !name.alias => &name.text,
                    _ => candidate.title(),
                },
                kind: best.map(|(kind, _)| kind),
                keyword,
                tier,
                usage: usage_of(&candidate.id),
            })
        })
        .collect();

    hits.sort_by(|a, b| {
        b.tier
            .cmp(&a.tier)
            .then_with(|| b.usage.total_cmp(&a.usage))
            .then_with(|| b.candidate.prior().cmp(&a.candidate.prior()))
            .then_with(|| b.keyword.cmp(&a.keyword))
            .then_with(|| a.title.len().cmp(&b.title.len()))
            .then_with(|| a.title.cmp(b.title))
            .then_with(|| a.candidate.id.cmp(&b.candidate.id))
    });
    hits.truncate(limit);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::apps::App;
    use crate::candidate::{Action, Kind, Name};

    fn apps_named(names: &[&str]) -> Vec<Candidate> {
        names
            .iter()
            .map(|n| {
                App::new(
                    PathBuf::from(format!("/Applications/{n}.app")),
                    n.to_string(),
                )
            })
            .filter_map(|app| crate::candidate::from_app(&app))
            .collect()
    }

    fn names_of<'a>(hits: &[Hit<'a>]) -> Vec<&'a str> {
        hits.iter().map(|hit| hit.title).collect()
    }

    #[test]
    fn stronger_match_kinds_come_first() {
        // "Terminal Pro" is a prefix match; "iTerm" is only a word-prefix match (on "Term").
        // The prefix match is also the *longer* name, so this fails if the kind
        // comparison is dropped and length alone decides.
        let apps = apps_named(&["Terminal Pro", "iTerm"]);
        let found = search(&apps, "term", 10, |_| 0.0);
        assert_eq!(names_of(&found), vec!["Terminal Pro", "iTerm"]);
    }

    #[test]
    fn shorter_names_break_ties() {
        let apps = apps_named(&["Contacts", "Code", "Console"]);
        let found = search(&apps, "co", 10, |_| 0.0);
        assert_eq!(names_of(&found), vec!["Code", "Console", "Contacts"]);
    }

    #[test]
    fn non_matches_are_excluded() {
        let apps = apps_named(&["Terminal", "Safari"]);
        let found = search(&apps, "term", 10, |_| 0.0);
        assert_eq!(names_of(&found), vec!["Terminal"]);
    }

    #[test]
    fn respects_the_limit() {
        let apps = apps_named(&["Code", "Console", "Contacts", "Compass"]);
        assert_eq!(search(&apps, "co", 2, |_| 0.0).len(), 2);
    }

    #[test]
    fn identical_names_are_kept_as_separate_results() {
        let apps: Vec<Candidate> = [
            App::new(
                PathBuf::from("/Applications/Python 3.12/IDLE.app"),
                "IDLE".into(),
            ),
            App::new(
                PathBuf::from("/Applications/Python 3.11/IDLE.app"),
                "IDLE".into(),
            ),
        ]
        .iter()
        .filter_map(crate::candidate::from_app)
        .collect();
        let found = search(&apps, "idle", 10, |_| 0.0);
        assert_eq!(found.len(), 2, "the id, not the name, is the identity");
    }

    #[test]
    fn ordering_is_stable_across_calls() {
        let apps = apps_named(&["Code", "Console", "Contacts"]);
        assert_eq!(
            search(&apps, "co", 10, |_| 0.0),
            search(&apps, "co", 10, |_| 0.0)
        );
    }

    #[test]
    fn empty_query_returns_nothing() {
        let apps = apps_named(&["Terminal"]);
        assert!(search(&apps, "", 10, |_| 0.0).is_empty());
    }

    #[test]
    fn usage_breaks_ties_within_a_kind() {
        let apps = apps_named(&["Chess", "Calendar"]);
        let found = search(&apps, "c", 10, |id| {
            if id.ends_with("Calendar.app") {
                5.0
            } else {
                0.0
            }
        });
        assert_eq!(names_of(&found), vec!["Calendar", "Chess"]);
    }

    #[test]
    fn usage_never_beats_a_stronger_kind() {
        let apps = apps_named(&["Terminal Pro", "iTerm"]);
        let found = search(&apps, "term", 10, |id| {
            if id.ends_with("iTerm.app") {
                100.0
            } else {
                0.0
            }
        });
        assert_eq!(names_of(&found), vec!["Terminal Pro", "iTerm"]);
    }

    #[test]
    fn equal_usage_falls_back_to_length_then_name() {
        let apps = apps_named(&["Contacts", "Code", "Console"]);
        let found = search(&apps, "co", 10, |_| 3.0);
        assert_eq!(names_of(&found), vec!["Code", "Console", "Contacts"]);
    }

    fn candidate(id: &str, kind: Kind, names: &[&str]) -> Candidate {
        Candidate {
            id: id.to_string(),
            kind,
            subtitle: String::new(),
            action: Action::OpenUrl(format!("test:{id}")),
            names: names.iter().map(|n| Name::new(n)).collect(),
            keywords: Vec::new(),
            file: None,
        }
    }

    fn ids<'a>(hits: &[Hit<'a>]) -> Vec<&'a str> {
        hits.iter().map(|hit| hit.candidate.id.as_str()).collect()
    }

    #[test]
    fn the_best_matching_name_becomes_the_title() {
        let candidates = [candidate(
            "settings:wifi#Advanced",
            Kind::Setting,
            &["Advanced", "Wi-Fi MAC Address"],
        )];
        let mac = search(&candidates, "mac", 10, |_| 0.0);
        assert_eq!(mac[0].title, "Wi-Fi MAC Address");
        assert_eq!(mac[0].kind, Some(MatchKind::WordPrefix));
        let adv = search(&candidates, "adv", 10, |_| 0.0);
        assert_eq!(adv[0].title, "Advanced");
        assert_eq!(adv[0].kind, Some(MatchKind::Prefix));
    }

    #[test]
    fn the_earlier_name_wins_a_tie() {
        let candidates = [candidate(
            "settings:x#a",
            Kind::Setting,
            &["Show legacy networks", "Show options"],
        )];
        assert_eq!(
            search(&candidates, "show", 10, |_| 0.0)[0].title,
            "Show legacy networks"
        );
    }

    #[test]
    fn one_destination_gives_at_most_one_hit() {
        let candidates = [candidate(
            "settings:x#a",
            Kind::Setting,
            &["Advanced", "Advanced options"],
        )];
        assert_eq!(search(&candidates, "adv", 10, |_| 0.0).len(), 1);
    }

    #[test]
    fn apps_and_settings_rank_together() {
        let candidates = [
            candidate(
                "app:/Applications/Utilities/Bluetooth File Exchange.app",
                Kind::App,
                &["Bluetooth File Exchange"],
            ),
            candidate(
                "settings:com.apple.BluetoothSettings",
                Kind::Setting,
                &["Bluetooth"],
            ),
        ];
        let hits = search(&candidates, "bluetooth", 10, |_| 0.0);
        assert_eq!(
            ids(&hits),
            vec![
                "settings:com.apple.BluetoothSettings",
                "app:/Applications/Utilities/Bluetooth File Exchange.app"
            ]
        );
    }

    #[test]
    fn usage_is_looked_up_by_id() {
        let candidates = [
            candidate("settings:a", Kind::Setting, &["Calendar sync"]),
            candidate("settings:b", Kind::Setting, &["Calendar view"]),
        ];
        let hits = search(&candidates, "cal", 10, |id| {
            if id == "settings:b" { 3.0 } else { 0.0 }
        });
        assert_eq!(ids(&hits), vec!["settings:b", "settings:a"]);
    }

    #[test]
    fn identical_titles_are_ordered_by_id() {
        let candidates = [
            candidate("settings:vpn", Kind::Setting, &["VPN options"]),
            candidate("settings:network", Kind::Setting, &["VPN options"]),
        ];
        assert_eq!(
            ids(&search(&candidates, "vpn", 10, |_| 0.0)),
            vec!["settings:network", "settings:vpn"]
        );
    }

    #[test]
    fn hits_borrow_their_candidates_instead_of_copying() {
        let candidates = [candidate("settings:a", Kind::Setting, &["Displays"])];
        let hits = search(&candidates, "dis", 10, |_| 0.0);
        assert!(std::ptr::eq(hits[0].candidate, &candidates[0]));
    }

    #[test]
    fn non_matches_limits_and_empty_queries() {
        let candidates = [
            candidate("settings:a", Kind::Setting, &["Displays"]),
            candidate("settings:b", Kind::Setting, &["Dock"]),
            candidate("settings:c", Kind::Setting, &["Music"]),
        ];
        assert_eq!(search(&candidates, "d", 10, |_| 0.0).len(), 2);
        assert_eq!(search(&candidates, "d", 1, |_| 0.0).len(), 1);
        assert!(search(&candidates, "", 10, |_| 0.0).is_empty());
    }

    fn with_keywords(mut candidate: Candidate, keywords: &[&str]) -> Candidate {
        candidate.keywords = keywords
            .iter()
            .flat_map(|k| crate::matching::words_of(k))
            .collect();
        candidate.keywords.sort();
        candidate.keywords.dedup();
        candidate
    }

    /// A file or folder candidate under `/Users/someone`, built the way the
    /// engine builds them.
    fn file(path: &str) -> Candidate {
        let entry = crate::files::FileEntry {
            path: std::path::PathBuf::from(path),
            is_folder: !path.contains('.'),
            modified: None,
        };
        crate::candidate::from_file(&entry, std::path::Path::new("/Users/someone")).unwrap()
    }

    fn file_tier_of(path: &str, query: &str) -> Option<Tier> {
        let candidate = file(path);
        file_tier(
            &candidate.names[0].key,
            candidate.file.as_ref().unwrap(),
            query,
        )
    }

    #[test]
    fn a_file_name_alone_scores_like_any_name() {
        let path = "/Users/someone/school/cse332/ex01.pdf";
        assert_eq!(
            file_tier_of(path, "ex01"),
            Some(Tier::Exact),
            "the extension is not part of the name"
        );
        assert_eq!(file_tier_of(path, "ex"), Some(Tier::Prefix));
        assert_eq!(
            file_tier_of("/Users/someone/final-report.pdf", "report"),
            Some(Tier::WordPrefix)
        );
        // Folding drops the dash, so the name reads "finalreport": both words
        // start words, but the name does not start with "final report".
        assert_eq!(
            file_tier_of("/Users/someone/final-report.pdf", "final report"),
            Some(Tier::WordPrefix)
        );
        assert_eq!(
            file_tier_of("/Users/someone/Final Report.pdf", "final report"),
            Some(Tier::Exact)
        );
    }

    #[test]
    fn files_need_a_strong_name_match() {
        let path = "/Users/someone/resume.pdf";
        assert_eq!(file_tier_of(path, "sume"), None, "no substrings");
        assert_eq!(file_tier_of(path, "rsm"), None, "no scattered letters");
        assert_eq!(
            file_tier_of("/Users/someone/final-report.pdf", "fr"),
            None,
            "no initials"
        );
        assert_eq!(file_tier_of(path, ""), None);
    }

    #[test]
    fn folders_and_the_extension_support_a_match_one_step_down() {
        let ex01 = "/Users/someone/school/cse332/ex01.pdf";
        assert_eq!(
            file_tier_of(ex01, "332 ex01"),
            Some(Tier::Prefix),
            "Exact, one step down"
        );
        assert_eq!(
            file_tier_of(ex01, "ex01 332"),
            Some(Tier::Prefix),
            "any order"
        );
        assert_eq!(
            file_tier_of(ex01, "school ex"),
            Some(Tier::WordPrefix),
            "Prefix, one step down"
        );
        assert_eq!(
            file_tier_of(ex01, "ex01 pdf"),
            Some(Tier::Prefix),
            "the extension supports too"
        );
        let notes = "/Users/someone/school/lecture-notes.md";
        assert_eq!(
            file_tier_of(notes, "school notes"),
            Some(Tier::PathSupported),
            "WordPrefix, one step down"
        );
    }

    #[test]
    fn folders_and_the_extension_never_match_alone() {
        let ex01 = "/Users/someone/school/cse332/ex01.pdf";
        assert_eq!(file_tier_of(ex01, "cse332"), None);
        assert_eq!(file_tier_of(ex01, "pdf"), None);
        assert_eq!(file_tier_of(ex01, "school pdf"), None);
        assert_eq!(
            file_tier_of(ex01, "ex01 math"),
            None,
            "every word must be found somewhere"
        );
    }

    #[test]
    fn a_word_in_the_name_counts_as_the_name_even_if_a_folder_has_it() {
        assert_eq!(
            file_tier_of("/Users/someone/notes/notes.txt", "notes"),
            Some(Tier::Exact)
        );
    }

    #[test]
    fn folders_match_by_their_whole_name() {
        assert_eq!(
            file_tier_of("/Users/someone/school/cse332", "cse332"),
            Some(Tier::Exact)
        );
        assert_eq!(
            file_tier_of("/Users/someone/school/cse332", "school 332"),
            Some(Tier::PathSupported)
        );
    }

    #[test]
    fn path_supported_sits_just_below_a_name_word_match() {
        assert!(Tier::WordPrefix > Tier::PathSupported);
        assert!(Tier::PathSupported > Tier::KeywordComplete);
    }

    #[test]
    fn the_file_named_for_the_query_beats_one_that_only_contains_it() {
        // Option C's deciding case: "ex01" *is* the first file's name, found in
        // the cse332 folder; the second only contains both words.
        let candidates = [
            file("/Users/someone/ex01-332-notes.txt"),
            file("/Users/someone/school/cse332/ex01.pdf"),
        ];
        let hits = search(&candidates, "332 ex01", 10, |_| 0.0);
        assert_eq!(
            ids(&hits),
            vec![
                "file:/Users/someone/school/cse332/ex01.pdf",
                "file:/Users/someone/ex01-332-notes.txt"
            ]
        );
        assert_eq!(hits[0].title, "ex01.pdf");
    }

    #[test]
    fn loose_matches_never_bring_in_files() {
        let candidates = [
            file("/Users/someone/resume.pdf"),
            file("/Users/someone/projects"),
        ];
        assert!(search(&candidates, "sume", 10, |_| 0.0).is_empty());
        assert!(search(&candidates, "pdf", 10, |_| 0.0).is_empty());
        assert_eq!(
            ids(&search(&candidates, "resume pdf", 10, |_| 0.0)),
            vec!["file:/Users/someone/resume.pdf"]
        );
    }

    #[test]
    fn tiers_interleave_title_and_keyword_evidence() {
        use KeywordMatch::{Complete, Partial};
        assert_eq!(tier(None, None), None);
        assert_eq!(
            tier(Some(MatchKind::Exact), Some(Complete)),
            Some(Tier::Exact)
        );
        assert_eq!(tier(None, Some(Complete)), Some(Tier::KeywordComplete));
        assert_eq!(
            tier(Some(MatchKind::Substring), Some(Partial)),
            Some(Tier::KeywordPartial)
        );
        assert_eq!(
            tier(Some(MatchKind::Acronym), Some(Complete)),
            Some(Tier::KeywordComplete)
        );
        assert!(Tier::WordPrefix > Tier::KeywordComplete);
        assert!(Tier::KeywordComplete > Tier::Acronym);
        assert!(Tier::KeywordPartial > Tier::Substring);
    }

    #[test]
    fn a_title_word_confirmed_by_a_keyword_counts_as_a_prefix() {
        assert_eq!(
            tier(Some(MatchKind::WordPrefix), Some(KeywordMatch::Complete)),
            Some(Tier::Prefix)
        );
        assert_eq!(
            tier(Some(MatchKind::WordPrefix), Some(KeywordMatch::Partial)),
            Some(Tier::WordPrefix)
        );
    }

    #[test]
    fn an_alias_matches_but_the_real_name_is_shown() {
        let mut trash = candidate("folder:/Users/someone/.Trash", Kind::Place, &["Trash"]);
        trash.names.push(Name::alias("Recycle Bin"));
        let candidates = [trash];
        let hits = search(&candidates, "recycle", 10, |_| 0.0);
        assert_eq!(hits[0].title, "Trash");
        assert_eq!(
            hits[0].kind,
            Some(MatchKind::Prefix),
            "the alias's match strength still counts"
        );
    }

    #[test]
    fn keywords_find_what_titles_miss() {
        let candidates = [with_keywords(
            candidate("settings:appearance", Kind::Setting, &["Appearance"]),
            &["theme", "Dark Mode"],
        )];
        let hits = search(&candidates, "dark mode", 10, |_| 0.0);
        assert_eq!(ids(&hits), vec!["settings:appearance"]);
        assert_eq!(
            hits[0].title, "Appearance",
            "keyword-only hits show the default title"
        );
        assert_eq!(hits[0].kind, None);
        assert_eq!(hits[0].tier, Tier::KeywordComplete);
    }

    #[test]
    fn a_confirmed_title_beats_an_unconfirmed_prefix() {
        let candidates = [
            candidate(
                "settings:a11y#camera",
                Kind::Setting,
                &["Camera Options (Head pointer)"],
            ),
            with_keywords(
                candidate(
                    "settings:privacy#Privacy_Camera",
                    Kind::Setting,
                    &["Allow applications to access the camera"],
                ),
                &["camera", "privacy"],
            ),
        ];
        let hits = search(&candidates, "camera", 10, |_| 0.0);
        assert_eq!(
            ids(&hits),
            vec!["settings:privacy#Privacy_Camera", "settings:a11y#camera"]
        );
    }

    #[test]
    fn apps_then_panes_then_sections_break_ties() {
        let candidates = [
            candidate(
                "settings:battery#options",
                Kind::Setting,
                &["Prevent automatic sleeping"],
            ),
            candidate("settings:prefs", Kind::Setting, &["Preferences"]),
            candidate("app:/Applications/Preview.app", Kind::App, &["Preview"]),
        ];
        let hits = search(&candidates, "pre", 10, |_| 0.0);
        assert_eq!(
            ids(&hits),
            vec![
                "app:/Applications/Preview.app",
                "settings:prefs",
                "settings:battery#options"
            ]
        );
    }

    #[test]
    fn usage_still_outranks_the_prior() {
        let candidates = [
            candidate(
                "settings:battery#options",
                Kind::Setting,
                &["Prevent automatic sleeping"],
            ),
            candidate("app:/Applications/Preview.app", Kind::App, &["Preview"]),
        ];
        let hits = search(&candidates, "pre", 10, |id| {
            if id == "settings:battery#options" {
                1.0
            } else {
                0.0
            }
        });
        assert_eq!(ids(&hits)[0], "settings:battery#options");
    }

    #[test]
    fn only_apps_match_as_subsequences() {
        let candidates = [
            candidate("app:/Applications/Safari.app", Kind::App, &["Safari"]),
            candidate("settings:x#y", Kind::Setting, &["Scroll speed (Trackpad)"]),
        ];
        assert_eq!(
            ids(&search(&candidates, "sfri", 10, |_| 0.0)),
            vec!["app:/Applications/Safari.app"]
        );
        assert!(
            search(&candidates, "sleep", 10, |_| 0.0).is_empty(),
            "s-l-e-e-p is scattered through the title"
        );
    }

    fn real_candidates() -> Vec<Candidate> {
        let home = std::env::home_dir().unwrap();
        crate::candidate::build(
            &crate::apps::discover_apps(),
            &crate::places::discover_places(&home),
            &crate::settings::discover_settings(),
            &[],
            &home,
        )
    }

    /// Real queries against this Mac's apps, places, and settings: the expected
    /// destination (an ID suffix) must rank within the first `within` results.
    /// A regression check for ranking changes, not a benchmark.
    #[test]
    fn real_queries_find_the_expected_destination() {
        let candidates = real_candidates();
        let cases: &[(&str, &str, usize)] = &[
            ("camera", "#Privacy_Camera", 1),
            ("mac address", "wifi-settings-extension#Advanced", 2),
            (
                "dark mode",
                "settings:com.apple.Appearance-Settings.extension",
                1,
            ),
            ("screen saver", "#ScreenSaver", 2),
            ("firewall", "#Firewall", 1),
            ("night shift", "#nightShiftSection", 1),
            ("bluetooth", "settings:com.apple.BluetoothSettings", 1),
            ("wifi", "settings:com.apple.wifi-settings-extension", 1),
            (
                "keyboard",
                "settings:com.apple.Keyboard-Settings.extension",
                1,
            ),
            (
                "printer",
                "settings:com.apple.Print-Scan-Settings.extension",
                1,
            ),
            ("ssh", "#Services_RemoteLogin", 1),
            ("dnd", "settings:com.apple.Focus-Settings.extension", 1),
            ("full disk access", "#Privacy_AllFiles", 1),
            (
                "natural scrolling",
                "Trackpad-Settings.extension#trackpadTab",
                2,
            ),
            ("sys", "System Settings.app", 1),
            ("prev", "Preview.app", 1),
            ("activity", "Activity Monitor.app", 1),
            ("safari", "Safari.app", 1),
            ("term", "Terminal.app", 1),
            ("downloads", "/Downloads", 1),
            ("down", "/Downloads", 1),
            ("documents", "/Documents", 1),
            ("trash", "/.Trash", 1),
            ("recycle bin", "/.Trash", 1),
            ("icloud", "/com~apple~CloudDocs", 1),
            ("applications", "folder:/Applications", 1),
            ("utilities", "folder:/Applications/Utilities", 1),
            ("music", "Music.app", 1),
            ("music", "/Music", 2),
        ];
        let mut failures = Vec::new();
        for &(query, expected, within) in cases {
            let hits = search(&candidates, query, 10, |_| 0.0);
            let rank = hits
                .iter()
                .position(|hit| hit.candidate.id.ends_with(expected));
            if rank.is_none_or(|rank| rank >= within) {
                failures.push(format!(
                    "{query:?}: {expected} at {rank:?}, top {:?}",
                    names_of(&hits[..hits.len().min(3)])
                ));
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn real_settings_are_findable_by_title() {
        let candidates = real_candidates();
        for (query, expected) in [
            (
                "night shift",
                "settings:com.apple.Displays-Settings.extension#nightShiftSection",
            ),
            ("bluetooth", "settings:com.apple.BluetoothSettings"),
            ("displays", "settings:com.apple.Displays-Settings.extension"),
        ] {
            let hits = search(&candidates, query, 10, |_| 0.0);
            assert!(ids(&hits).contains(&expected), "{query}: {:?}", ids(&hits));
        }
    }
}
