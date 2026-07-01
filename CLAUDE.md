---
---
# gene-knowledge — Development Guide

**Last Updated:** 2026-03-13
**Current Phase:** Phase 1 — PLANNING

---

## Project Overview

**gene-knowledge** — [ONE LINE DESCRIPTION]

**Architecture:** [STACK SUMMARY]
**Full Details:** `/docs/decisions/001-core-architecture.md`

**Infrastructure:**
- [ADD SERVICES, PORTS, DB CONNECTIONS HERE]

---

## User Preferences (Apply Always)

- Never be verbose — clear, concise responses
- Small, incremental changes — don't build everything at once
- Ask before expanding scope — stay within defined task
- Document as you go — ADRs for decisions, session notes for progress
- Show, don't just talk about it
- Always check the codebase — read files, never rely on memory
- Plan before implementing — get approval on approach first

---

## What NOT to Do

- Don't implement without explicit approval
- Don't build features not in the current phase
- Don't add "nice to have" features without discussion
- Don't create files unless explicitly requested
- Don't run the application — user runs all services
- Don't repeat a failed approach more than once — stop and ask
- Don't be verbose
- Don't assume environment — confirm before acting on infra

---

## Mandatory Workflow

For EVERY change, task, or request:

1. **STOP** — Do not implement yet
2. **READ** — Read existing related files
3. **VERIFY** — Check schemas, callers, dependencies
4. **PROPOSE** — Show what you plan to change and why
5. **WAIT** — Do NOT implement until explicitly approved
6. **IMPLEMENT** — After approval, implement exactly what was approved

**Breaking this workflow = wasted work.**

---

## Verification Checklist (Before Every Code Change)

- [ ] Read files you'll modify
- [ ] Check if files/functions already exist
- [ ] Verify schemas and signatures match usage
- [ ] Find all callers before changing signatures
- [ ] Confirm dependencies are available
- [ ] Follow existing patterns and style

---

## Session Types

**Foundation Session (odin-f)**
- Architecture, ADRs, task breakdown, verification
- Produces task prompts saved to `.odin/tasks/`
- Model: Claude (foundation reasoning)

**Implementation Session (odin-i)**
- Executes one scoped task — reads and writes files
- Auto-loads CLAUDE.md + latest task prompt
- Model: Local (Ollama on Bosgame) or Claude fallback
- Reports summary back to foundation session

---

## Session Notes (MANDATORY)

After every implementation session, create a note in `/docs/sessions/`:
- Max 300 lines — summary only, not documentation
- What was built (bullet points)
- Key decisions made
- Issues encountered
- Next steps
- NO code examples, NO detailed explanations

---

## Documentation Structure

```
/docs/
  /decisions/    ← ADRs (001-core-architecture.md, 002-...)
  /sessions/     ← Implementation session notes
  /workflow/     ← multi-session-workflow.md
/CLAUDE.md       ← This file (rules, never status tracking)
/.odin/
  /tasks/        ← Task prompts from foundation sessions
  /sessions/     ← Session metadata
```

**CLAUDE.md is the rulebook, not a status tracker.**
**Status and progress belong in session notes.**

---

## ADR Rules

- Never include code examples in ADRs
- Never include database schemas in ADRs
- Never include "Alternatives Considered" sections
- Keep ADRs 300-400 lines max
- Code belongs in implementation, schemas in migration files

---

## Lessons Learned

_(Add corrections and recurring mistakes here as they occur)_

---

## Key Files

- This file: Rules and guidelines for all sessions
- `/docs/decisions/001-core-architecture.md`: Full architecture
- `/docs/workflow/multi-session-workflow.md`: Development process
- `/docs/sessions/`: Progress tracking
- `/.odin/tasks/`: Implementation task prompts
