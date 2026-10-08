//! Turning a query and a set of candidates into an ordered list of results.

use std::cmp::Reverse;
use std::path::{Path, PathBuf};

use crate::apps::App;
use crate::matching::{MatchKind, match_key};

/// One app that matched a query.
///
/// `path` is the identity — app names are not unique. Three Python installs on
/// this machine each ship an `IDLE.app`, so anything keyed by name would
/// collapse them into one.
#[derive(Debug, Clone, PartialEq)]
pub struct AppMatch {
    pub name: String,
    pub path: PathBuf,
    pub kind: MatchKind,
    /// Decayed launch count at query time; 0.0 if never launched.
    pub usage: f64,
}

/// Apps matching `query`, best first, capped at `limit`.
///
/// Takes the discovered apps rather than scanning: a full scan costs about
/// 1.6 ms, far too much to repeat per keystroke, so the caller holds the list.
///
/// `query` must already be normalized by [`crate::normalize_query`].
pub fn search_apps(
    apps: &[App],
    query: &str,
    limit: usize,
    usage_of: impl Fn(&Path) -> f64,
) -> Vec<AppMatch> {
    let mut matches: Vec<AppMatch> = apps
        .iter()
        .filter_map(|app| {
            let kind = match_key(&app.key, query)?;
            Some(AppMatch {
                name: app.name.clone(),
                path: app.path.clone(),
                kind,
                usage: usage_of(&app.path),
            })
        })
        .collect();

    matches.sort_by(|a, b| {
        Reverse(a.kind)
            .cmp(&Reverse(b.kind))
            .then_with(|| b.usage.total_cmp(&a.usage))
            .then_with(|| a.name.len().cmp(&b.name.len()))
            .then_with(|| a.name.cmp(&b.name))
    });
    matches.truncate(limit);
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps_named(names: &[&str]) -> Vec<App> {
        names
            .iter()
            .map(|n| {
                App::new(
                    PathBuf::from(format!("/Applications/{n}.app")),
                    n.to_string(),
                )
            })
            .collect()
    }

    fn names_of(matches: &[AppMatch]) -> Vec<&str> {
        matches.iter().map(|m| m.name.as_str()).collect()
    }

    #[test]
    fn stronger_match_kinds_come_first() {
        // "Terminal Pro" is a prefix match; "iTerm" is only a word-prefix match (on "Term").
        // The prefix match is also the *longer* name, so this fails if the kind
        // comparison is dropped and length alone decides.
        let apps = apps_named(&["Terminal Pro", "iTerm"]);
        let found = search_apps(&apps, "term", 10, |_| 0.0);
        assert_eq!(names_of(&found), vec!["Terminal Pro", "iTerm"]);
    }

    #[test]
    fn shorter_names_break_ties() {
        let apps = apps_named(&["Contacts", "Code", "Console"]);
        let found = search_apps(&apps, "co", 10, |_| 0.0);
        assert_eq!(names_of(&found), vec!["Code", "Console", "Contacts"]);
    }

    #[test]
    fn non_matches_are_excluded() {
        let apps = apps_named(&["Terminal", "Safari"]);
        let found = search_apps(&apps, "term", 10, |_| 0.0);
        assert_eq!(names_of(&found), vec!["Terminal"]);
    }

    #[test]
    fn respects_the_limit() {
        let apps = apps_named(&["Code", "Console", "Contacts", "Compass"]);
        assert_eq!(search_apps(&apps, "co", 2, |_| 0.0).len(), 2);
    }

    #[test]
    fn identical_names_are_kept_as_separate_results() {
        let apps = vec![
            App::new(
                PathBuf::from("/Applications/Python 3.12/IDLE.app"),
                "IDLE".into(),
            ),
            App::new(
                PathBuf::from("/Applications/Python 3.11/IDLE.app"),
                "IDLE".into(),
            ),
        ];
        let found = search_apps(&apps, "idle", 10, |_| 0.0);
        assert_eq!(found.len(), 2, "path is the identity, not name");
    }

    #[test]
    fn ordering_is_stable_across_calls() {
        let apps = apps_named(&["Code", "Console", "Contacts"]);
        assert_eq!(
            search_apps(&apps, "co", 10, |_| 0.0),
            search_apps(&apps, "co", 10, |_| 0.0)
        );
    }

    #[test]
    fn empty_query_returns_nothing() {
        let apps = apps_named(&["Terminal"]);
        assert!(search_apps(&apps, "", 10, |_| 0.0).is_empty());
    }

    #[test]
    fn usage_breaks_ties_within_a_kind() {
        let apps = apps_named(&["Chess", "Calendar"]);
        let found = search_apps(&apps, "c", 10, |path| {
            if path.ends_with("Calendar.app") {
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
        let found = search_apps(&apps, "term", 10, |path| {
            if path.ends_with("iTerm.app") {
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
        let found = search_apps(&apps, "co", 10, |_| 3.0);
        assert_eq!(names_of(&found), vec!["Code", "Console", "Contacts"]);
    }
}
