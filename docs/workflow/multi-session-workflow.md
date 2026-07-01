# Multi-Session Development Workflow

**Purpose:** Coordinate foundation and implementation sessions for complex projects.

---

## Core Concept

One **foundation session** coordinates multiple **implementation sessions** that report back for verification before proceeding. Zero copy/paste — task prompts are written to `.odin/tasks/` and auto-loaded by `odin-i`.

---

## Session Types

### Foundation Session (`odin-f`)
- Architecture, ADRs, task breakdown, verification
- Persists throughout the project or phase
- Produces task prompts → writes to `.odin/tasks/task-[name].md`
- Verifies implementation summaries before proceeding
- Model: Claude

### Implementation Session (`odin-i`)
- Executes one scoped task
- Auto-loads CLAUDE.md + latest task prompt from `.odin/tasks/`
- Reports summary back (written to `.odin/tasks/task-[name]-summary.md`)
- Model: Local Ollama model (Bosgame) or Claude fallback

### Ideation Session (optional, external)
- ChatGPT or Claude for freeform idea exploration
- Output fed into `odin-f` as starting context

---

## Workflow

### New Project
```
odin-f "idea or project description"
  → scaffolds project structure
  → foundation session starts with idea pre-loaded
  → produces ADR-001 + Phase 1 task breakdown
  → writes first task to .odin/tasks/task-1.1.md
```

### Implementation Loop
```
1. odin-f produces task prompt
2. Foundation writes it: .odin/tasks/task-[name].md
3. User runs: odin-i (auto-loads task prompt)
4. Implementation executes task, writes files
5. Implementation writes summary: .odin/tasks/task-[name]-summary.md
6. User runs: odin-f (loads summary via --rule)
7. Foundation verifies, produces next task
8. Repeat
```

### Task Prompt Format

Foundation sessions must write task prompts in this format:

```markdown
# Task [X.Y] — [Task Name]

## Context
[Brief context from ADR or previous session]

## Scope
[Exactly what to implement — no more, no less]

## Files to Read First
- [file1]
- [file2]

## Files to Create/Modify
- [file1]: [what to do]

## Out of Scope
- [what NOT to do]

## Definition of Done
- [ ] [criteria 1]
- [ ] [criteria 2]
```

### Summary Format

Implementation sessions must write summaries in this format:

```markdown
# Summary — Task [X.Y] — [Task Name]

## Status: COMPLETE / PARTIAL / BLOCKED

## What Was Done
- [bullet points only]

## Files Changed
- [file]: [what changed]

## Issues Encountered
- [any blockers or decisions made]

## Next Steps
- [what foundation session should know]
```

---

## Project Structure

```
project/
├── CLAUDE.md                    ← Rules (auto-loaded by odin-i)
├── README.md
├── .odin/
│   ├── tasks/                   ← Task prompts + summaries
│   │   ├── task-1.1.md
│   │   ├── task-1.1-summary.md
│   │   └── task-1.2.md
│   └── sessions/                ← Session metadata
├── docs/
│   ├── decisions/               ← ADRs
│   │   └── 001-core-architecture.md
│   ├── sessions/                ← Implementation notes
│   └── workflow/
│       └── multi-session-workflow.md
└── src/
```

---

## Rules

**Foundation sessions:**
- Always verify implementation by reading changed files, not trusting the summary alone
- Keep task prompts scoped to 1-3 related tasks max
- Write task prompt to file before ending session

**Implementation sessions:**
- Read CLAUDE.md first, always
- Read the task prompt completely before touching any file
- Do not expand scope without asking
- Do not run the application
- Write summary to file when done

**Both:**
- Session notes max 300 lines
- ADRs max 400 lines, no code examples
