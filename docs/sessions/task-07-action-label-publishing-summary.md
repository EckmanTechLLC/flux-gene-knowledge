# Task 07 — Publish suggested_action_labels + Prose Reference

**Date:** 2026-05-09
**Status:** Complete
**cargo check:** Passed cleanly

## Changes Made

### src/publisher.rs
- Added `use std::collections::HashMap;`
- Changed `publish` signature to accept `key: Option<&HashMap<String, serde_json::Value>>`
- Derives `suggested_action_labels: Vec<String>` by mapping each integer N in
  `interp.suggested_actions` through `key["actions"]["action_N"]`
- Falls back to `"action_N"` string if key is None or entry missing
- Added `"suggested_action_labels"` to published properties JSON, immediately after `"suggested_actions"`

### src/main.rs
- Updated call site: `publisher.publish(&interp, key_opt.as_ref()).await`

### src/context.rs
- Added task 6 to SYSTEM_PROMPT numbered list (before CRITICAL CONSTRAINTS):

> 6. NAME ACTIONS — When you recommend actions in `suggested_actions`, also reference each one by name (taken from the action key) in your interpretation prose so the narrative is readable on its own.

## Verification
- `suggested_actions` stays `Vec<u32>` integers — unchanged
- Empty `suggested_actions` → empty `suggested_action_labels`
- None key → all-fallback labels (`"action_N"`)
- `cargo check` passed, no warnings
