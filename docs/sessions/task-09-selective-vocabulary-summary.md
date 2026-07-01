# Task 09 — Selective Symbol Vocabulary — Session Summary

**Date:** 2026-05-09
**cargo check:** PASSED (clean, 1.58s)

---

## What Was Built

- `ContextBuilder` extended with two new fields: `last_seen_tick: HashMap<String, u64>` and `recency_window_ticks: u64`.
- `ContextBuilder::new` takes a fourth argument `recency_window_ticks: u64`.
- `push_state` now records last-seen tick for every cluster member after pushing the snapshot (borrows from `state_history.back()`).
- New `active_vocabulary(&self, current, symbols) -> (Vec<String>, usize, usize)` method:
  - Closure via worklist BFS from `current.cluster`, following Φ_ entries in JSON array definitions recursively.
  - Recency via `last_seen_tick` scan — includes symbols with `age < recency_window_ticks` that exist in `symbols`.
  - Returns (sorted active union, closure_count, recency_count); early-returns empty on `tick == 0`.
- `build_user_message` return type changed from `String` to `(String, usize, usize, usize)` — tuple is `(msg, closure_count, recency_count, active_count)`.
- Symbol Vocabulary section now emits only the active subset (sorted); no section header emitted if active set is empty.
- `SYSTEM_PROMPT` now includes the subset-clarification sentence (see below).
- CLI flag `--vocab-recency-ticks` (u64, default 1_700_000) added to `Config`.
- Startup log line added: `vocab recency: N ticks`.
- `ContextBuilder::new` call in `main.rs` updated to pass `cfg.vocab_recency_ticks`.
- `llm.rs`: `build_user_message` call now destructures the tuple; success-path log adds a second info line with vocab counts.

---

## Exact SYSTEM_PROMPT Wording Added

> The Symbol Vocabulary section below is the active and recently-active subset of observer-gene's symbol vocabulary, not the complete set. If a symbol referenced elsewhere is not present here, treat it as a known but currently-dormant pattern, not an unknown one.

Inserted between the "Your role is to reason..." paragraph and "Your tasks for each interpretation:".

---

## Plumbing Approach

Return tuple from `build_user_message` — `(String, usize, usize, usize)`. The caller in `llm.rs` destructures it and uses the counts in a second `tracing::info!` line immediately after the existing timing/token log. No intermediate struct needed.

---

## Log Format (success path)

```
llm: elapsed=X.Xs prompt_tok=N completion_tok=N reasoning_tok=N
     vocab: closure=N recency=N active=N total=N
```

---

## Files Changed

- `src/context.rs` — struct, new(), push_state, active_vocabulary helper, build_user_message signature+return+vocab block, SYSTEM_PROMPT
- `src/main.rs` — Config field, startup log, ContextBuilder::new call
- `src/llm.rs` — destructure build_user_message return, add vocab log line
