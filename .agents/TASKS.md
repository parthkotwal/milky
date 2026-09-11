# Issues

This is the persistent work log for unfinished problems, bugs, investigations, and next steps.

Keep entries concise. Remove or move completed items when they no longer help future sessions.

## Now

- [ ] Define repository structure for Swift app, Rust core, and Python ML workspace.
- [ ] Decide Swift ↔ Rust integration approach.
- [ ] Define the common candidate/result/action data model.
- [ ] Install full Xcode. Needed for `swift test` (XCTest ships with Xcode, not
      with the Command Line Tools) and for a real `.app` bundle.
- [ ] Define the query and selection event schema.
- [ ] Choose the first lexical retrieval implementation.
- [ ] Benchmark candidate local text embedding models.
- [ ] Benchmark candidate local image embedding models.
- [ ] Decide local model inference runtime.
- [ ] Define initial indexing policy and supported document types.
- [ ] Design the first unified ranking feature set.
- [ ] Define latency targets and a benchmark harness.

## Soon

- [ ] App discovery and launching. Bundle scanning works; display names are still
      the bundle directory name, which is wrong for some apps (see `ISSUES.md`).
- [ ] Decide how to get correct localized app display names: `core-foundation`
      from Rust, or supplied by Swift.
- [ ] File/folder indexing.
- [ ] Document text extraction.
- [ ] Screenshot OCR.
- [ ] Image embedding index.
- [ ] Semantic text retrieval.
- [ ] System settings/actions source.
- [ ] Web-search fallback.
- [ ] Usage-history persistence.
- [ ] Background filesystem watching and incremental updates.
- [ ] Result previews and secondary actions.

## Later

- [ ] Learned ranking.
- [ ] Contextual personalization.
- [ ] Browser history.
- [ ] Entity graph.
- [ ] Quantized embeddings.
- [ ] Custom or specialized search indexes where justified.
- [ ] Native performance work based on profiling.
- [ ] Broader personal data sources.
