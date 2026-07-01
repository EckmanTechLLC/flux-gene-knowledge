# ADR 003 — Action Label Publishing

**Status:** Proposed
**Date:** 2026-05-09

---

## Context

knowledge-gene publishes `suggested_actions` as a list of integer
action IDs (e.g. `[106, 108]`) per ADR 001's steering protocol. The
steering protocol consumer (observer-gene's receiver) needs integers
to dispatch.

Human-facing consumers — primarily the flux-universe.com website at
`projects/flux-site` — need readable labels (e.g.
`adjust_decay.europe_air_count.faster (+0.005)`) to be useful.
Today they have to do a join: take the integer N, prefix it to
`"action_N"`, look it up in `observer-gene/key.actions`. The website
currently does this lookup correctly.

The friction: `observer-gene/key.actions` keys are `"action_NNN"`
strings while KG publishes raw integers. Any consumer that joins
naively (`actionMap[a]`) misses, and falls back to displaying the bare
integer. Each new consumer reinvents the join.

Earlier, when knowledge-gene was using `Vec<String>` for the field,
suggestions came through as `["action_106"]` and the website's join
worked without conversion. Task 03 (Apr 5) changed the type to
`Vec<u32>` to satisfy the steering protocol's integer contract.
That change was correct for steering but silently broke the
website's display path because both sides were keyed on the same
string and that link is now gone.

---

## Decision

### Publish both forms, in one entity

`knowledge-gene/state` gains a new property `suggested_action_labels`
alongside the existing `suggested_actions`:

- `suggested_actions` — list of integers, unchanged. Steering protocol
  contract preserved.
- `suggested_action_labels` — list of strings, parallel index. Each
  string is the resolved description from `observer-gene/key.actions`
  using the canonical lookup `actions["action_<N>"]`. If a given
  action ID is not present in the key, the label falls back to
  `"action_<N>"` (so consumers always have a non-empty parallel list).

### Resolution at publish time

knowledge-gene already has `observer-gene/key` in its shared state
(loaded at bootstrap and kept current via WS). The publisher does the
lookup at publish time using the most recent `actions` map. Resolution
is local; no extra HTTP call.

If for any reason the `actions` map is missing or empty when an
interpretation publishes, every entry falls back to `"action_<N>"`.
Better than dropping the field.

### LLM also references actions by name in prose

The interpretation narrative is the human-readable artifact most
operators read first. Today the LLM picks action IDs but doesn't
mention them in the prose. SYSTEM_PROMPT is updated with a single
instruction asking the model to reference recommended actions by name
(taken from the action key already in the prompt) in addition to
returning the integer IDs in the schema field. No schema change.

### What this does NOT change

- `suggested_actions` keeps publishing integers. Steering protocol is
  untouched.
- `Interpretation` struct gains no new field — labels are derived at
  publish time, not stored on the struct.
- The website does not need to change to start showing labels — its
  existing path (read `suggested_actions`, prefix `action_`, look up
  in actionMap) keeps working. After this change, the website can
  optionally simplify by reading `suggested_action_labels` directly.

---

## Scope of Changes

### knowledge-gene
- `src/publisher.rs` — `publish` signature gains
  `key: Option<&HashMap<String, Value>>`. Builds
  `suggested_action_labels` parallel to `suggested_actions` at the
  property assembly step.
- `src/main.rs` — pass `&key_opt` into the `publisher.publish(...)`
  call. (Already in scope, just plumb it.)
- `src/context.rs` — append one instruction line to `SYSTEM_PROMPT`
  asking the LLM to reference suggested actions by name in the
  interpretation prose.

### observer-gene
- No changes. Steering protocol contract unchanged.

### flux-universe.com (separate repo, separate session)
- Optional consumer simplification. Out of scope here.

---

## What This Does Not Cover

- Per-action confidence or rationale fields. Single confidence
  applies to the whole interpretation.
- Action ID validation against the active key at publish time
  (i.e. dropping IDs that don't exist). Falling back to
  `"action_<N>"` is sufficient signal that something is off; the
  steering receiver's safety gates already validate IDs.
- Live updates to `actions` between bootstrap and publish. The
  shared state map is updated by the WS subscriber, so resolution
  uses whatever is current at publish time.
