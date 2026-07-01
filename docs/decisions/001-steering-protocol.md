# ADR 001 — Steering Protocol

**Status:** Proposed
**Date:** 2026-04-05

---

## Context

knowledge-gene interprets observer-gene's state via LLM and publishes
narrative interpretations to Flux. The interpretation includes a
`suggested_actions` field, but this currently goes nowhere — `steer.rs`
is a no-op stub.

The goal is to close the loop: knowledge-gene's action suggestions should
reach observer-gene's regulation engine so the LLM's interpretive
intelligence can influence signal tuning decisions.

observer-gene already has a structured action space:
- **Hardcoded actions (100–125)**: AdjustDecay, AdjustBaseline,
  CoinDerivedSignal, GenAction, WritePrompt, ReadPrompt, ReloadActions
- **Runtime-generated actions (200+)**: Corrective actions coined by
  GenAction targeting specific signal combinations

The hardcoded actions are published in `observer-gene/key` under the
`actions` property. Runtime actions are persisted to `actions.json` on
disk but not published to Flux.

---

## Decision

### Communication: Flux Entity (Option A)

All steering flows through Flux. knowledge-gene publishes a
`knowledge-gene/steer` entity. observer-gene subscribes to it via its
existing WS connection.

No direct HTTP endpoints. No new servers. Consistent with how all data
moves in the gene architecture.

### Entity Format

Entity: `knowledge-gene/steer`
Stream: `knowledge.gene`

Properties:
- `action_ids` — array of integer action IDs from observer-gene's space
- `confidence` — float 0.0–1.0, the interpretation confidence that
  produced these suggestions
- `tick_ref` — the observer-gene tick this suggestion is based on
- `reason` — short string explaining why these actions were suggested

### Action Translation

knowledge-gene is responsible for translating LLM suggestions into
concrete action IDs. The LLM already receives the action key
(`observer-gene/key` → `actions` map) in its context. The system prompt
will be updated to instruct the LLM to return action IDs rather than
free-text suggestions.

Only hardcoded actions (100–125) are targetable. Runtime corrective
actions (200+) are auto-generated for specific signal corrections and
are not appropriate for LLM-directed steering. If future genes (Mind Gene)
need runtime action awareness, `observer-gene/key` can be extended then.

### Integration into Observer-Gene

Steered actions enter as a **third preference input** alongside the
existing two:
1. `ActionSelector.select()` — regulation engine (imbalance-driven)
2. `SelfModel.preferred_action()` — learned preference from history
3. **New**: Steered action from knowledge-gene

The `ActionEvaluator.select()` method gains a `steered_action: Option<u32>`
parameter. When present and the action passes all safety checks, it
competes with the other two preferences using the same scoring logic.
It does not override — it influences.

The evaluator already handles the regulation vs self-model tradeoff
using confidence and preference scores. The steered action enters the
same framework: if its confidence is high and the action scores well
against current imbalance, it wins. If not, the regulation engine's
choice prevails.

### Safety

All existing safety gates apply to steered actions without exception:

- **`harms_continuity` check**: Steered actions are filtered through
  `ImbalanceScorer::harms_continuity()` like every other action. If a
  steered action would harm continuity signals, it is rejected.
- **Action ID validation**: observer-gene validates the action ID exists
  in its current action space before considering it.
- **System action cooldowns**: If a steered action is a system action
  (GenAction, WritePrompt, etc.), existing per-action and global
  cooldowns apply.
- **Staleness check**: observer-gene ignores steer commands where
  `tick_ref` is more than 100,000 ticks behind the current tick.

knowledge-gene cannot bypass any safety mechanism. The entire point of
the safety layer is that nothing — not the self-model, not an external
gene — can override self-preservation.

### Confidence Gating

knowledge-gene only publishes to `knowledge-gene/steer` when the
interpretation confidence is ≥ 0.80. Below that threshold, the
interpretation is too uncertain to drive action selection.

observer-gene also checks the confidence field and ignores steered
actions with confidence < 0.80 (defense in depth).

### Cooldown

knowledge-gene publishes steer at most once per interpretation cycle
(currently ~5 minutes with qwen3.5:35b). No additional cooldown needed
beyond what's already in the interpretation interval and the early
cooldown mechanism.

observer-gene processes the steer entity on its next tick after receiving
the WS update. It uses the steered action for a single tick cycle, then
clears it. It does not repeatedly execute a stale steer command.

---

## Scope of Changes

### knowledge-gene
- Update system prompt to request action IDs instead of free-text
- Update `Interpretation` struct: `suggested_actions` becomes `Vec<u32>`
  (or add a new `suggested_action_ids` field alongside existing)
- Implement `steer.rs`: publish `knowledge-gene/steer` to Flux when
  confidence ≥ 0.80 and action IDs are non-empty
- Confidence gating logic in the main loop

### observer-gene
- Subscribe to `knowledge-gene/steer` entity via existing WS connection
- Parse steer entity into a steered action candidate
- Extend `ActionEvaluator.select()` with steered action parameter
- Validate action ID, apply safety checks, feed into selection

---

## What This Does Not Cover

- Runtime action (200+) awareness — future Mind Gene scope
- Multiple simultaneous steered actions — first valid ID wins
- Steering priority escalation — no "force" mode, ever
- Bidirectional negotiation — observer-gene does not respond to
  knowledge-gene about whether it accepted the suggestion
