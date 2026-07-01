# ADR 006 — Long-Horizon State Samples (Two-Tier State Buffer)

**Status:** Proposed
**Date:** 2026-05-09

---

## Context

Today knowledge-gene's state lookback is ~3.3 minutes — `state_history`
holds 40 snapshots sampled at observer-gene's tick rate, and the
main loop polls every 5 seconds. That's adequate for moment-to-moment
context but useless for the prediction goal: detecting precursor
patterns over hours, recognizing regime returns, and tracking trends
across the full day.

Stage 1's tactical trim and Stage 3 #1's selective vocabulary
addressed prompt-budget problems. Neither extended the model's
temporal horizon. Without longer lookback, KG's "track continuity"
task can only reach the last few minutes.

Three places aggregation could live (analyzed during Stage 3 design):

- A. Inside KG, derived from state_history. Doesn't work — KG
  doesn't have anything older than 3.3 min to derive from.
- B. In observer-gene — publishes its own aggregated state entity.
  Cleanest semantically; requires upstream work and another deploy
  cycle.
- C. Inside KG, sampling at a coarser cadence into a separate
  buffer. Same WS subscription, all KG-internal.

This ADR adopts C as the first move. B is a future option if KG
eventually needs richer aggregation (min/max/transitions/variance
over windows) than point samples can provide.

---

## Decision

### Two-tier state buffer

`ContextBuilder` keeps two parallel state buffers:

- **`state_history` (existing)**: dense recent buffer.
  Cap 40, tick-rate sampling. Covers ~3.3 min.
- **`state_long_history` (new)**: sparse long-horizon buffer.
  Cap 250, sampled every `state_long_interval_ticks` observer ticks.
  At default `6000` ticks (~5 min at observer-gene's ~20 Hz),
  covers ~20.8 hours.

Both are populated from the same `push_state` calls — the long
buffer simply gates writes by tick interval.

### Sampling rule

When `push_state(snap)` is called:

- Always push to `state_history` (unchanged).
- Push to `state_long_history` if either:
  - The long buffer is empty (first call), or
  - `snap.tick >= last_long_push_tick + state_long_interval_ticks`.
- On tick regression (observer-gene restart) detected as
  `snap.tick < last_long_push_tick`, reset state — drop
  `state_long_history` to empty and treat `snap` as the new first
  sample.

`last_long_push_tick` is bookkeeping state on `ContextBuilder`.

### Rendering position in the prompt

Build order (oldest → newest, chronological):

1. Symbol Vocabulary (Stage 3 #1: active subset)
2. Available Actions
3. Current State + signal drivers
4. **Long-Horizon State Samples (new)** — oldest snapshots, ~5 min apart
5. State Trajectory (older state_history without cluster)
6. Recent State (newer state_history with cluster detail)
7. Prior Interpretations

Section heading: `## Long-Horizon State Samples (last <count>
samples, ~<interval-min> min apart, oldest → newest)`. Uses the
compact summary format identical to State Trajectory:

```
tick=N dom=Φ_X cluster_size=N imb=N.N trend=Y align=Z.ZZ
```

No cluster lists in this section — point samples are
trend-focused, not overlap-focused. The Recent State block is
where overlap analysis happens.

### Defaults

- `--state-long-cap`: 250 (≈20.8 h coverage)
- `--state-long-interval-ticks`: 6000 (≈5 min at 20 Hz)

Both CLI-tunable. Operators can widen to ~42 h with `--state-long-cap
500` or shorten the interval for finer-grained mid-term tracking.

### Token cost

At default settings, the long block is ~250 entries × ~80 bytes
≈ 20 KB ≈ 7 K tokens added to the prompt steady-state.

### Cache impact

The long buffer is a sliding window — every ~5 min a new entry
enters and the oldest rotates out. That position shift means the
prompt's long-block bytes change between most calls, so vLLM
prefix cache will essentially miss this section every time.

That's ~7 K tokens of fresh prefill per call ≈ ~30 s additional
latency at observed prefill rates. Acceptable cost for ~21 h of
lookback.

The static prefix (vocabulary subset, system prompt, actions) and
the dynamic-but-stable Recent State / State Trajectory blocks are
unaffected by this change.

### Observability

The success-path info log is extended to include the long-buffer
size:

```
hist: detail=N trajectory=N long=N
```

Either appended to the existing `vocab:` line or as its own info
line — implementation choice.

---

## What This Does Not Cover

- **Compressed long-term memory across days/weeks (Stage 3 #3).**
  Point samples for 21 h are sufficient for same-day pattern
  recognition. Cross-day or cross-week pattern recognition needs
  semantic compression of past states (LLM-distilled "lessons"),
  which is a separate ADR.
- **Aggregation primitives in observer-gene (option B above).**
  Could layer on later if point samples prove insufficient.
- **Adaptive sampling.** This ADR uses a fixed tick interval. A
  future refinement could vary sampling density by regime stability
  (denser samples during transitions, sparser during stable
  periods).
- **Any change to the steering protocol, the LLM request body
  shape, the publisher, the schema, or the steering receiver.**
  Steering is unchanged.
- **Any change to selective-vocabulary behavior.** Stage 3 #1's
  closure ∪ recency logic is independent of and unaffected by this
  buffer.
