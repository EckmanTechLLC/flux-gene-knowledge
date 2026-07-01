# Task 11 — Long-Term Pattern Lessons: Session Summary

**Date:** 2026-05-13
**Status:** Complete — `cargo check` passes cleanly

---

## What Was Built

- `src/lessons.rs` — new module with:
  - `Lesson` struct (tick_range_start, tick_range_end, themes, narrative, created_at_ms) — serde Serialize+Deserialize
  - `DISTILL_SYSTEM_PROMPT` constant
  - `distill(llm, state_long_history, interp_history, distill_max_tokens)` async fn — builds user message from long buffer + interp history, calls LLM via `complete_with_schema`, parses response, sets `created_at_ms` at call time
  - `render_lessons_section(lessons, budget)` — greedy newest-to-oldest inclusion within char/3 token budget, reversed to oldest-newest for render, returns (String, count)
  - `parse_lessons_from_properties(props)` — iterates "lesson_" keys, skip-on-parse-failure with warn!, returns Vec sorted by tick_range_end ascending

- `src/llm.rs` — added `complete_with_schema` generic JSON-schema helper (same request/parse pattern as `interpret`); updated `hist:` log line to include `lessons=N`

- `src/context.rs` — `ContextBuilder` gains `lessons: Vec<Lesson>` and `lesson_token_budget: usize`; `new` now takes 7 args (lesson_token_budget added); `set_lessons` and `push_lesson` methods added; `build_user_message` inserts Past Pattern Lessons section between Available Actions and Current State; `BufferSizes` gains `lessons: usize`; `SYSTEM_PROMPT` gains the pattern lessons clarification line

- `src/publisher.rs` — `publish_lesson(&Lesson)` using padded `lesson_{:013}` key on `knowledge-gene/lessons` entity; `delete_lesson(u64)` writes null to property key

- `src/main.rs` — `mod lessons;` added; four new CLI flags (distill_interval_secs=86400, lesson_token_budget=8000, max_lessons=500, distill_max_tokens=2000); `bootstrap_lessons()` helper that fetches `knowledge-gene/lessons` (404 = normal, logged as info); separate `distill_llm` client constructed with `distill_max_tokens`; `ctx.set_lessons(bootstrap_lesson_list)` after bootstrap; `last_distillation = Instant::now()` initialized so first distill fires after interval; distillation block at bottom of main loop after sleep — logs, distills, publishes, pushes to ctx, prunes with delete_lesson when over max_lessons; new flags logged at startup

---

## Key Decisions

- **Second LlmClient approach**: constructed a separate `distill_llm` with `distill_max_tokens`. Cleaner than per-call max_tokens since `LlmClient::interpret` uses `self.max_tokens`. Added `complete_with_schema` as a generic helper on `LlmClient` that takes explicit `max_tokens`.
- **delete_lesson**: implemented using null property write (not a real Flux delete, but consistent with how the task described Flux's constraints).
- **Circular module imports**: `context.rs` imports from `lessons.rs`; `lessons.rs` imports from `context.rs` and `llm.rs`. Rust crate modules allow this without issue.
- **publisher.rs key construction**: used `serde_json::Map::new()` + `insert` instead of `serde_json::json!({key: value})` to avoid move-borrow compile error on variable-key JSON macros.

---

## Issues Encountered

- `serde_json::json!({key: value})` moves the `key` String, causing borrow error when `key` is later used in the tracing log. Fixed by using explicit `Map::new()` + insert + `serde_json::Value::Object(...)`.

---

## Next Steps

- Deploy and validate against live observer-gene
- Monitor first distillation run log (`distill: elapsed=...`)
- Verify lesson persistence across KG restart
