# Task 06 — Configurable HTTP Timeout for LLM Calls

**Date:** 2026-05-07
**Status:** Complete — `cargo check` passed cleanly

## What Was Built

- `LlmClient::new` in `src/llm.rs` now accepts `http_timeout_secs: u64` as its 6th argument; replaces hardcoded 300s timeout
- `--http-timeout-secs` flag added to `Config` in `src/main.rs`, default **2700s** (45 min)
- Startup log line added: `llm http_to: <N>s`
- `LlmClient::new` call site in `main.rs` passes `cfg.http_timeout_secs` as arg 6

## Key Decisions

- Default set to 2700s (3× the originally proposed 900s) at user request, to give ample headroom for cold thinking-on calls with full 313-symbol vocabulary in prefix
- Single overall timeout (no separate connect vs. read split) — sufficient per task scope

## New Arg Position in `LlmClient::new`

1. `endpoint: String`
2. `model: String`
3. `api_key: Option<String>`
4. `enable_thinking: bool`
5. `max_tokens: u32`
6. `http_timeout_secs: u64` ← new

## Issues

None.

## Next Steps

- Foundation to rebuild and integration-test against vLLM with cold prefix-cache call
