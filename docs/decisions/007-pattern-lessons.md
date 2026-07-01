# ADR 007 — Long-Term Pattern Lessons (Compressed Memory)

**Status:** Proposed
**Date:** 2026-05-09

---

## Context

Stage 3 #1 selectively scopes the symbol vocabulary so KG fits at
scale. Stage 3 #2 extends recent lookback to ~21 hours via point
samples. Neither lets the model recognize patterns across days or
weeks — the temporal horizon for which "this looks like the
precursor to last quarter's event" inferences live.

The user's prediction goal genuinely requires that horizon. State
samples can't carry it; a 30-day rolling window at 5-minute
sampling is ~17 K tokens of mostly-flat signal data the model has
to mentally compress on every call. That's the wrong shape — the
compression should already be done.

Stage 3 #3 introduces a compressed memory: model-distilled
narrative summaries of past periods, persisted across KG restarts,
read back into every prompt. Each is a "lesson" — a few hundred
characters of prose that captures a regime's shape, dominant
themes, and notable transitions.

This ADR adopts the simplest viable shape (narrative ledger, daily
distillation, all-lessons-within-budget rendering, Flux storage) so
v1 ships without RAG infrastructure or multi-resolution rollup.

---

## Decision

### Lesson shape: narrative ledger

A lesson is a JSON object with these fields:

- `tick_range_start` — integer; observer-gene tick at the start of
  the period being summarized.
- `tick_range_end` — integer; observer-gene tick at the end.
- `themes` — array of strings; high-level theme labels, like the
  themes already present on `knowledge-gene/state`.
- `narrative` — string; 200–500 characters of prose describing
  what the period looked like, what was dominant, and any
  transitions worth remembering.
- `created_at_ms` — integer; unix milliseconds at which this
  lesson was distilled. Used for chronological sort and for
  pruning.

Each lesson is treated as an immutable snapshot. No editing, no
merging in v1. Future ADRs can add weekly/monthly rollup if the
ledger outgrows its budget.

### Storage: Flux entity `knowledge-gene/lessons`

KG publishes lessons to a new Flux entity. Each lesson is a
*single property* on the entity, keyed by its end-tick:

```
entity_id: knowledge-gene/lessons
properties:
  lesson_173600000:    { tick_range_start, tick_range_end, themes, narrative, created_at_ms }
  lesson_173800000:    { ... }
  ...
```

Why one-property-per-lesson rather than a single array property:
appending to a Flux entity is a single property write; replacing
an array property would require a read-modify-write with race
risk. Per-lesson keys avoid the round-trip.

The end-tick prefix gives lexicographic-sortable keys when zero-
padded. Implementation should pad to a width that exceeds plausible
tick values (the binary already uses `u64`, so a 13-digit pad like
`lesson_0000173600000` is sufficient).

### Persistence across KG restarts

At bootstrap, KG fetches `knowledge-gene/lessons` from Flux's REST
API alongside the existing observer-gene entities. Properties are
parsed into an in-memory `Vec<Lesson>`. If the entity does not yet
exist (first deploy), bootstrap handles the 404 gracefully and
starts with an empty list.

### Distillation cadence: daily, configurable

KG runs a separate LLM call once per `--distill-interval-secs`
(default 86400 = 24 h). The distillation prompt is distinct from
the interpretation prompt — different system prompt, different
schema, smaller `max_tokens` cap (the output is bounded prose, not
extended reasoning).

The first distillation runs `--distill-interval-secs` seconds after
KG starts; if KG was just deployed and lessons exist already in
Flux from a previous run, the timer still counts from the new
process's start. Simple, predictable.

If the distillation LLM call fails, KG logs a warning and resets
the timer. The next attempt runs at the next interval. No
exponential backoff or retry storms.

### Distillation prompt and schema

System prompt (separate constant, e.g. `DISTILL_SYSTEM_PROMPT`):

> You are distilling observer-gene's recent state activity into a
> compact narrative lesson, suitable for future pattern recognition
> by the same agent. Capture the dominant regime, key transitions,
> notable cross-domain correlations, and anything worth remembering
> for recognizing returning patterns. Be specific about which
> domains, signals, or symbols characterized the period. Avoid
> hedging language and bullet lists; aim for 200–500 characters of
> prose.

User message: a compact dump of the last `--distill-interval-secs`
of activity drawn from `state_long_history` (the Stage 3 #2 long
buffer) plus the recent `interp_history`. No full vocabulary, no
state trajectory — distillation works on already-summarized data,
not raw signals.

Response schema (strict json_schema):

```
type: object
properties:
  tick_range_start: integer
  tick_range_end:   integer
  themes:           array of string
  narrative:        string
required: [tick_range_start, tick_range_end, themes, narrative]
additionalProperties: false
```

`created_at_ms` is set by KG at publish time, not asked from the
model.

### Inclusion in prompts: budget-bounded, all included

Lessons are rendered in `build_user_message` as a new section
positioned between Available Actions and Current State (so they
frame the model's reading of current state):

```
1. Symbol Vocabulary
2. Available Actions
3. Past Pattern Lessons          ← NEW: oldest → newest
4. Current State + drivers
5. Long-Horizon State Samples
6. State Trajectory
7. Recent State
8. Prior Interpretations
```

All lessons within `--lesson-token-budget` (default 8000 tokens,
roughly 8000×~3.5 = ~28 KB of prose) are included. Token counting
uses a simple char/3 heuristic; accuracy isn't critical because the
budget exists to prevent unbounded growth, not to shave precise
tokens.

When budget is exceeded, oldest lessons are dropped first.

### Pruning

KG enforces a hard cap on lesson count (`--max-lessons`, default
500). When exceeded, the oldest entry is removed from local state
*and* deleted from Flux via property delete (or via writing a
`null` value — match whatever Flux supports for property removal;
implementation can decide).

This is a simple cap, not a multi-resolution rollup. The
budget-bounded inclusion above means the *displayed* lessons are
already capped per call; this `--max-lessons` cap is for total
storage growth.

### System-prompt clarification

`SYSTEM_PROMPT` gains a single line so the model treats the new
section correctly:

> The Past Pattern Lessons section is a journal of compressed
> memories from previous periods. Use them to recognize returning
> patterns and to contextualize current activity. Older lessons
> appear first.

### Observability

The success-path info log on each interpretation is extended to
include lessons-included count:

```
hist: detail=N trajectory=N long=N interps=N lessons=N
```

A new info log fires when a distillation runs:

```
distill: elapsed=X.Xs tick_range=N..N themes=[...] narrative_chars=N
```

---

## Tradeoffs This Accepts

- **No RAG / semantic retrieval.** All lessons within budget go in,
  selected by recency only. The model itself does the relevance
  matching internally. Acceptable for now; ledger budgets sustain
  ~50 lessons (50+ days at default), well past where this becomes
  uncomfortable.
- **No multi-resolution rollup.** When `--max-lessons` is hit, we
  just delete the oldest. A future ADR can add daily→weekly→monthly
  summarization if pruning by recency proves too lossy.
- **First distillation 24h after bootup.** A KG restart resets the
  cadence. In practice this matters only if KG restarts very
  frequently; not worth complexity.
- **Distillation runs inline in the main loop.** No separate
  scheduler thread. The distillation LLM call adds latency to one
  poll iteration per day. Acceptable.

---

## Scope of Changes

### knowledge-gene
- `src/lessons.rs` — **new module**:
  - `Lesson` struct (the five fields above)
  - `DISTILL_SYSTEM_PROMPT` constant
  - `distill(...) -> Result<Lesson>` function — builds the
    distillation user message from buffers, calls vLLM, parses
    response, sets `created_at_ms`.
  - `render_lessons_section(lessons: &[Lesson], budget: usize)
    -> (String, usize)` — produces the prompt section + count of
    lessons actually included.
  - Parsing helper for the Flux entity properties → `Vec<Lesson>`
    on bootstrap.
- `src/context.rs` — `ContextBuilder` gains a `lessons:
  Vec<Lesson>` field; `build_user_message` calls
  `render_lessons_section` and emits the new section. The returned
  `BufferSizes` struct gains a `lessons` field.
- `src/main.rs` — bootstrap fourth entity
  `knowledge-gene/lessons`; new CLI flags
  (`--distill-interval-secs`, `--lesson-token-budget`,
  `--max-lessons`, `--distill-max-tokens`); inline distillation
  scheduling at the bottom of the main loop iteration; pass
  lessons into `ContextBuilder`.
- `src/publisher.rs` — new method `publish_lesson(lesson:
  &Lesson)` that posts a single property update to
  `knowledge-gene/lessons` keyed by the padded end-tick. Use the
  existing `post_raw` helper.
- `src/llm.rs` — either expose a generic
  `complete_with_schema` helper for distillation reuse, or accept
  that lessons.rs has its own minimal HTTP path. Implementation
  can pick.
- `src/context.rs` — extend `SYSTEM_PROMPT` with the lessons
  clarification line.

---

## What This Does Not Cover

- Per-regime-end triggered distillation. v1 is daily.
- Editing or merging lessons. v1 immutable.
- Lesson search / RAG / embeddings.
- Cross-instance lesson sharing.
- Schema evolution. If the lesson shape changes later, old lessons
  are still parseable as long as required fields are stable.
- Manual distillation triggers (e.g. a CLI command). All
  distillation is timer-driven in v1.
