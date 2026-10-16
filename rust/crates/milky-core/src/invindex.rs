//! The inverted index of document contents (DECISIONS 2026-10-16, document
//! search, beat D3).
//!
//! For every word, the documents containing it and the positions it occurs
//! at: `"kafka" -> [doc 3 at [12, 40], doc 7 at [2]]`. A query looks up only
//! its own words' lists instead of reading 20 million characters of text.
//!
//! Documents are numbered (`DocId`), in the order they are added, and a
//! number is never reused, so every word's list stays sorted by number simply
//! by appending. Removing a document only marks its number as dead (a
//! tombstone): queries skip dead numbers, and [`InvertedIndex::compact`]
//! sweeps their entries out of the lists now and then.

use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;
use std::path::{Path, PathBuf};

use crate::terms::{DocumentTerms, occurrences};

/// A document's number in the index. Small and dense: lists of them are
/// compact, compare fast, and keep their order by appending.
pub type DocId = u32;

/// How many dictionary words a word being typed may stand for. `a` starts
/// thousands of words; past this many, the shortest are kept (the words a
/// person is most likely still typing toward).
pub const MAX_PREFIX_WORDS: usize = 64;

/// A query word as the index matches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryTerm {
    /// A finished word: exactly this word.
    Word(String),
    /// The word still being typed: any word starting with it.
    Prefix(String),
}

/// The terms of a query, split exactly as documents are ([`occurrences`]),
/// each once. Everything from the query's last whitespace-separated word (its
/// parts and joined forms) is a [`QueryTerm::Prefix`]: that is the word being
/// typed. `"kafka strea"` -> `Word("kafka")`, `Prefix("strea")`.
pub fn query_terms(query: &str) -> Vec<QueryTerm> {
    let found = occurrences(query);
    let last_token_start = query
        .split_whitespace()
        .last()
        .map_or(0, |last| occurrences(&query[..query.len() - last.len()]).iter().map(|o| o.position + 1).max().unwrap_or(0));
    let mut terms: Vec<QueryTerm> = Vec::new();
    for occurrence in found {
        let term = if occurrence.position >= last_token_start {
            QueryTerm::Prefix(occurrence.word)
        } else {
            QueryTerm::Word(occurrence.word)
        };
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    terms
}

/// One document's occurrences of one word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posting {
    pub doc: DocId,
    /// Where the word occurs in the document, ascending.
    pub positions: Vec<u32>,
}

/// What the index knows about a live document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocInfo {
    pub path: PathBuf,
    /// How many positions it has (see [`DocumentTerms::length`]).
    pub length: u32,
}

#[derive(Debug, Default)]
pub struct InvertedIndex {
    /// Every word, sorted (so the words starting with a prefix are one range,
    /// for queries typed partway), with its postings sorted by `DocId`.
    terms: BTreeMap<String, Vec<Posting>>,
    /// Indexed by `DocId`: the document, or `None` once it was removed.
    docs: Vec<Option<DocInfo>>,
    /// The `DocId` of every live document, by path.
    ids: HashMap<PathBuf, DocId>,
    /// Sum of the live documents' lengths, for the average BM25 needs.
    total_length: u64,
    /// Documents removed since the last [`InvertedIndex::compact`]: their
    /// postings are still in the lists.
    removed: usize,
}

impl InvertedIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many live documents.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The average length of the live documents (0 when there are none).
    pub fn average_length(&self) -> f64 {
        if self.ids.is_empty() { 0.0 } else { self.total_length as f64 / self.ids.len() as f64 }
    }

    /// The live document numbered `id`, or `None` if removed or unknown.
    pub fn doc(&self, id: DocId) -> Option<&DocInfo> {
        self.docs.get(id as usize)?.as_ref()
    }

    pub fn id_of(&self, path: &Path) -> Option<DocId> {
        self.ids.get(path).copied()
    }

    /// The live postings of `word`, in `DocId` order. Dead documents'
    /// postings, not yet compacted away, are skipped here.
    pub fn postings<'a>(&'a self, word: &str) -> impl Iterator<Item = &'a Posting> + 'a {
        self.terms
            .get(word)
            .into_iter()
            .flatten()
            .filter(|posting| self.doc(posting.doc).is_some())
    }

    /// How many distinct words the dictionary holds (including words whose
    /// only documents were removed, until compaction).
    pub fn word_count(&self) -> usize {
        self.terms.len()
    }

    /// Documents removed since the last compaction.
    pub fn removed(&self) -> usize {
        self.removed
    }

    /// Whether enough documents were removed that compacting is worth it:
    /// more than a quarter of the live count. Until then, skipping their
    /// postings costs less than rewriting every list.
    pub fn should_compact(&self) -> bool {
        self.removed > self.len() / 4
    }

    /// Add the document at `path` with its words, and return its new number.
    ///
    /// If `path` is already indexed (the file changed), remove the old version
    /// first: a changed document gets a fresh number, never its old one.
    ///
    /// The new number is the next unused one (`self.docs.len()`), so it is
    /// larger than every number in every list: appending it to each word's
    /// list (`self.terms.entry(word).or_default().push(...)`) keeps the list
    /// sorted. Record the document in `docs` and `ids`, and add its length
    /// to `total_length`.
    pub fn add(&mut self, path: &Path, terms: DocumentTerms) -> DocId {
        self.remove(path);
        let id = self.docs.len() as DocId;
        self.docs.push(Some(DocInfo { path: path.to_path_buf(), length: terms.length }));
        self.ids.insert(path.to_path_buf(), id);
        self.total_length += u64::from(terms.length);
        for (word, positions) in terms.positions {
            self.terms.entry(word).or_default().push(Posting { doc: id, positions });
        }
        id
    }

    /// Remove the document at `path`. Returns whether it was indexed.
    ///
    /// A tombstone, not a deletion: take its entry out of `ids`, set its slot
    /// in `docs` to `None` (`Option::take`), subtract its length from
    /// `total_length`, and count it in `removed`. Its postings stay in the
    /// lists, where `postings` now skips them. Finding them would mean
    /// visiting every list the document's words are in, and the index does
    /// not keep a document's words.
    pub fn remove(&mut self, path: &Path) -> bool {
        let Some(id) = self.ids.remove(path) else {
            return false;
        };
        if let Some(info) = self.docs[id as usize].take() {
            self.total_length -= u64::from(info.length);
        }
        self.removed += 1;
        true
    }

    /// Sweep removed documents' postings out of every list, drop words left
    /// with no postings, and reset `removed`. Afterwards every posting belongs
    /// to a live document. `Vec::retain` and `BTreeMap::retain` keep the
    /// entries a closure approves of, in order.
    pub fn compact(&mut self) {
        let docs = &self.docs;
        self.terms.retain(|_, postings| {
            postings.retain(|posting| docs[posting.doc as usize].is_some());
            !postings.is_empty()
        });
        self.removed = 0;
    }

    /// The dictionary words starting with `prefix`, in dictionary order; if
    /// more than `limit`, the `limit` shortest (ties in dictionary order).
    ///
    /// The dictionary is sorted, so the words starting with `prefix` are one
    /// unbroken run: `self.terms.range::<str, _>((Bound::Included(prefix),
    /// Bound::Unbounded))` starts at the first one, and `take_while` stops at
    /// the first word that does not start with it, as `FileIndex::children`
    /// does with paths.
    pub fn words_with_prefix(&self, prefix: &str, limit: usize) -> Vec<&str> {
        todo!()
    }

    /// The live postings matching one term, in `DocId` order: a word's own
    /// postings, or for a prefix, the postings of every word it stands for
    /// ([`InvertedIndex::words_with_prefix`], at most [`MAX_PREFIX_WORDS`])
    /// merged into one posting per document, positions combined and sorted.
    ///
    /// A document with `kafka` at 3 and `kafkaesque` at 9 gives one posting
    /// with positions `[3, 9]` for the prefix `kafka`. Collecting into a
    /// `BTreeMap<DocId, Vec<u32>>` keeps documents in order for you.
    pub fn term_postings(&self, term: &QueryTerm) -> Vec<Posting> {
        todo!()
    }

    /// The documents containing every term, in `DocId` order, each with its
    /// positions for every term (in the order of `terms`). Empty if any term
    /// matches nothing, or `terms` is empty.
    ///
    /// Intersect sorted lists the way merge sort merges them: walk two lists
    /// side by side, advance whichever is behind, keep a document when both
    /// point at it. Start from the shortest list: the result can be no
    /// longer, and every later step only shrinks it.
    pub fn matching_docs(&self, terms: &[QueryTerm]) -> Vec<(DocId, Vec<Vec<u32>>)> {
        todo!()
    }

    /// The number of postings stored for `word`, dead documents' included.
    #[cfg(test)]
    fn stored_postings(&self, word: &str) -> usize {
        self.terms.get(word).map_or(0, Vec::len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terms::document_terms;

    fn docs_with(index: &InvertedIndex, word: &str) -> Vec<(String, Vec<u32>)> {
        index
            .postings(word)
            .map(|posting| {
                let path = index.doc(posting.doc).unwrap().path.to_string_lossy().into_owned();
                (path, posting.positions.clone())
            })
            .collect()
    }

    fn sample() -> InvertedIndex {
        let mut index = InvertedIndex::new();
        index.add(Path::new("/h/streams.md"), document_terms("kafka streams and kafka connect"));
        index.add(Path::new("/h/notes.txt"), document_terms("zookeeper keeps kafka brokers"));
        index.add(Path::new("/h/todo.txt"), document_terms("buy milk"));
        index
    }

    #[test]
    fn each_word_lists_its_documents_and_positions() {
        let index = sample();
        assert_eq!(
            docs_with(&index, "kafka"),
            vec![("/h/streams.md".to_string(), vec![0, 3]), ("/h/notes.txt".to_string(), vec![2])]
        );
        assert_eq!(docs_with(&index, "milk"), vec![("/h/todo.txt".to_string(), vec![1])]);
        assert!(docs_with(&index, "spark").is_empty());
        assert_eq!(index.len(), 3);
    }

    #[test]
    fn numbers_follow_insertion_and_lists_stay_sorted() {
        let mut index = InvertedIndex::new();
        let a = index.add(Path::new("/h/a.txt"), document_terms("shared"));
        let b = index.add(Path::new("/h/b.txt"), document_terms("shared"));
        let c = index.add(Path::new("/h/c.txt"), document_terms("shared"));
        assert_eq!((a, b, c), (0, 1, 2));
        let numbers: Vec<DocId> = index.postings("shared").map(|posting| posting.doc).collect();
        assert_eq!(numbers, vec![0, 1, 2]);
        assert_eq!(index.id_of(Path::new("/h/b.txt")), Some(1));
    }

    #[test]
    fn lengths_and_their_average_are_kept() {
        let index = sample();
        assert_eq!(index.doc(0).unwrap().length, 5);
        assert!((index.average_length() - (5.0 + 4.0 + 2.0) / 3.0).abs() < 1e-9);
        assert_eq!(InvertedIndex::new().average_length(), 0.0);
    }

    #[test]
    fn a_removed_document_disappears_from_results_at_once() {
        let mut index = sample();
        assert!(index.remove(Path::new("/h/streams.md")));
        assert_eq!(docs_with(&index, "kafka"), vec![("/h/notes.txt".to_string(), vec![2])]);
        assert_eq!(index.len(), 2);
        assert_eq!(index.doc(0), None);
        assert_eq!(index.id_of(Path::new("/h/streams.md")), None);
        assert!((index.average_length() - (4.0 + 2.0) / 2.0).abs() < 1e-9);
        assert!(!index.remove(Path::new("/h/streams.md")), "already gone");
        assert!(!index.remove(Path::new("/h/never.txt")));
    }

    #[test]
    fn removal_leaves_a_tombstone_until_compaction() {
        let mut index = sample();
        index.remove(Path::new("/h/streams.md"));
        assert_eq!(index.removed(), 1);
        assert_eq!(index.stored_postings("kafka"), 2, "still stored, just skipped");
        index.compact();
        assert_eq!(index.removed(), 0);
        assert_eq!(index.stored_postings("kafka"), 1);
        assert_eq!(index.stored_postings("streams"), 0);
        assert_eq!(docs_with(&index, "kafka"), vec![("/h/notes.txt".to_string(), vec![2])], "results unchanged");
    }

    #[test]
    fn compaction_drops_words_no_document_has_left() {
        let mut index = sample();
        let before = index.word_count();
        index.remove(Path::new("/h/todo.txt"));
        assert_eq!(index.word_count(), before, "a tombstone keeps its words for now");
        index.compact();
        assert_eq!(index.word_count(), before - 2, "buy and milk are gone");
        assert!(docs_with(&index, "milk").is_empty());
    }

    #[test]
    fn a_changed_document_replaces_its_old_version() {
        let mut index = sample();
        let old = index.id_of(Path::new("/h/todo.txt")).unwrap();
        let new = index.add(Path::new("/h/todo.txt"), document_terms("buy oat milk"));
        assert_ne!(old, new, "a fresh number, never the old one");
        assert_eq!(index.len(), 3);
        assert_eq!(docs_with(&index, "milk"), vec![("/h/todo.txt".to_string(), vec![2])]);
        assert_eq!(docs_with(&index, "oat"), vec![("/h/todo.txt".to_string(), vec![1])]);
        assert_eq!(index.removed(), 1);
    }

    #[test]
    fn numbers_are_never_reused_even_after_compaction() {
        let mut index = sample();
        index.remove(Path::new("/h/todo.txt"));
        index.compact();
        assert_eq!(index.add(Path::new("/h/new.txt"), document_terms("fresh")), 3);
    }

    #[test]
    fn compaction_is_suggested_once_a_quarter_is_dead() {
        let mut index = InvertedIndex::new();
        for i in 0..8 {
            index.add(Path::new(&format!("/h/{i}.txt")), document_terms("word"));
        }
        index.remove(Path::new("/h/0.txt"));
        assert!(!index.should_compact());
        index.remove(Path::new("/h/1.txt"));
        index.remove(Path::new("/h/2.txt"));
        assert!(index.should_compact(), "3 dead against 5 live");
    }

    fn word(w: &str) -> QueryTerm {
        QueryTerm::Word(w.to_string())
    }

    fn prefix(w: &str) -> QueryTerm {
        QueryTerm::Prefix(w.to_string())
    }

    #[test]
    fn the_last_word_of_a_query_is_the_one_being_typed() {
        assert_eq!(query_terms("kafka strea"), vec![word("kafka"), prefix("strea")]);
        assert_eq!(query_terms("kaf"), vec![prefix("kaf")]);
        assert_eq!(query_terms("attention mechanism"), vec![word("attention"), prefix("mechanism")]);
        assert!(query_terms("").is_empty());
    }

    #[test]
    fn a_typed_identifier_matches_by_all_its_parts_as_prefixes() {
        assert_eq!(query_terms("getUser"), vec![prefix("get"), prefix("user"), prefix("getuser")]);
        assert_eq!(query_terms("notes cse332"), vec![word("notes"), prefix("cse"), prefix("332"), prefix("cse332")]);
    }

    fn words_index() -> InvertedIndex {
        let mut index = InvertedIndex::new();
        index.add(Path::new("/h/a.md"), document_terms("kafka streams kafkaesque"));
        index.add(Path::new("/h/b.md"), document_terms("kafka connectors and kalman filters"));
        index.add(Path::new("/h/c.md"), document_terms("streams of consciousness"));
        index
    }

    #[test]
    fn a_prefix_finds_its_run_of_the_dictionary() {
        let index = words_index();
        assert_eq!(index.words_with_prefix("kaf", 10), vec!["kafka", "kafkaesque"]);
        assert_eq!(index.words_with_prefix("ka", 10), vec!["kafka", "kafkaesque", "kalman"]);
        assert_eq!(index.words_with_prefix("ka", 2), vec!["kafka", "kalman"], "the shortest when over the limit");
        assert!(index.words_with_prefix("zebra", 10).is_empty());
        assert_eq!(index.words_with_prefix("kafka", 10), vec!["kafka", "kafkaesque"], "a whole word is its own prefix");
    }

    #[test]
    fn a_prefix_merges_its_words_into_one_posting_per_document() {
        let index = words_index();
        let postings = index.term_postings(&prefix("kafka"));
        assert_eq!(
            postings,
            vec![Posting { doc: 0, positions: vec![0, 2] }, Posting { doc: 1, positions: vec![0] }]
        );
        assert_eq!(index.term_postings(&word("kafka")).len(), 2);
        assert!(index.term_postings(&word("kaf")).is_empty(), "a finished word must match exactly");
    }

    #[test]
    fn every_term_must_be_in_the_document() {
        let index = words_index();
        let docs = |terms: &[QueryTerm]| -> Vec<DocId> { index.matching_docs(terms).into_iter().map(|(doc, _)| doc).collect() };
        assert_eq!(docs(&[word("kafka")]), vec![0, 1]);
        assert_eq!(docs(&[word("kafka"), prefix("stre")]), vec![0]);
        assert_eq!(docs(&[word("streams")]), vec![0, 2]);
        assert!(docs(&[word("kafka"), word("consciousness")]).is_empty());
        assert!(docs(&[word("kafka"), word("spark")]).is_empty(), "a term matching nothing matches nothing");
        assert!(docs(&[]).is_empty());
    }

    #[test]
    fn matches_carry_each_terms_positions_in_query_order() {
        let index = words_index();
        let found = index.matching_docs(&[word("streams"), prefix("kaf")]);
        assert_eq!(found, vec![(0, vec![vec![1], vec![0, 2]])]);
    }

    #[test]
    fn removed_documents_never_match() {
        let mut index = words_index();
        index.remove(Path::new("/h/a.md"));
        let docs: Vec<DocId> = index.matching_docs(&[prefix("kafka")]).into_iter().map(|(doc, _)| doc).collect();
        assert_eq!(docs, vec![1]);
    }

    #[test]
    fn a_thousand_documents_index_quickly() {
        let mut index = InvertedIndex::new();
        let started = std::time::Instant::now();
        for i in 0..1_000 {
            let text = format!("document number {i} mentions kafka and topic{} often", i % 10);
            index.add(Path::new(&format!("/h/{i}.md")), document_terms(&text));
        }
        assert_eq!(index.postings("kafka").count(), 1_000);
        assert_eq!(index.postings("topic3").count(), 100);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
}
