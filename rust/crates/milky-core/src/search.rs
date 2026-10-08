//! Turning a query and a set of candidates into an ordered list of results.

use std::cmp::Reverse;

use crate::candidate::Candidate;
use crate::matching::{MatchKind, match_key};

/// One candidate that matched a query. Borrows from the candidate list
/// instead of copying it, so a search allocates nothing per candidate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit<'a> {
    pub candidate: &'a Candidate,
    /// The name that matched best — shown as the result's title.
    pub title: &'a str,
    pub kind: MatchKind,
    pub usage: f64,
}

/// Candidates matching `query`, best first, capped at `limit`. One hit per
/// candidate, titled by its best-matching name (the earlier name wins a tie).
/// Sorted by match kind, then usage, then title length, then title, then ID,
/// so the order is fully deterministic.
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
            let (kind, name) = candidate
                .names
                .iter()
                .filter_map(|name| Some((match_key(&name.key, query)?, name)))
                .min_by_key(|(kind, _)| Reverse(*kind))?;
            Some(Hit {
                candidate,
                title: &name.text,
                kind,
                usage: usage_of(&candidate.id),
            })
        })
        .collect();

    hits.sort_by(|a, b| {
        Reverse(a.kind)
            .cmp(&Reverse(b.kind))
            .then_with(|| b.usage.total_cmp(&a.usage))
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
            .map(|n| App::new(PathBuf::from(format!("/Applications/{n}.app")), n.to_string()))
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
            App::new(PathBuf::from("/Applications/Python 3.12/IDLE.app"), "IDLE".into()),
            App::new(PathBuf::from("/Applications/Python 3.11/IDLE.app"), "IDLE".into()),
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
        }
    }

    fn ids<'a>(hits: &[Hit<'a>]) -> Vec<&'a str> {
        hits.iter().map(|hit| hit.candidate.id.as_str()).collect()
    }

    #[test]
    fn the_best_matching_name_becomes_the_title() {
        let candidates = [candidate("settings:wifi#Advanced", Kind::Setting, &["Advanced", "Wi-Fi MAC Address"])];
        let mac = search(&candidates, "mac", 10, |_| 0.0);
        assert_eq!(mac[0].title, "Wi-Fi MAC Address");
        assert_eq!(mac[0].kind, MatchKind::WordPrefix);
        let adv = search(&candidates, "adv", 10, |_| 0.0);
        assert_eq!(adv[0].title, "Advanced");
        assert_eq!(adv[0].kind, MatchKind::Prefix);
    }

    #[test]
    fn the_earlier_name_wins_a_tie() {
        let candidates = [candidate("settings:x#a", Kind::Setting, &["Show legacy networks", "Show options"])];
        assert_eq!(search(&candidates, "show", 10, |_| 0.0)[0].title, "Show legacy networks");
    }

    #[test]
    fn one_destination_gives_at_most_one_hit() {
        let candidates = [candidate("settings:x#a", Kind::Setting, &["Advanced", "Advanced options"])];
        assert_eq!(search(&candidates, "adv", 10, |_| 0.0).len(), 1);
    }

    #[test]
    fn apps_and_settings_rank_together() {
        let candidates = [
            candidate("app:/Applications/Utilities/Bluetooth File Exchange.app", Kind::App, &["Bluetooth File Exchange"]),
            candidate("settings:com.apple.BluetoothSettings", Kind::Setting, &["Bluetooth"]),
        ];
        let hits = search(&candidates, "bluetooth", 10, |_| 0.0);
        assert_eq!(
            ids(&hits),
            vec!["settings:com.apple.BluetoothSettings", "app:/Applications/Utilities/Bluetooth File Exchange.app"]
        );
    }

    #[test]
    fn usage_is_looked_up_by_id() {
        let candidates = [
            candidate("settings:a", Kind::Setting, &["Calendar sync"]),
            candidate("settings:b", Kind::Setting, &["Calendar view"]),
        ];
        let hits = search(&candidates, "cal", 10, |id| if id == "settings:b" { 3.0 } else { 0.0 });
        assert_eq!(ids(&hits), vec!["settings:b", "settings:a"]);
    }

    #[test]
    fn identical_titles_are_ordered_by_id() {
        let candidates = [
            candidate("settings:vpn", Kind::Setting, &["VPN options"]),
            candidate("settings:network", Kind::Setting, &["VPN options"]),
        ];
        assert_eq!(ids(&search(&candidates, "vpn", 10, |_| 0.0)), vec!["settings:network", "settings:vpn"]);
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

    #[test]
    fn real_settings_are_findable_by_title() {
        let candidates = crate::candidate::build(&crate::apps::discover_apps(), &crate::settings::discover_settings());
        for (query, expected) in [
            ("night shift", "settings:com.apple.Displays-Settings.extension#nightShiftSection"),
            ("bluetooth", "settings:com.apple.BluetoothSettings"),
            ("displays", "settings:com.apple.Displays-Settings.extension"),
        ] {
            let hits = search(&candidates, query, 10, |_| 0.0);
            assert!(ids(&hits).contains(&expected), "{query}: {:?}", ids(&hits));
        }
    }
}
