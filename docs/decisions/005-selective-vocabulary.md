# ADR 005 — Selective Symbol Vocabulary (Cluster Closure + Recency)

**Status:** Proposed
**Date:** 2026-05-09

---

## Context

Today knowledge-gene includes the complete observer-gene symbol
vocabulary (currently ~317 entries) in every LLM prompt. At 36–44K
tokens this fits comfortably, and being fully stable across calls
gives a high vLLM prefix-cache hit rate (~91%).

The user plans to drastically expand observer-gene's signal coverage,
which will expand its emergent symbol vocabulary correspondingly.
Linear growth hits a hard ceiling: vLLM's context window is 262,144
tokens. At ~5,000 symbols the vocabulary alone would consume the
entire context. Without selective inclusion, KG simply stops
functioning at scale.

Stage 1's tactical trim addressed the dynamic suffix (state history,
signal drivers, prior interpretations) but left the symbol vocabulary
unscoped. That section is now the dominant cost and the dominant
ceiling.

The user has approved combined options A (cluster-relevance closure)
and B (recency window) from the Stage 3 design discussion.

---

## Decision

### Active vocabulary = closure(current cluster) ∪ recency window

For each interpretation, KG includes a *subset* of symbols in the
prompt's `## Symbol Vocabulary` section:

- **Closure of the current cluster.** Start from the symbols in
  `current.cluster`. For each symbol, examine its definition value.
  If it contains entries that are themselves Φ tokens (composites
  referencing primitives, or composites of composites), include
  those. Recurse until no new symbols are added. This guarantees
  every composite the model sees has every primitive it references
  in scope.
- **Recency window.** Maintain a `last_seen_tick: HashMap<String, u64>`
  in `ContextBuilder`. On each `push_state`, mark every cluster
  member as seen at that tick. At render time, include any symbol
  whose last-seen tick is within `recency_window_ticks` of the
  current tick.

Union of the two sets is the active vocabulary. Render alphabetically
sorted for stable ordering across calls.

### Recency window default: 1.7M ticks (≈24 hours)

At observer-gene's ~20 Hz tick rate, 1.7M ticks is one day of
recency. This matches the hourly interpretation cadence (24 cycles
of memory) and is operator-tunable via a new CLI flag
`--vocab-recency-ticks`.

A symbol that hasn't appeared in any cluster within the window drops
out of the vocabulary section automatically. If it returns to a
cluster later, it re-enters via the closure path.

### System-prompt clarification

A single line is added to `SYSTEM_PROMPT` so the model understands
the vocabulary is now a curated subset. Wording (open to refinement
by the impl session):

> The Symbol Vocabulary section below is the active and recently-active
> subset of observer-gene's symbol vocabulary, not the complete set.
> If a symbol referenced elsewhere is not present here, treat it as a
> known but currently-dormant pattern, not an unknown one.

This avoids the model concluding "I've never heard of this" when KG
simply elected not to include a dormant symbol.

### Observability

`build_user_message` (or its caller) logs the size of the active
vocabulary at info level on each interpretation, so growth and
selection effectiveness are visible in the journal:

```
vocab: closure=NNN recency=NNN active=NNN total=NNN
```

`closure` and `recency` may overlap; `active` is the union;
`total` is the size of the symbols entity (the cap that selection
is reducing).

### What this changes vs Stage 1

Stage 1 trimmed the dynamic suffix. Stage 3 (this ADR) trims the
static prefix. Together they push KG's per-call prompt budget toward
the closure-plus-recency size, regardless of total vocabulary
growth.

---

## Tradeoffs This Accepts

### Prefix cache hit rate falls

Today the vocabulary section is constant across calls and almost
all of it caches. With selective vocabulary, the section changes
whenever the cluster shifts (which is most calls). Cache misses
extend from the first divergence point to the end of the section.

Expected post-deploy hit rate: 30–60% on the static prefix, down
from ~91%. This is the cost of bounded prompts at scale. Total
wall-clock for the current 313-symbol vocabulary is roughly
unchanged (smaller prompt, more recompute). The win is at scale —
without this change, KG breaks at ~1500 symbols. With it, KG remains
within budget at 5,000+ symbols because active-vocabulary size grows
sub-linearly with total vocabulary.

### Vocabulary visible to the model is incomplete by design

If observer-gene has a symbol that was relevant N+1 days ago but
hasn't appeared in any cluster since, the model can't read its
definition. The system-prompt clarification softens this for symbols
referenced in other contexts (state history, prior interpretations),
but the model genuinely loses introspection into those definitions.

This is an explicit design choice, not a bug. Recovery: tune
`--vocab-recency-ticks` wider if the regime needs longer memory; or
introduce a long-term compressed memory of past patterns (a separate
Stage 3 concern).

---

## Scope of Changes

### knowledge-gene
- `src/context.rs` —
  - `ContextBuilder` gains `last_seen_tick: HashMap<String, u64>`
    and `recency_window_ticks: u64`.
  - `ContextBuilder::new` takes one more argument
    (`recency_window_ticks`).
  - `push_state` updates `last_seen_tick` for each cluster member.
  - New helper: `compute_active_vocabulary(&self,
    current: &ObserverSnapshot, symbols: &HashMap<String, Value>)
    -> BTreeSet<String>` returning the closure ∪ recency set
    sorted lexicographically.
  - `build_user_message` Symbol Vocabulary section uses the active
    set instead of all symbols.
  - `SYSTEM_PROMPT` gains the subset-clarification line.
- `src/main.rs` —
  - New CLI flag `--vocab-recency-ticks` (u64, default 1700000).
  - Pass into `ContextBuilder::new`.
  - Startup info log includes the new value.
- `src/llm.rs` —
  - In the success-path observability log, also include the active
    vocabulary count. Either passed through from
    `build_user_message` (preferred) or read from a new helper.

---

## What This Does Not Cover

- Long-term compressed memory of past patterns (separate Stage 3
  ADR if needed).
- Two-tier observer cadence — still pulling state at observer-gene's
  native 20 Hz.
- Differential interpretation (skipping LLM calls when state is
  stable) — separate Stage 3 ADR.
- Pre-categorized vocabulary tiers (Option C from the design
  discussion). Could layer on top later if recency proves
  insufficient.
- Any change to the steering protocol, the schema, the LLM request
  body shape, or the publisher.
