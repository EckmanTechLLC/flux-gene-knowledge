# ADR 002 — vLLM Migration & Reasoning-Mode Adoption

**Status:** Proposed
**Date:** 2026-05-07

---

## Context

knowledge-gene was originally pointed at Ollama on the internal LLM host. Ollama
has since been replaced on that host by vLLM, serving
`Qwen/Qwen3.6-35B-A3B` (model id `qwen3.6`, 262K context) at
`http://<llm-host>:8000/v1`.

A live end-to-end test from .13 against the existing release binary and
vLLM uncovered three structural problems that make a clean
configuration-only switch insufficient:

1. **Latency.** A cold first call took ~275s with thinking disabled and
   65,005 input tokens. The current message structure puts dynamic
   content before the bulk of the stable content (symbol definitions),
   so vLLM's prefix cache cannot retain the expensive prefix across
   calls. Every interpretation pays full prefill cost.

2. **Schema drift.** `qwen3.6` returned a JSON object whose prose field
   was named `interpret`, with an extra `assess` field — not the
   `interpretation` field the system prompt specifies. The existing
   parser silently drops the prose because it reads `interpretation`.
   `response_format: json_object` enforces JSON-shape only, not field
   names.

3. **Reasoning behaviour.** `qwen3.6` is a reasoning model. Its
   reasoning trace lands in a separate `reasoning` field on the
   response, with `content` populated only after reasoning completes.
   With reasoning enabled, calls run for many minutes; with reasoning
   disabled (`chat_template_kwargs.enable_thinking: false`), the same
   call returned 275s with empty reasoning and clean content.

The role of knowledge-gene is to supply the semantic layer
observer-gene structurally lacks, and to steer observer-gene's
perception via `suggested_actions`. Interpretation quality directly
shapes future perception. Cheap interpretations are not free — they
degrade the loop.

---

## Decision

### Endpoint and model

knowledge-gene targets vLLM at `http://<llm-host>:8000/v1` with
model id `qwen3.6`. No code change for endpoint/model; configured via
existing `--llm-endpoint` and `--llm-model` flags.

### Reasoning enabled by default

Thinking is left **on** in the request body, configurable off via a new
CLI flag. The task class — continuity tracking across 75 history
snapshots, distinguishing genuine cross-domain correlation from naturally
coupled or pipeline-artifact noise, calibrated confidence — benefits
materially from a reasoning trace. The earlier instinct to disable
thinking was driven by latency, which is solved by the prefix-cache
restructure below.

A bounded `max_tokens` cap is added to prevent pathological reasoning
runs from monopolising the GPU and to surface truncation cleanly
(`finish_reason: length`) rather than hanging on the existing 300s
reqwest timeout.

### Strict schema enforcement

The request switches from `response_format: json_object` to
`response_format: json_schema` with the four-field schema below and
`strict: true`. vLLM's guided decoder constrains the output tokens so
the model cannot emit alternate field names regardless of how the
system prompt is read.

Schema fields (matching the existing `Interpretation` struct):
- `interpretation` — string
- `suggested_actions` — array of integers
- `confidence` — number, 0.0–1.0
- `themes` — array of strings

The existing tolerant parsing in `llm.rs` (think-tag stripping,
brace-extraction fallback) is retained as defence in depth across
future model swaps.

### Prefix-cache-friendly message structure

`build_user_message` is reordered so the stable prefix dominates and
the cache can hold it across calls. New order:

1. **Stable prefix** — all 313 symbol definitions, then the action key.
2. **Dynamic suffix** — current state, signal drivers, state history,
   prior interpretations.

Two consequences:

- All known symbols are sent, not only the current cluster's members.
  This is also semantically correct: composite symbols
  (Φ_C_NNNN) reference primitives that may not be in the active
  cluster, and the model cannot decode a composite without its
  primitives' definitions. The cluster is still flagged in the dynamic
  section so the model knows which subset is active.
- The system prompt continues to be sent as the `system` message and
  remains the very first thing in the prompt. It already participates
  in the cache.

Expected effect: cold first call near current latency (~5 minutes with
thinking on); warm subsequent calls drop into the tens of seconds for
prefill, plus reasoning decode time.

### Observability

The `interpret` method logs at info level on success: prompt token
count, completion token count, reasoning token count (where exposed),
and wall-clock elapsed. This makes warm-call latency and cache
behaviour visible in the systemd journal without a code change.

### Cadence (operational, not code)

`--interval-secs` and `--early-cooldown-secs` defaults remain at 300
and 60 in code. The .107 systemd unit will set them to 600 and 300 to
align with thinking-on warm-call latency, leaving CLI defaults
appropriate for development.

---

## Scope of Changes

### knowledge-gene
- `src/llm.rs` — request body: add `chat_template_kwargs.enable_thinking`,
  add `max_tokens`, replace `response_format: json_object` with
  `json_schema` (strict). Log token counts and elapsed at info level.
- `src/context.rs` — `build_user_message`: reorder so symbol
  definitions and action key are emitted before current state and
  history. Dump all entries from the symbols map, not just cluster
  members. Cluster is still indicated in the dynamic section.
- `src/main.rs` — new CLI flags: `--disable-thinking` (bool, default
  off → thinking on), `--max-tokens` (default 12000). Plumbed into
  `LlmClient`.

### Deployment (separate task)
- Rebuild release binary on .13 (cargo confirmed present).
- Deploy to .107 at `/home/etl/knowledge-gene/`, update systemd unit
  with new endpoint, model, and operational cadence.

---

## What This Does Not Cover

- Switching to a different vLLM-served model. `qwen3.6` is what's
  loaded on .17 today and what odin uses, so it's free to share.
- Per-call thinking gating (e.g. thinking-on for significant-change
  triggers, off for routine ticks). Possible future refinement; not
  needed at this stage.
- Caching of LLM responses on the client side. vLLM's prefix cache is
  the only caching layer in scope.
- Changes to the steering protocol (ADR 001) or the
  `Interpretation` struct shape.
