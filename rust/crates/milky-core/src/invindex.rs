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
use std::cmp::Ordering;
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
/// each once, in query order. The query's last part is a
/// [`QueryTerm::Prefix`], the word being typed, unless the query ends in a
/// space (the word is finished). `"kafka strea"` -> `Word("kafka")`,
/// `Prefix("strea")`; `"cse33"` -> `Word("cse")`, `Prefix("33")`.
///
/// Only parts, never joined forms (`cse332`, at the position of its first
/// part). Documents index both, so the parts alone find `cse332`, `cse-332`,
/// and `CSE 332`; requiring the joined form too would miss `CSE 332`. A
/// query typed as one lowercase word (`getuser`) is a single part and still
/// finds `getUser` through the document's joined form.
pub fn query_terms(query: &str) -> Vec<QueryTerm> {
    let mut parts: Vec<String> = Vec::new();
    for occurrence in occurrences(query) {
        // Parts take positions 0, 1, 2, ...; a joined form repeats an earlier one.
        if occurrence.position as usize == parts.len() {
            parts.push(occurrence.word);
        }
    }
    let typing = !query.ends_with(char::is_whitespace);
    let last = parts.len().saturating_sub(1);
    let mut terms: Vec<QueryTerm> = Vec::new();
    for (i, part) in parts.into_iter().enumerate() {
        let term = if typing && i == last { QueryTerm::Prefix(part) } else { QueryTerm::Word(part) };
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
        let mut words: Vec<&str> = self
            .terms
            .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
            .take_while(|(word, _)| word.starts_with(prefix))
            .map(|(word, _)| word.as_str())
            .collect();
        if words.len() > limit {
            words.sort_by_key(|word| (word.len(), *word));
            words.truncate(limit);
            words.sort();
        }
        words
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
        match term {
            QueryTerm::Word(word) => self.postings(word).cloned().collect(),
            QueryTerm::Prefix(prefix) => {
                let mut merged: BTreeMap<DocId, Vec<u32>> = BTreeMap::new();
                for word in self.words_with_prefix(prefix, MAX_PREFIX_WORDS) {
                    for posting in self.postings(word) {
                        merged.entry(posting.doc).or_default().extend_from_slice(&posting.positions);
                    }
                }
                merged
                    .into_iter()
                    .map(|(doc, mut positions)| {
                        positions.sort_unstable();
                        positions.dedup();
                        Posting { doc, positions }
                    })
                    .collect()
            }
        }
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
        let lists: Vec<Vec<Posting>> = terms.iter().map(|term| self.term_postings(term)).collect();
        Self::intersect_all(&lists)
    }

    /// [`InvertedIndex::matching_docs`] over each term's postings, already
    /// looked up.
    fn intersect_all(lists: &[Vec<Posting>]) -> Vec<(DocId, Vec<Vec<u32>>)> {
        if lists.is_empty() || lists.iter().any(|list| list.is_empty()) {
            return Vec::new();
        }

        let mut order: Vec<usize> = (0..lists.len()).collect();
        order.sort_by_key(|&i| lists[i].len());

        let mut docs: Vec<DocId> = lists[order[0]].iter().map(|posting| posting.doc).collect();
        for &i in &order[1..] {
            docs = Self::intersect(&docs, &lists[i]);
            if docs.is_empty() {
                return Vec::new();
            }
        }

        docs.into_iter()
            .filter_map(|doc| {
                lists
                    .iter()
                    .map(|list| {
                        let at = list.binary_search_by_key(&doc, |posting| posting.doc).ok()?;
                        Some(list[at].positions.clone())
                    })
                    .collect::<Option<Vec<_>>>()
                    .map(|positions| (doc, positions))
            })
            .collect()
    }

    /// The number of postings stored for `word`, dead documents' included.
    #[cfg(test)]
    fn stored_postings(&self, word: &str) -> usize {
        self.terms.get(word).map_or(0, Vec::len)
    }

    /// The documents in both `docs` and `list`, walking the two sorted lists together.
    fn intersect(docs: &[DocId], list: &[Posting]) -> Vec<DocId> {
        let (mut a, mut b) = (0, 0);
        let mut both = Vec::new();
        while a < docs.len() && b < list.len() {
            match docs[a].cmp(&list[b].doc) {
                Ordering::Less => a += 1,
                Ordering::Greater => b += 1,
                Ordering::Equal => {
                    both.push(docs[a]);
                    a += 1;
                    b += 1;
                }
            }
        }
        both
    }

    /// The best `limit` documents containing every term, best first (see
    /// [`ContentHit`] for the order).
    ///
    /// Each match is scored by BM25: for every term, its weight
    /// ([`bm25_weight`], high for rare words) times how strongly this document
    /// is about it ([`bm25_saturation`]: more occurrences count less and less,
    /// and the same count means more in a short document), summed over the
    /// terms. A prefix counts as one term: its merged postings give its count
    /// per document and how many documents it is in.
    pub fn search(&self, terms: &[QueryTerm], limit: usize) -> Vec<ContentHit> {
        if limit == 0 {
            return Vec::new();
        }
        let lists: Vec<Vec<Posting>> = terms.iter().map(|term| self.term_postings(term)).collect();
        let documents = self.len() as f64;
        let average = self.average_length();
        let weights: Vec<f64> = lists.iter().map(|list| bm25_weight(documents, list.len() as f64)).collect();
        let mut hits: Vec<ContentHit> = Self::intersect_all(&lists)
            .into_iter()
            .map(|(doc, positions)| {
                let length = self.doc(doc).map_or(0.0, |info| f64::from(info.length));
                let score = positions
                    .iter()
                    .zip(&weights)
                    .map(|(found, weight)| weight * bm25_saturation(found.len() as f64, length, average))
                    .sum();
                let phrase = phrase_start(&positions);
                ContentHit { doc, score, phrase: phrase.is_some(), at: phrase.unwrap_or_else(|| tightest_window(&positions)) }
            })
            .collect();
        if hits.len() > limit {
            hits.select_nth_unstable_by(limit - 1, ContentHit::rank);
            hits.truncate(limit);
        }
        hits.sort_by(ContentHit::rank);
        hits
    }
}

/// How quickly repeats stop adding to a word's score (BM25's `k1`). At 1.2, in
/// a document of average length, one occurrence scores 1, two 1.38, three
/// 1.57, and no count ever passes 2.2: a page that says `kafka` forty times
/// is not forty times about Kafka.
pub const BM25_K1: f64 = 1.2;

/// How much document length discounts a count (BM25's `b`): 0 ignores
/// length, 1 divides by it fully. A word is weaker evidence in a 50-page PDF
/// than in a three-line note.
pub const BM25_B: f64 = 0.75;

/// A term's weight, given how many of `documents` contain it: about
/// `ln(documents / containing)`, so a word in every document weighs almost
/// nothing and a word in one of 2,000 weighs about 7.2. The `+ 0.5`s keep it
/// finite, and the `1 +` keeps it above zero (Lucene's form of BM25's IDF).
pub fn bm25_weight(documents: f64, containing: f64) -> f64 {
    (1.0 + (documents - containing + 0.5) / (containing + 0.5)).ln()
}

/// How strongly a document with `count` occurrences of a term, `length`
/// positions long, is about it, between 0 and `BM25_K1 + 1`:
/// `count * (k1 + 1) / (count + k1 * (1 - b + b * length / average))`.
pub fn bm25_saturation(count: f64, length: f64, average: f64) -> f64 {
    let stretch = if average > 0.0 { length / average } else { 1.0 };
    count * (BM25_K1 + 1.0) / (count + BM25_K1 * (1.0 - BM25_B + BM25_B * stretch))
}

/// One document matching every term of a content search.
#[derive(Debug, Clone, PartialEq)]
pub struct ContentHit {
    pub doc: DocId,
    /// The BM25 score: higher is better.
    pub score: f64,
    /// Whether the terms occur side by side somewhere, in query order (always
    /// true for one term).
    pub phrase: bool,
    /// Where a snippet should show the match: the first phrase occurrence,
    /// or else the start of the shortest stretch holding every term.
    pub at: u32,
}

impl ContentHit {
    /// Best first: a phrase above scattered words (`attention mechanism`
    /// written together beats a long paper that uses both words apart), then
    /// a higher score, then the lower number, so equal hits keep a fixed order.
    fn rank(a: &ContentHit, b: &ContentHit) -> Ordering {
        b.phrase.cmp(&a.phrase).then(b.score.total_cmp(&a.score)).then(a.doc.cmp(&b.doc))
    }
}

/// The first position where term 0 sits at `p`, term 1 at `p + 1`, and so on,
/// given each term's sorted positions in one document.
fn phrase_start(positions: &[Vec<u32>]) -> Option<u32> {
    let (first, rest) = positions.split_first()?;
    first.iter().copied().find(|&start| {
        rest.iter().zip(1u32..).all(|(list, offset)| list.binary_search(&(start + offset)).is_ok())
    })
}

/// The start of the shortest stretch of a document holding at least one
/// occurrence of every term, given each term's sorted positions (none empty).
///
/// Keep one pointer per list. The stretch the pointers mark runs from the
/// lowest position to the highest. Any shorter stretch starting at that
/// lowest position would have to leave out its own term, so the only way to
/// find a shorter one is to move the lowest pointer forward. Repeat until a
/// list runs out: every list's positions are visited once.
fn tightest_window(positions: &[Vec<u32>]) -> u32 {
    let mut next = vec![0usize; positions.len()];
    let (mut best_span, mut best_start) = (u32::MAX, 0);
    loop {
        let (mut low, mut low_list, mut high) = (u32::MAX, 0, 0);
        for (i, list) in positions.iter().enumerate() {
            let position = list[next[i]];
            if position < low {
                low = position;
                low_list = i;
            }
            high = high.max(position);
        }
        if high - low < best_span {
            best_span = high - low;
            best_start = low;
        }
        next[low_list] += 1;
        if next[low_list] == positions[low_list].len() {
            return best_start;
        }
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
    fn a_typed_identifier_is_matched_by_its_parts() {
        assert_eq!(query_terms("getUser"), vec![word("get"), prefix("user")]);
        assert_eq!(query_terms("notes cse332"), vec![word("notes"), word("cse"), prefix("332")]);
        assert_eq!(query_terms("getuser"), vec![prefix("getuser")], "one lowercase part");
    }

    #[test]
    fn a_trailing_space_finishes_the_last_word() {
        assert_eq!(query_terms("kafka "), vec![word("kafka")]);
        assert_eq!(query_terms("kafka kafka"), vec![word("kafka"), prefix("kafka")]);
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

    fn found(index: &InvertedIndex, query: &str) -> Vec<String> {
        index
            .search(&query_terms(query), 10)
            .into_iter()
            .map(|hit| index.doc(hit.doc).unwrap().path.to_string_lossy().into_owned())
            .collect()
    }

    fn index_of(documents: &[(&str, &str)]) -> InvertedIndex {
        let mut index = InvertedIndex::new();
        for (path, text) in documents {
            index.add(Path::new(path), document_terms(text));
        }
        index
    }

    #[test]
    fn scores_follow_the_bm25_formula() {
        // streams.md: kafka twice in 5 positions; notes.txt: once in 4;
        // todo.txt: none, 2 positions. Average length 11/3.
        let index = sample();
        let hits = index.search(&[word("kafka")], 10);
        let weight = (1.0_f64 + (3.0 - 2.0 + 0.5) / (2.0 + 0.5)).ln();
        let strength = |count: f64, length: f64| count * 2.2 / (count + 1.2 * (0.25 + 0.75 * length / (11.0 / 3.0)));
        assert_eq!(hits.iter().map(|hit| hit.doc).collect::<Vec<_>>(), vec![0, 1]);
        assert!((hits[0].score - weight * strength(2.0, 5.0)).abs() < 1e-9);
        assert!((hits[1].score - weight * strength(1.0, 4.0)).abs() < 1e-9);
    }

    #[test]
    fn rarer_words_weigh_more() {
        let index = index_of(&[
            ("/h/more-common.md", "alpha alpha beta"),
            ("/h/more-rare.md", "alpha beta beta"),
            ("/h/c.md", "alpha"),
            ("/h/d.md", "alpha"),
        ]);
        assert_eq!(found(&index, "alpha beta "), vec!["/h/more-rare.md", "/h/more-common.md"]);
        assert!(bm25_weight(4.0, 2.0) > bm25_weight(4.0, 4.0));
        assert!(bm25_weight(4.0, 4.0) > 0.0, "never negative, even in every document");
    }

    #[test]
    fn repeats_add_less_and_less() {
        let one = bm25_saturation(1.0, 10.0, 10.0);
        let two = bm25_saturation(2.0, 10.0, 10.0);
        let three = bm25_saturation(3.0, 10.0, 10.0);
        assert!((one - 1.0).abs() < 1e-9, "one occurrence at average length scores 1");
        assert!(two - one > three - two && three > two);
        assert!(bm25_saturation(1_000.0, 10.0, 10.0) < BM25_K1 + 1.0);
    }

    #[test]
    fn the_same_count_means_more_in_a_shorter_document() {
        let index = index_of(&[
            ("/h/long.md", "kafka and a great many other words that dilute it here"),
            ("/h/short.md", "kafka notes"),
        ]);
        assert_eq!(found(&index, "kafka "), vec!["/h/short.md", "/h/long.md"]);
    }

    #[test]
    fn a_phrase_ranks_above_scattered_words() {
        let index = index_of(&[
            ("/h/scattered.md", "attention attention is all about the mechanism mechanism"),
            ("/h/phrase.md", "the attention mechanism explained in a few more words"),
        ]);
        let hits = index.search(&query_terms("attention mechanism"), 10);
        assert_eq!(found(&index, "attention mechanism"), vec!["/h/phrase.md", "/h/scattered.md"]);
        assert!(hits[0].phrase && !hits[1].phrase);
        assert!(hits[1].score > hits[0].score, "the scattered one scores higher, yet ranks lower");
        assert!(index.search(&query_terms("mechanism attention"), 10).iter().all(|hit| !hit.phrase), "order matters");
    }

    #[test]
    fn the_snippet_goes_at_the_phrase_or_the_tightest_stretch() {
        // mechanism@0 ... attention@5 ... mechanism@9
        let index = index_of(&[("/h/a.md", "mechanism and then much later attention far from the mechanism")]);
        let hit = &index.search(&query_terms("attention mechanism"), 10)[0];
        assert!(!hit.phrase);
        assert_eq!(hit.at, 5, "attention@5..mechanism@9 is shorter than mechanism@0..attention@5");

        let index = index_of(&[("/h/b.md", "kafka streams then kafka streams again")]);
        let hit = &index.search(&query_terms("kafka streams"), 10)[0];
        assert!(hit.phrase);
        assert_eq!(hit.at, 0, "the first occurrence of the phrase");
        assert_eq!(tightest_window(&[vec![0, 9], vec![5]]), 5);
        assert_eq!(tightest_window(&[vec![3]]), 3);
    }

    #[test]
    fn only_the_best_few_are_kept_in_order() {
        let documents: Vec<(String, String)> =
            (1..=12).map(|i| (format!("/h/{i}.md"), format!("{} filler words", "kafka ".repeat(i)))).collect();
        let pairs: Vec<(&str, &str)> = documents.iter().map(|(p, t)| (p.as_str(), t.as_str())).collect();
        let index = index_of(&pairs);
        let all = index.search(&query_terms("kafka"), 100);
        assert_eq!(all.len(), 12);
        assert_eq!(index.search(&query_terms("kafka"), 3), all[..3].to_vec());
        assert!(all.windows(2).all(|pair| pair[0].score >= pair[1].score));
        assert!(index.search(&query_terms("kafka"), 0).is_empty());
    }

    #[test]
    fn a_course_number_is_found_however_it_was_written() {
        let index = index_of(&[
            ("/h/joined.md", "cse332 midterm"),
            ("/h/spaced.md", "CSE 332 midterm"),
            ("/h/dashed.md", "cse-332 midterm"),
            ("/h/other.md", "cse 351 midterm"),
        ]);
        let mut found = found(&index, "cse332");
        found.sort();
        assert_eq!(found, vec!["/h/dashed.md", "/h/joined.md", "/h/spaced.md"]);
        assert_eq!(index.search(&query_terms("cse332"), 10).iter().filter(|hit| hit.phrase).count(), 3);
    }

    #[test]
    fn a_word_being_typed_already_finds_its_documents() {
        let index = words_index();
        assert_eq!(found(&index, "kafka strea"), vec!["/h/a.md"]);
        assert_eq!(found(&index, "getuser"), Vec::<String>::new());
        assert!(index.search(&[], 10).is_empty());
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
