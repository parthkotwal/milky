//! Turning a query and a set of candidates into an ordered list of results.

use std::cmp::Reverse;
use std::path::{PathBuf};

use crate::apps::app_name;
use crate::matching::{MatchKind, match_kind};

/// One app that matched a query.
///
/// `path` is the identity — app names are not unique. Three Python installs on
/// this machine each ship an `IDLE.app`, so anything keyed by name would
/// collapse them into one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppMatch {
    pub name: String,
    pub path: PathBuf,
    pub kind: MatchKind,
}

/// Apps matching `query`, best first, capped at `limit`.
///
/// Takes the candidate paths rather than scanning: a full scan costs about
/// 1.6 ms, far too much to repeat per keystroke, so the caller holds the list.
///
/// `query` must already be normalized by [`crate::normalize_query`].
pub fn search_apps(apps: &[PathBuf], query: &str, limit: usize) -> Vec<AppMatch> {
    let mut matches: Vec<AppMatch> = apps
        .iter()
        .filter_map(|path| {
            let name = app_name(path)?;
            let kind = match_kind(&name, query)?;
            Some(AppMatch {
                name, 
                path: path.clone(),
                kind
            })
        })
        .collect();

    matches.sort_by(|a, b| {
        Reverse(a.kind)
            .cmp(&Reverse(b.kind))
            .then_with(|| a.name.len().cmp(&b.name.len()))
            .then_with(|| a.name.cmp(&b.name))
    });
    matches.truncate(limit);
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(|n| PathBuf::from(format!("/Applications/{n}.app"))).collect()
    }

    fn names_of(matches: &[AppMatch]) -> Vec<&str> {
        matches.iter().map(|m| m.name.as_str()).collect()
    }

    #[test]
    fn stronger_match_kinds_come_first() {
        // "Terminal Pro" is a prefix match; "iTerm" is only a substring match.
        // The prefix match is also the *longer* name, so this fails if the kind
        // comparison is dropped and length alone decides.
        let apps = paths(&["Terminal Pro", "iTerm"]);
        let found = search_apps(&apps, "term", 10);
        assert_eq!(names_of(&found), vec!["Terminal Pro", "iTerm"]);
    }

    #[test]
    fn shorter_names_break_ties() {
        let apps = paths(&["Contacts", "Code", "Console"]);
        let found = search_apps(&apps, "co", 10);
        assert_eq!(names_of(&found), vec!["Code", "Console", "Contacts"]);
    }

    #[test]
    fn non_matches_are_excluded() {
        let apps = paths(&["Terminal", "Safari"]);
        let found = search_apps(&apps, "term", 10);
        assert_eq!(names_of(&found), vec!["Terminal"]);
    }

    #[test]
    fn respects_the_limit() {
        let apps = paths(&["Code", "Console", "Contacts", "Compass"]);
        assert_eq!(search_apps(&apps, "co", 2).len(), 2);
    }

    #[test]
    fn identical_names_are_kept_as_separate_results() {
        let apps = vec![
            PathBuf::from("/Applications/Python 3.12/IDLE.app"),
            PathBuf::from("/Applications/Python 3.11/IDLE.app"),
        ];
        let found = search_apps(&apps, "idle", 10);
        assert_eq!(found.len(), 2, "path is the identity, not name");
    }

    #[test]
    fn ordering_is_stable_across_calls() {
        let apps = paths(&["Code", "Console", "Contacts"]);
        assert_eq!(search_apps(&apps, "co", 10), search_apps(&apps, "co", 10));
    }

    #[test]
    fn empty_query_returns_nothing() {
        let apps = paths(&["Terminal"]);
        assert!(search_apps(&apps, "", 10).is_empty());
    }
}