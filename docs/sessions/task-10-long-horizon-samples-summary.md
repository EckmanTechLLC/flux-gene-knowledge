# Task 10 — Long-Horizon State Samples (Two-Tier State Buffer)

**Date:** 2026-05-09
**Status:** Complete
**cargo check:** Passed cleanly

---

## What Was Built

- `BufferSizes` struct added to `context.rs` with fields: `detail`, `trajectory`, `long`, `interps`
- `ContextBuilder` extended with 4 new fields: `state_long_history`, `state_long_cap`, `state_long_interval_ticks`, `last_long_push_tick`
- `ContextBuilder::new` updated to 6 arguments (added `state_long_cap`, `state_long_interval_ticks`)
- `push_state` updated to drive the long buffer: tick regression clears the buffer; push gate fires on empty-or-interval; cap enforced via `pop_front`
- `build_user_message` inserts `## Long-Horizon State Samples` section between Current State + drivers (§3) and State Trajectory (§5); section skipped cleanly when buffer is empty
- `build_user_message` return type extended to `(String, usize, usize, usize, BufferSizes)`; `BufferSizes` computed from live buffer sizes
- `Config` in `main.rs` gains `--state-long-cap` (default 250) and `--state-long-interval-ticks` (default 6000)
- `ContextBuilder::new` call site in `main.rs` updated to 6 args
- Startup log line added: `state long: 250 (sample interval: 6000 ticks)`
- `llm.rs` destructure updated to capture `buf_sizes`; success-path log extended with `hist: detail=N trajectory=N long=N interps=N`

## Key Decisions

- Used a named `BufferSizes` struct (not a bare tuple) — reads clearly at the call site in llm.rs
- `BufferSizes` not imported in llm.rs (type inferred from destructure); avoids unused-import warning
- `interval_min` computed as `(state_long_interval_ticks * 5) / 6000` per spec (integer arithmetic, rounding implicit)
- `push_state` clones `snap` before moving it into `state_history` so the long-buffer path can also use it

## Tick Regression Handling

Confirmed in place: if `snap.tick < self.last_long_push_tick`, `state_long_history` is cleared and `last_long_push_tick` reset to 0 before the push gate is evaluated.

## Issues

None. Straightforward implementation.

## Next Steps

Foundation to rebuild and validate against live observer-gene.
