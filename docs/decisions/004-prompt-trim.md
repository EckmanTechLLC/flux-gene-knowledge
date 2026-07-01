# ADR 004 — Tactical Prompt Trim (Stage 1)

**Status:** Proposed
**Date:** 2026-05-09

---

## Context

Production deploy on 2026-05-09 confirmed that knowledge-gene's prompt
behaves predictably (vLLM prefix cache hit ~91%, schema enforcement
holds, decode at ~2 tok/s on ROCm), but the prompt is structurally
heavier than necessary.

At steady state — once the rolling buffers fill — the prompt
distributes roughly as:

| Section | tokens (est.) | dynamic? |
|---|---|---|
| SYSTEM_PROMPT | ~700 | no (cached) |
| Symbol Vocabulary (313 symbols) | ~36,000 | no (cached) |
| Available Actions | ~500 | no (cached) |
| Current state + signal drivers | ~1,000–2,000 | yes |
| State History (75 snapshots) | ~50,000 | yes (grows) |
| Prior Interpretations (20) | ~3,000 | yes (grows) |

Two factors dominate the dynamic portion:

1. Each historical snapshot dumps the full cluster list in
   `cluster=[Φ_xxxx,Φ_yyyy,...]` form. With clusters of 200+ symbols
   that's ~2.2KB per snapshot. 75 entries shifts ~165KB / ~50K tokens
   per call that the prefix cache cannot retain.
2. When the +0.500 phantom-signal regime is active, the signal-drivers
   section enumerates 200+ signals at the same deviation, line by
   line. Hundreds of identical-value rows with no aggregation.

The model verifiably uses recent history for continuity tracking
("last 15+ ticks", "structurally identical macro-pattern"). Trimming
must preserve that recent-detail view while compressing the older tail
and the redundant driver rows.

This is bridge work, not a redesign. Stage 3 will eventually replace
the "send everything every call" architecture with selective
vocabulary and a coarser long-horizon state stream. The trim here
keeps production efficient until that lands.

---

## Decision

### Tiered state history

`build_user_message` renders state history in two parts:

- **Recent State (default last 10 entries):** rendered as today —
  `tick=N dom=Φ_xxxx cluster=[Φ_xxxx,Φ_yyyy,...] imb=N.N trend=Y align=Z.ZZ`.
  Preserves cluster-overlap detail for the recent window where the
  model actually uses it.
- **State Trajectory (older entries):** rendered without the cluster
  list — `tick=N dom=Φ_xxxx cluster_size=N imb=N.N trend=Y align=Z.ZZ`.
  Keeps the dominant symbol per tick (continuity anchor) and the
  imbalance/trend/alignment trajectory.

Storage stays a single VecDeque; the split happens at render time.
Both tiers are CLI-tunable.

### Run-length compression for signal drivers

When five or more signal drivers share an identical deviation value,
they collapse into one summary line:

`  N signals all at <dev>   (range: s_AAAA..s_BBBB)`

The range hint is the lexicographic min/max of the affected sig_ids,
purely advisory — we do not require contiguity. Drivers not part of
a large group render individually as today. Five is the threshold;
hard-coded, not CLI-tunable (operational detail).

This addresses the +0.500 phantom regime specifically but applies to
any future regime where many signals deviate uniformly.

### Reduced default history depths

CLI defaults change to align with the trimmed structure:

- `--state-history` default: **75 → 40** (was 6 min lookback,
  becomes ~3.3 min — sufficient given the detailed window covers
  the recent ~50 seconds where overlap analysis matters)
- `--interp-history` default: **20 → 10** (still ~10 hours of
  narrative continuity at hourly cadence)
- new `--state-detail-cap`: default **10** (recent entries rendered
  in detailed form)

All three remain operator-tunable on .107 if needed.

### Expected impact at steady state

- State history section: ~50K tokens → ~7.5K tokens
- Signal drivers section (under +0.500 regime): ~700 tokens → ~50
  tokens
- Prior interpretations: ~3K tokens → ~1.5K tokens
- Total prompt: ~91K → ~46K tokens (≈50% reduction)

Cold-call latency should fall roughly in proportion. Warm-call
benefit smaller in absolute terms (cache already covered most of the
unchanged sections) but still meaningful because the dynamic suffix
shrinks.

---

## Scope of Changes

### knowledge-gene
- `src/context.rs` — `build_user_message`: split state-history
  rendering into detailed vs trajectory tiers; add RLE pass for
  identical-deviation signal drivers; rename the section heading
  appropriately.
- `src/main.rs` — change defaults for `--state-history` (75→40) and
  `--interp-history` (20→10); add new CLI flag `--state-detail-cap`
  (default 10); plumb the new value into `ContextBuilder` (probably
  via a new field) so `build_user_message` knows the split point.
- No struct schema changes for `Interpretation`. No LLM request body
  changes.

---

## What This Does Not Cover

- Symbol vocabulary scope (still all 313 symbols every call). This is
  the largest remaining cacheable section; selective vocabulary is a
  Stage 3 concern.
- The 5-second tick rate of state-history accumulation — entries
  still arrive every observer-gene tick. Coarser aggregation is a
  Stage 3 concern (two-tier observer cadence).
- Prior interpretation excerpt length (still 400 chars per entry).
  Could be trimmed in a future ADR if the cap reduction here proves
  insufficient.
- Steering loop or any observer-gene-side change. Out of scope —
  Stage 2.
