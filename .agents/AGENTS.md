# Agent Instructions

## Project context

This is a long-term passion project, not a quick resume project or a YC-style AI wrapper.

The goal is to build a fully local macOS search and action engine that can eventually replace Spotlight for its user. It should combine the simplicity and speed of Spotlight with richer actions and previews similar to Raycast.

The project should become technically deep over time. Search, ranking, personalization, local ML, multimodal retrieval, indexing, filesystem integration, caching, graphs, and systems work are all part of the intended direction.

The current expected stack is:

- Swift / SwiftUI / AppKit for the macOS UI and native integration
- Rust for the core search, indexing, ranking, storage, and systems-heavy runtime
- Python for ML experimentation, evaluation, and model development
- SQLite for metadata and history where appropriate
- Fully local inference and storage

These choices can change when there is a good technical reason. Record meaningful changes in `.agents/DECISIONS.md`.

## How to work with me

I am a CS student building this partly because I want to learn.

Do not treat me as a product manager who only wants completed code. Treat me as the engineer you are pairing with.

I do not know Rust yet, and I will encounter many search, systems, ML, macOS, and low-level concepts for the first time. Teach them clearly when they become relevant.

When introducing something unfamiliar:

1. Explain what problem it solves in this project.
2. Explain the idea from first principles without assuming I already know the terminology.
3. Show how it fits into the surrounding system.
4. Explain important alternatives and why we are choosing this one.
5. Then implement it with me.

Do not make explanations unnecessarily academic or verbose. Prefer concrete explanations tied to the codebase.

If a concept depends on another concept I may not know, explain that dependency too instead of skipping over it.

## Coding style of collaboration

You are allowed to write substantial code. Do not artificially restrict yourself to tiny patches.

However, distinguish between code that is useful for me to write myself and code that is mostly mechanical.

For important learning moments, especially Rust, search/indexing logic, ranking, concurrency, storage, systems code, and ML:

- explain the implementation before or while writing it;
- point out the parts worth understanding;
- when appropriate, offer me a small meaningful section to write myself;
- review what I write and explain mistakes precisely.

For boilerplate, frontend polish, repetitive integration code, generated bindings, basic configuration, and other low-learning-value work, it is fine to handle more of it directly.

Do not repeatedly stop to ask whether you may proceed. Make reasonable decisions and keep moving. Ask only when a decision meaningfully changes the product or architecture and cannot be inferred from existing project context.

## Engineering goals

Prioritize:

- fast perceived latency;
- fully local operation;
- useful behavior before unnecessary abstraction;
- clear module boundaries;
- measurable ranking and retrieval quality;
- incremental indexing rather than repeated full rebuilds;
- privacy by design;
- code we can understand and evolve;
- instrumentation and benchmarks for performance-sensitive paths.

Do not add ML just because it is available. Use deterministic or lexical methods when they solve the problem better or faster.

Similarly, do not avoid ML where it provides real value. The intended system should eventually use multiple specialized techniques rather than depend on one giant model.

Likely long-term ML areas include:

- semantic text retrieval;
- image and screenshot embeddings;
- query intent classification;
- learning-to-rank;
- personalization from selections;
- contextual ranking;
- multimodal retrieval and reranking;
- optional small local language models for difficult query understanding or reranking.

## Product direction

The initial serious product should support a unified ranked interface across:

- installed applications;
- local files and folders;
- document contents;
- screenshots and images;
- system settings and actions;
- web-search fallback.

The interaction model is hybrid search + action.

Results are not only documents. A result can represent an entity and expose actions such as:

- open;
- open with;
- reveal in Finder;
- launch;
- open in an associated application;
- run a system action;
- search the web.

The system should learn from usage over time. Query/result impressions and selections should be recorded locally so ranking can eventually become personalized.

The interface should feel closer to Spotlight in visual simplicity and invocation speed, while supporting Raycast-like actions, previews, and keyboard interaction.

## Project discipline

Project memory lives in `.agents/`. Each file has one job, and they should not
blur into each other:

- `.agents/ARCHITECTURE.md` — how the system is meant to fit together.
- `.agents/DECISIONS.md` — durable product/architecture decisions, dated, with
  the reasoning and consequences. Append-only in spirit; supersede an old entry
  with a new one rather than quietly rewriting history.
- `.agents/TASKS.md` — the work queue: Now / Soon / Later.
- `.agents/ISSUES.md` — bugs, failures, dead ends, and gotchas we actually hit,
  with cause and lesson. This is how we avoid repeating mistakes across
  sessions. Not a task list.

Before large implementation changes:

- read `.agents/ARCHITECTURE.md`;
- read relevant entries in `.agents/DECISIONS.md`;
- read `.agents/ISSUES.md` for known traps in the area you are touching;
- read `.agents/TASKS.md`.

After meaningful work:

- update `.agents/TASKS.md` (check off, add follow-ups that actually surfaced);
- add to `.agents/ISSUES.md` when we hit a real bug, failure, or gotcha, or when
  we knowingly leave a trap behind. Record the cause and the lesson, not just
  the symptom;
- record durable architecture/product decisions in `.agents/DECISIONS.md`;
- update `.agents/ARCHITECTURE.md` when the actual system changes.

Do not turn these files into diaries. Record information future sessions will genuinely need.

If implementation and documentation disagree, call it out and fix the documentation.

## Scope philosophy

Do not reduce the project to toy milestones merely because they are easy.

The first meaningful version should already express the real thesis of the product: unified local search, semantic retrieval, actions, personalization signals, and fast native interaction.

At the same time, avoid building speculative infrastructure before the product needs it.

Use existing libraries when they let us move forward. Reimplement components later when doing so provides a real learning opportunity, measurable improvement, or important control over the system.

The project is expected to evolve for a long time. Optimize for an architecture we can understand and change, not for pretending we know the final design today.
