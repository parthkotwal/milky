//! Milky's search engine core.

pub mod apps;
pub mod matching;
pub mod search;
pub mod engine;
pub mod usage;

/// Normalize a raw query for retrieval.
///
/// Lowercases, trims surrounding whitespace, and collapses internal runs of
/// whitespace to single spaces.
pub fn normalize_query(input: &str) -> String {
    // todo!("write me")
    let lowered: String = input.to_lowercase();
    let pieces: std::str::SplitWhitespace<'_>= lowered.split_whitespace();
    let words: Vec<&str> = pieces.collect();
    words.join(" ")
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
}