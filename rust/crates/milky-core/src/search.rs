//! Turning a query and a set of candidates into an ordered list of results.

use std::cmp::{Ordering, Reverse};
use std::path::Path;

use std::time::SystemTime;

use crate::candidate::{Action, Candidate, FileFacts, Kind, folder_id};
use crate::matching::{KeywordMatch, MatchKind, NameKey, keyword_match, match_key, words_of};

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

/// How many characters (spaces aside) a query needs before files and folders
/// match normally. Shorter queries are almost always the start of an app
/// name, and short folder names (`go`, `ui`, `db`) would otherwise match
/// exactly and outrank the app (`go` above Google Chrome). Below this, a file
/// or folder must match exactly and then ranks as a [`Tier::WordPrefix`]
/// match: `uw` still finds `~/UW` when no app starts with it, and `go` lists
/// `~/go` below Google Chrome. Measured 2026-10-09: at 3 characters the app
/// came first in every sample query.
pub const MIN_FILE_QUERY_CHARS: usize = 3;

/// Whether `query` ends with this file's whole name, extension included, at a
/// word boundary: `... parth kotwal google pdf`. Compares strings without
/// allocating, so it can run on every file before [`names_whole_path`].
fn ends_with_whole_name(name: &NameKey, facts: &FileFacts, query: &str) -> bool {
    let rest = match facts.extension.as_deref() {
        Some(extension) => match query.strip_suffix(extension).and_then(|rest| rest.strip_suffix(' ')) {
            Some(rest) => rest,
            None => return false,
        },
        None => query,
    };
    rest.strip_suffix(name.folded())
        .is_some_and(|before| before.is_empty() || before.ends_with(' '))
}

/// Whether `query` names the candidate's file by its path: the whole name,
/// extension included (`parth kotwal google pdf`), or a path ending in it,
/// written the way it is shown (`desktop customized parth kotwal google pdf`
/// from `~/Desktop/Customized/...`; `icloud drive taxes ...`) or in full
/// (`/Users/...`). Compares whole path components, so `kotwal google pdf`
/// is not a whole name. Only called for candidates that already matched, so
/// the allocations here stay off the per-candidate path.
fn names_whole_path(candidate: &Candidate, query: &str) -> bool {
    let Action::Open(path) = &candidate.action else {
        return false;
    };
    let shown: Vec<&str> = candidate
        .subtitle
        .split('/')
        .chain(std::iter::once(candidate.title()))
        .collect();
    let full: Vec<&str> = path.iter().filter_map(|part| part.to_str()).collect();
    [shown, full].iter().any(|parts| {
        (1..=parts.len()).any(|count| {
            crate::normalize_query(&parts[parts.len() - count..].join(" ")) == query
        })
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
    let short_query = query.chars().filter(|c| *c != ' ').count() < MIN_FILE_QUERY_CHARS;
    let hits: Vec<Hit<'a>> = candidates
        .iter()
        .filter_map(|candidate| {
            // Files and folders match by their own rule: strong name matches
            // only, with folders and the extension as support. Naming the
            // whole file or a path to it is exact. Short queries need an exact
            // match, which then ranks below anything the query starts.
            if let Some(facts) = &candidate.file {
                let name = candidate.names.first()?;
                // Naming the whole file or a path to it is exact. The path check
                // allocates, so it runs only when the query ends with this
                // file's whole name, which rules out almost every file for free.
                // A full path (`/Users/...`, `iCloud Drive/...`) can name a file
                // even when `file_tier` finds no match: it has words no folder
                // word covers.
                let names_path = |tier: Option<Tier>| {
                    tier < Some(Tier::Exact)
                        && ends_with_whole_name(&name.key, facts, query)
                        && names_whole_path(candidate, query)
                };
                let tier = file_tier(&name.key, facts, query);
                let mut tier = if names_path(tier) { Tier::Exact } else { tier? };
                if short_query {
                    if tier < Tier::Exact {
                        return None;
                    }
                    tier = Tier::WordPrefix;
                }
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

    let best = select_best(hits, limit, file_cap(limit));
    with_typed_folder(best, candidates, query, limit)
}

/// The folder `query` named on the way to the file at `path`: the deepest
/// folder above it with a word that some query word starts, counting only the
/// query words the file's own name does not account for (a word starting no
/// word of `name`). `None` if no query word names a folder above it.
///
/// `wags arch` for `~/Projects/wags/Archive.zip` -> `~/Projects/wags`;
/// `school ex01` for `~/school/cse332/ex01.pdf` -> `~/school`, the folder the
/// user typed, not the file's parent; `ex01 pdf` -> `None`.
///
/// `path.ancestors()` yields the path, then its parent, and so on up to `/`:
/// skip the path itself. Words of a folder's name: [`words_of`].
pub fn typed_folder<'p>(path: &'p Path, name: &NameKey, query: &str) -> Option<&'p Path> {
    let supporting: Vec<&str> = query
        .split(' ')
        .filter(|word| !name.words().iter().any(|w| w.starts_with(*word)))
        .collect();

    path.ancestors().skip(1).find(|dir| {
        dir.file_name().is_some_and(|folder| {
            words_of(&folder.to_string_lossy())
                .iter()
                .any(|w| supporting.iter().any(|s| w.starts_with(*s)))
        })
    })
}

/// `hits` plus, as the last result, the folder the user typed to reach the
/// best file that needed one: so `wags arch` offers `~/Projects/wags` after
/// `Archive.zip`, to open and look around.
///
/// - The file: the first hit (they are in rank order) that is a file or
///   folder whose name did not match the whole query (`hit.kind` is `None`:
///   a folder or the extension helped).
/// - The folder: [`typed_folder`] of its path (from `hit.candidate.action`,
///   `Action::Open(path)`), found among `candidates` by its ID
///   ([`crate::candidate::folder_id`]). Not indexed (excluded, or above the
///   home folder): add nothing.
/// - Already among `hits`: add nothing; it is shown once.
/// - Otherwise it goes last, as a hit with `kind: None`, `keyword: None`,
///   tier [`Tier::PathSupported`], usage 0, titled by the folder's name. If the
///   list is full (`limit`), it replaces the last hit; with `limit` below 2,
///   add nothing, so the file itself is never pushed out.
pub fn with_typed_folder<'a>(
    mut hits: Vec<Hit<'a>>,
    candidates: &'a [Candidate],
    query: &str,
    limit: usize,
) -> Vec<Hit<'a>> {
    if limit < 2 {
        return hits;
    }
    let Some(trigger) = hits
        .iter()
        .find(|hit| hit.candidate.file.is_some() && hit.kind.is_none())
        .copied()
    else {
        return hits;
    };
    let Action::Open(path) = &trigger.candidate.action else {
        return hits;
    };
    let Some(name) = trigger.candidate.names.first() else {
        return hits;
    };
    let Some(folder_path) = typed_folder(path, &name.key, query) else {
        return hits;
    };
    let Some(id) = folder_id(folder_path) else {
        return hits;
    };
    if hits.iter().any(|hit| hit.candidate.id == id) {
        return hits;
    }
    let Some(folder) = candidates.iter().find(|candidate| candidate.id == id) else {
        return hits;
    };

    if hits.len() >= limit {
        hits.pop();
    }
    hits.push(Hit {
        candidate: folder,
        title: folder.title(),
        kind: None,
        keyword: None,
        tier: Tier::PathSupported,
        usage: 0.0,
    });
    hits
}

/// How two hits compare: `Less` means `a` ranks first. Tier, then usage, then
/// prior (apps, places, settings, folders, files), then keyword match, then
/// the more recently modified file or folder, then the shorter title, then
/// title, then ID. Total: no two distinct hits compare `Equal` (IDs differ),
/// so any correct selection or sort gives the same order.
pub fn rank(a: &Hit, b: &Hit) -> Ordering {
    b.tier
        .cmp(&a.tier)
        .then_with(|| b.usage.total_cmp(&a.usage))
        .then_with(|| b.candidate.prior().cmp(&a.candidate.prior()))
        .then_with(|| b.keyword.cmp(&a.keyword))
        .then_with(|| modified(b).cmp(&modified(a)))
        .then_with(|| a.title.len().cmp(&b.title.len()))
        .then_with(|| a.title.cmp(b.title))
        .then_with(|| a.candidate.id.cmp(&b.candidate.id))
}

/// When a file or folder last changed; `None` for everything else, and for
/// files whose date is unknown, which then rank after dated ones.
fn modified(hit: &Hit) -> Option<SystemTime> {
    hit.candidate.file.as_ref()?.modified
}

/// How many of `limit` results may be files or folders: 40%, at least one.
/// The rest is kept for apps, places, and settings, which are fewer and more
/// often what a launcher query wants. A candidate for a user setting later.
pub fn file_cap(limit: usize) -> usize {
    (limit * 2 / 5).max(1)
}

/// Whether a hit is a walked file or folder, which the file cap limits.
fn is_file(hit: &Hit) -> bool {
    hit.candidate.file.is_some()
}

/// The best `n` hits in [`rank`] order, without sorting all of them.
///
/// `select_nth_unstable_by(n - 1, rank)` moves the `n` best hits to the front
/// (in no particular order) in one pass. Then sort just that front part and
/// drop the rest. Fewer than `n` hits: sort them all. `n == 0`: nothing.
pub fn top<'a>(mut hits: Vec<Hit<'a>>, n: usize) -> Vec<Hit<'a>> {
    if n == 0 {
        return Vec::new();
    }
    if hits.len() > n {
        hits.select_nth_unstable_by(n - 1, rank);
        hits.truncate(n);
    }
    hits.sort_by(rank);
    hits
}

/// At most `limit` hits in [`rank`] order, with at most `file_cap` files or
/// folders among them, unless there are not enough other hits to fill
/// `limit`, in which case more files fill the rest, still best first.
///
/// Split the hits into files and everything else; take the [`top`] `limit`
/// of each; then merge the two ranked lists, always taking whichever head
/// ranks first, but skipping files once `file_cap` are taken. If the merged
/// list is short of `limit`, add the skipped files, best first, and keep the
/// result in rank order.
pub fn select_best<'a>(hits: Vec<Hit<'a>>, limit: usize, file_cap: usize) -> Vec<Hit<'a>> {
    let (files, others): (Vec<_>, Vec<_>) = hits.into_iter().partition(is_file);
    let mut files = top(files, limit).into_iter().peekable();
    let mut others = top(others, limit).into_iter().peekable();

    let mut chosen: Vec<Hit<'a>> = Vec::with_capacity(limit);
    let mut skipped: Vec<Hit<'a>> = Vec::new();
    let mut files_taken = 0;

    while chosen.len() < limit {
        let take_file = match (files.peek(), others.peek()) {
            (None, None) => break,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (Some(f), Some(o)) => rank(f, o) == Ordering::Less,
        };
        if take_file {
            if let Some(hit) = files.next() {
                if files_taken < file_cap {
                    files_taken += 1;
                    chosen.push(hit);
                } else {
                    skipped.push(hit);
                }
            }
        } else if let Some(hit) = others.next() {
            chosen.push(hit);
        }
    }

    let room = limit - chosen.len();
    chosen.extend(skipped.into_iter().take(room));
    chosen.sort_by(rank);
    chosen
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

    fn dated_file(path: &str, seconds: u64) -> Candidate {
        let entry = crate::files::FileEntry {
            path: std::path::PathBuf::from(path),
            is_folder: false,
            modified: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds)),
        };
        crate::candidate::from_file(&entry, std::path::Path::new("/Users/someone")).unwrap()
    }

    /// Hits for every candidate, as if all matched equally, in a scrambled order.
    fn scrambled_hits(candidates: &[Candidate]) -> Vec<Hit<'_>> {
        let mut hits: Vec<Hit> = candidates
            .iter()
            .map(|candidate| Hit {
                candidate,
                title: candidate.title(),
                kind: Some(MatchKind::Prefix),
                keyword: None,
                tier: Tier::Prefix,
                usage: 0.0,
            })
            .collect();
        // A fixed shuffle: reverse, then swap pairs.
        hits.reverse();
        for pair in hits.chunks_mut(2) {
            pair.reverse();
        }
        hits
    }

    fn sorted<'a>(mut hits: Vec<Hit<'a>>) -> Vec<Hit<'a>> {
        hits.sort_by(rank);
        hits
    }

    fn key(name: &str) -> NameKey {
        NameKey::new(name)
    }

    #[test]
    fn the_typed_folder_is_the_one_the_query_named() {
        let archive = Path::new("/Users/someone/Projects/wags/Archive.zip");
        assert_eq!(typed_folder(archive, &key("Archive"), "wags arch"), Some(Path::new("/Users/someone/Projects/wags")));
        assert_eq!(typed_folder(archive, &key("Archive"), "projects arch"), Some(Path::new("/Users/someone/Projects")));
        let ex01 = Path::new("/Users/someone/school/cse332/ex01.pdf");
        assert_eq!(
            typed_folder(ex01, &key("ex01"), "school ex01"),
            Some(Path::new("/Users/someone/school")),
            "the folder typed, not the parent"
        );
        assert_eq!(typed_folder(ex01, &key("ex01"), "332 ex01"), Some(Path::new("/Users/someone/school/cse332")));
    }

    #[test]
    fn no_folder_is_typed_when_the_name_or_extension_covers_the_query() {
        let ex01 = Path::new("/Users/someone/school/cse332/ex01.pdf");
        assert_eq!(typed_folder(ex01, &key("ex01"), "ex01 pdf"), None, "the extension is not a folder");
        assert_eq!(typed_folder(ex01, &key("ex01"), "ex01"), None);
    }

    #[test]
    fn a_file_found_through_a_folder_offers_that_folder_last() {
        let candidates = [
            file("/Users/someone/Projects/wags/Archive.zip"),
            file("/Users/someone/Projects/wags/archive-notes.md"),
            file("/Users/someone/Projects/wags"),
            file("/Users/someone/Projects"),
        ];
        assert_eq!(
            ids(&search(&candidates, "wags arch", 10, |_| 0.0)),
            vec![
                "file:/Users/someone/Projects/wags/Archive.zip",
                "file:/Users/someone/Projects/wags/archive-notes.md",
                "folder:/Users/someone/Projects/wags",
            ],
            "the folder once, after every file"
        );
    }

    #[test]
    fn the_typed_folder_takes_the_last_slot_of_a_full_list() {
        let mut candidates: Vec<Candidate> = (0..5)
            .map(|i| file(&format!("/Users/someone/wags/arch{i}.zip")))
            .collect();
        candidates.push(file("/Users/someone/wags"));
        let hits = search(&candidates, "wags arch", 3, |_| 0.0);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[2].candidate.id, "folder:/Users/someone/wags");
        assert!(
            search(&candidates, "wags arch", 1, |_| 0.0)[0].candidate.id.starts_with("file:"),
            "a one-result list keeps the file"
        );
    }

    #[test]
    fn no_folder_is_added_when_none_was_needed_or_it_is_already_shown() {
        let candidates = [file("/Users/someone/wags/Archive.zip"), file("/Users/someone/wags")];
        assert_eq!(ids(&search(&candidates, "archive", 10, |_| 0.0)), vec!["file:/Users/someone/wags/Archive.zip"]);
        let unindexed = [file("/Users/someone/wags/Archive.zip")];
        assert_eq!(search(&unindexed, "wags arch", 10, |_| 0.0).len(), 1, "an excluded folder is not invented");
        // The folder "wags arch" matches the query itself, so it is already a
        // result: Archive.zip inside it must not bring it in a second time.
        let shown = [file("/Users/someone/wags arch/Archive.zip"), file("/Users/someone/wags arch")];
        assert_eq!(
            ids(&search(&shown, "wags arch", 10, |_| 0.0)),
            vec!["folder:/Users/someone/wags arch", "file:/Users/someone/wags arch/Archive.zip"]
        );
    }

    #[test]
    fn short_queries_need_an_exact_file_name_and_rank_it_below_apps() {
        let candidates = [
            file("/Users/someone/go"),
            file("/Users/someone/goal.txt"),
            candidate("app:/Applications/Google Chrome.app", Kind::App, &["Google Chrome"]),
        ];
        assert_eq!(ids(&search(&candidates, "g", 10, |_| 0.0)), vec!["app:/Applications/Google Chrome.app"]);
        let hits = search(&candidates, "go", 10, |_| 0.0);
        assert_eq!(
            ids(&hits),
            vec!["app:/Applications/Google Chrome.app", "folder:/Users/someone/go"],
            "the exact folder is listed, below the app; goal.txt is not exact"
        );
        assert_eq!(hits[1].tier, Tier::WordPrefix);
        assert_eq!(search(&candidates, "goa", 10, |_| 0.0).len(), 1, "from three characters, as usual");
        assert!(
            search(&candidates, "g o", 10, |_| 0.0).iter().all(|hit| hit.candidate.file.is_none()),
            "spaces do not count"
        );
    }

    #[test]
    fn a_short_exact_folder_leads_when_nothing_starts_with_the_query() {
        let candidates = [
            file("/Users/someone/UW"),
            candidate("app:/Applications/uTorrent Web.app", Kind::App, &["uTorrent Web"]),
        ];
        assert_eq!(
            ids(&search(&candidates, "uw", 10, |_| 0.0)),
            vec!["folder:/Users/someone/UW", "app:/Applications/uTorrent Web.app"]
        );
    }

    #[test]
    fn the_whole_file_name_with_its_extension_is_exact() {
        let candidates = [file("/Users/someone/Desktop/Customized/Parth Kotwal Google.pdf")];
        for query in ["Parth Kotwal Google.pdf", "parth kotwal google pdf"] {
            let hits = search(&candidates, &crate::normalize_query(query), 10, |_| 0.0);
            assert_eq!(hits[0].tier, Tier::Exact, "{query}");
        }
        let partial = search(&candidates, &crate::normalize_query("kotwal google.pdf"), 10, |_| 0.0);
        assert!(partial[0].tier < Tier::Exact, "part of a name is not the whole name");
    }

    #[test]
    fn a_pasted_path_is_exact() {
        let candidates = [file("/Users/someone/Desktop/Customized/Parth Kotwal Google.pdf")];
        for query in [
            "desktop/customized/Parth Kotwal Google.pdf",
            "~/Desktop/Customized/Parth Kotwal Google.pdf",
            "/Users/someone/Desktop/Customized/Parth Kotwal Google.pdf",
            "Customized/Parth Kotwal Google.pdf",
        ] {
            let hits = search(&candidates, &crate::normalize_query(query), 10, |_| 0.0);
            assert_eq!(hits.len(), 1, "{query}");
            assert_eq!(hits[0].tier, Tier::Exact, "{query}");
        }
    }

    #[test]
    fn an_icloud_path_is_written_as_finder_shows_it() {
        let path = format!("/Users/someone/{}/Taxes/w2.pdf", crate::candidate::ICLOUD_DRIVE);
        let candidates = [file(&path)];
        let hits = search(&candidates, &crate::normalize_query("iCloud Drive/Taxes/w2.pdf"), 10, |_| 0.0);
        assert_eq!(hits[0].tier, Tier::Exact);
    }

    #[test]
    fn places_still_match_from_the_first_character() {
        let candidates = [candidate("folder:/Users/someone/Downloads", Kind::Place, &["Downloads"])];
        assert_eq!(search(&candidates, "d", 10, |_| 0.0).len(), 1);
    }

    #[test]
    fn the_newest_of_equally_good_files_comes_first() {
        let candidates = [
            dated_file("/Users/someone/a/index.ts", 100),
            dated_file("/Users/someone/b/index.ts", 300),
            dated_file("/Users/someone/c/index.ts", 200),
        ];
        assert_eq!(
            ids(&search(&candidates, "index", 10, |_| 0.0)),
            vec!["file:/Users/someone/b/index.ts", "file:/Users/someone/c/index.ts", "file:/Users/someone/a/index.ts"]
        );
    }

    #[test]
    fn the_cap_is_forty_percent_and_at_least_one() {
        assert_eq!(file_cap(10), 4);
        assert_eq!(file_cap(20), 8);
        assert_eq!(file_cap(1), 1);
        assert_eq!(file_cap(2), 1);
    }

    #[test]
    fn top_returns_the_best_n_in_rank_order() {
        let candidates: Vec<Candidate> = (0..40).map(|i| dated_file(&format!("/Users/someone/f{i}.txt"), i)).collect();
        let hits = scrambled_hits(&candidates);
        let expected: Vec<&str> = ids(&sorted(hits.clone())[..5]);
        assert_eq!(ids(&top(hits.clone(), 5)), expected);
        assert_eq!(ids(&top(hits.clone(), 100)), ids(&sorted(hits.clone())), "fewer than n: all, sorted");
        assert!(top(hits, 0).is_empty());
    }

    #[test]
    fn files_are_capped_when_other_results_can_fill_the_list() {
        // Every file outranks every app (Exact vs Prefix), yet only 4 files
        // make the list: the best 4, still ranked above the apps.
        let mut candidates: Vec<Candidate> = (0..3000)
            .map(|i| dated_file(&format!("/Users/someone/p{i}/report.txt"), i))
            .collect();
        candidates.extend((0..20).map(|i| {
            candidate(&format!("app:/Applications/Report Viewer {i}.app"), Kind::App, &[&format!("Report Viewer {i}")])
        }));
        let hits = search(&candidates, "report", 10, |_| 0.0);
        assert_eq!(hits.len(), 10);
        let files: Vec<&str> = ids(&hits).into_iter().filter(|id| id.starts_with("file:")).collect();
        assert_eq!(
            files,
            vec![
                "file:/Users/someone/p2999/report.txt",
                "file:/Users/someone/p2998/report.txt",
                "file:/Users/someone/p2997/report.txt",
                "file:/Users/someone/p2996/report.txt",
            ],
            "the four newest"
        );
        assert!(ids(&hits)[..4].iter().all(|id| id.starts_with("file:")), "kept in rank order");
    }

    #[test]
    fn files_fill_the_list_when_little_else_matches() {
        let mut candidates: Vec<Candidate> = (0..30)
            .map(|i| dated_file(&format!("/Users/someone/p{i}/report.txt"), i))
            .collect();
        candidates.push(candidate("app:/Applications/Report Viewer.app", Kind::App, &["Report Viewer"]));
        let hits = search(&candidates, "report", 10, |_| 0.0);
        assert_eq!(hits.len(), 10);
        assert_eq!(ids(&hits).iter().filter(|id| id.starts_with("file:")).count(), 9);
        assert_eq!(ids(&hits), ids(&sorted(hits.clone())), "still in rank order");
    }

    #[test]
    fn select_best_keeps_rank_order_across_both_lists() {
        // Mixed hits in one tier: apps outrank files by prior, so the list is
        // apps first, then up to the cap of files.
        let mut candidates: Vec<Candidate> = (0..6).map(|i| dated_file(&format!("/Users/someone/n{i}.txt"), i)).collect();
        candidates.extend((0..6).map(|i| candidate(&format!("app:/A{i}.app"), Kind::App, &[&format!("N{i}")])));
        let hits = scrambled_hits(&candidates);
        let best = select_best(hits.clone(), 5, 2);
        let expected: Vec<&str> = ids(&sorted(hits)).into_iter().filter(|id| id.starts_with("app:")).take(5).collect();
        assert_eq!(ids(&best), expected, "five apps outrank every file, so no file is needed");
        let best = select_best(scrambled_hits(&candidates[..6]), 5, 2);
        assert_eq!(best.len(), 5, "only files: they fill the list past the cap");
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
