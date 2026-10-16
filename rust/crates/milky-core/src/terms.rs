//! Splitting a document's text into indexed words, with positions
//! (DECISIONS 2026-10-16, document search, beat D2).
//!
//! Words are split exactly as names are (`matching`): at spaces, punctuation,
//! letter/digit changes, and camelCase, folded to lowercase without accents.
//! Each part gets the next position, so a phrase query finds parts that sit
//! next to each other; a token's joined form (`cse332`, `finalreport`,
//! `getusername`) is also indexed, at the position of its first part, so the
//! token typed whole is found too.
//!
//! `getUserName(id)` -> `get`@0 `user`@1 `name`@2 `id`@3, then the joined
//! forms `getusername`@0 (the piece) and `getusernameid`@0 (the whole token,
//! the same rule that makes `wi-fi` findable as `wifi`).

use std::collections::HashMap;

use crate::matching::{fold, piece_parts, punctuation_pieces};

/// Words longer than this are not indexed: in code and text they are almost
/// always encoded data (hashes, base64), never something typed.
pub const MAX_WORD_CHARS: usize = 64;

/// One word occurrence: the folded word and its position in the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub word: String,
    pub position: u32,
}

/// [`fold`] for one word part. Parts contain only letters, digits, and
/// accents (punctuation already split them), so ASCII parts need nothing but
/// lowercasing: a fast path that skips Unicode normalization, which is most
/// of the cost on code and English text.
fn fold_word(part: &str) -> String {
    if part.is_ascii() { part.to_ascii_lowercase() } else { fold(part) }
}

/// Every indexed word occurrence in `text`: each token's parts in order of
/// position, then its joined forms.
pub fn occurrences(text: &str) -> Vec<Occurrence> {
    let mut found = Vec::new();
    let mut next: u32 = 0;
    for token in text.split_whitespace() {
        let token_start = next;
        let mut token_parts: Vec<String> = Vec::new();
        let mut joins: Vec<(String, u32)> = Vec::new();
        let mut pieces = 0;
        for piece in punctuation_pieces(token) {
            pieces += 1;
            let piece_start = next;
            let parts: Vec<String> = piece_parts(piece)
                .into_iter()
                .map(fold_word)
                .filter(|part| !part.is_empty() && part.chars().count() <= MAX_WORD_CHARS)
                .collect();
            for part in &parts {
                found.push(Occurrence { word: part.clone(), position: next });
                next += 1;
            }
            if parts.len() > 1 {
                joins.push((parts.concat(), piece_start));
            }
            token_parts.extend(parts);
        }
        // With one piece, the token's joined form is that piece's, already
        // added; checking only this case keeps a minified line (one "token" of
        // thousands of pieces) linear instead of quadratic.
        if pieces > 1 && token_parts.len() > 1 {
            joins.push((token_parts.concat(), token_start));
        }
        for (word, position) in joins {
            if word.chars().count() <= MAX_WORD_CHARS {
                found.push(Occurrence { word, position });
            }
        }
    }
    found
}

/// The words of one document, ready for the inverted index: each word with
/// every position it occurs at (ascending), and the document's length.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DocumentTerms {
    pub positions: HashMap<String, Vec<u32>>,
    /// How many positions the document has. BM25 uses it to discount long
    /// documents: a word is weaker evidence in a 50-page PDF than in a note.
    pub length: u32,
}

/// Split `text` into [`DocumentTerms`].
pub fn document_terms(text: &str) -> DocumentTerms {
    let mut terms = DocumentTerms::default();
    for occurrence in occurrences(text) {
        terms.length = terms.length.max(occurrence.position + 1);
        terms.positions.entry(occurrence.word).or_default().push(occurrence.position);
    }
    for positions in terms.positions.values_mut() {
        positions.sort_unstable();
        positions.dedup();
    }
    terms
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(text: &str) -> Vec<(String, u32)> {
        occurrences(text).into_iter().map(|o| (o.word, o.position)).collect()
    }

    fn p(word: &str, position: u32) -> (String, u32) {
        (word.to_string(), position)
    }

    #[test]
    fn plain_words_get_consecutive_positions() {
        assert_eq!(pairs("The attention mechanism"), vec![p("the", 0), p("attention", 1), p("mechanism", 2)]);
    }

    #[test]
    fn code_identifiers_split_like_names() {
        assert_eq!(
            pairs("getUserName(id)"),
            vec![p("get", 0), p("user", 1), p("name", 2), p("id", 3), p("getusername", 0), p("getusernameid", 0)]
        );
    }

    #[test]
    fn letters_and_digits_split_and_the_whole_stays_findable() {
        assert_eq!(pairs("cse332 notes"), vec![p("cse", 0), p("332", 1), p("cse332", 0), p("notes", 2)]);
    }

    #[test]
    fn punctuation_splits_and_each_piece_and_token_is_joined() {
        assert_eq!(
            pairs("final-report_v2.pdf"),
            vec![
                p("final", 0),
                p("report", 1),
                p("v", 2),
                p("2", 3),
                p("pdf", 4),
                p("v2", 2),
                p("finalreportv2pdf", 0),
            ]
        );
    }

    #[test]
    fn a_phrase_across_punctuation_stays_adjacent() {
        let terms = document_terms("state-of-the-art attention");
        let at = |word: &str| terms.positions[word].clone();
        assert_eq!(at("state"), vec![0]);
        assert_eq!(at("art"), vec![3]);
        assert_eq!(at("attention"), vec![4]);
    }

    #[test]
    fn the_ascii_fast_path_agrees_with_full_folding() {
        for part in ["Kafka", "getUserName", "X11", "ABC", "a"] {
            assert_eq!(fold_word(part), fold(part), "{part}");
        }
    }

    #[test]
    fn accents_and_case_fold_away() {
        assert_eq!(pairs("Café Über"), vec![p("cafe", 0), p("uber", 1)]);
    }

    #[test]
    fn a_huge_token_is_split_in_linear_time() {
        // Minified code: one whitespace-free token of 100,000 pieces.
        let minified = "a.".repeat(100_000);
        let started = std::time::Instant::now();
        assert_eq!(occurrences(&minified).len(), 100_000);
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "took {:?}", started.elapsed());
    }

    #[test]
    fn encoded_blobs_are_not_indexed() {
        let blob = "a".repeat(MAX_WORD_CHARS + 1);
        assert_eq!(pairs(&format!("key {blob} end")), vec![p("key", 0), p("end", 1)]);
    }

    #[test]
    fn document_terms_collect_positions_and_length() {
        let terms = document_terms("kafka streams and kafka connect");
        assert_eq!(terms.positions["kafka"], vec![0, 3]);
        assert_eq!(terms.positions["streams"], vec![1]);
        assert_eq!(terms.length, 5);
        assert_eq!(document_terms("").length, 0);
    }
}
