# Task 05 — vLLM Migration & Reasoning-Mode Adoption

**Date:** 2026-05-07
**Status:** Complete — `cargo check` passed cleanly

---

## What Was Built

### `src/llm.rs`
- Added `enable_thinking: bool` and `max_tokens: u32` fields to `LlmClient`
- `LlmClient::new` now accepts both (5-param signature)
- Request body now includes:
  - `chat_template_kwargs.enable_thinking` (bool from field)
  - `max_tokens` (u32 from field)
  - `response_format` changed from `json_object` to `json_schema` (strict) with
    four-field schema: `interpretation` (string), `suggested_actions` (integer array),
    `confidence` (number 0–1), `themes` (string array); `additionalProperties: false`
- Timing: `Instant::now()` captured before `req.send()`, elapsed computed after body read
- Success-path `tracing::info!` logs elapsed (secs), prompt_tokens, completion_tokens,
  reasoning_tokens (from `usage.completion_tokens_details.reasoning_tokens`); missing
  values render as `?`

### `src/context.rs` — `build_user_message`
- Reordered sections for vLLM prefix-cache stability:
  1. **Symbol Vocabulary** — all known symbols (entire `sym_props` map), sorted by key ascending
  2. **Available Actions** — sorted (unchanged)
  3. **Current Observer State** — unchanged content (dominant, cluster, imbalance, etc.)
  4. **State History** — unchanged
  5. **Prior Interpretations** — unchanged
- Removed "Active Symbol Definitions" section (was cluster-only subset; replaced by full vocabulary above)

### `src/main.rs`
- Added two CLI flags to `Config`:
  - `--disable-thinking` (bool, default false) — polarity: thinking on by default
  - `--max-tokens` (u32, default 12000)
- Both logged in startup info block alongside endpoint/model
- `LlmClient::new` call updated: passes `!cfg.disable_thinking` and `cfg.max_tokens`

---

## Signature Change

`LlmClient::new` signature changed from:
```
new(endpoint, model, api_key)
```
to:
```
new(endpoint, model, api_key, enable_thinking, max_tokens)
```

Only one call site (`main.rs` L179) — updated in this session.

---

## Issues Encountered

None. Compilation was clean on first attempt.

---

## Next Steps

- Task 06: Deploy updated binary to .107
