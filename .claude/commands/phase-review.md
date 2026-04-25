---
description: "Review a completed development phase. Rechecks timeline, validates code quality and conventions, detects over-engineering and missing edge cases, updates docs, and commits with the phase number."
allowed-tools: Read Write Bash(*)
---

# Phase Review — Validate and Commit Completed Phase

You are reviewing a development phase that was built with `/phase-start`. Your job is to be thorough: check quality, find gaps, and only commit when everything is solid.

## Step 1 — Identify the Phase

Read `docs/TIMELINE.md` and identify which phase was just completed (the most recent uncompleted phase).

## Step 2 — Timeline Compliance

Re-read the phase's scope, milestones, and acceptance criteria from `docs/TIMELINE.md`.

For each milestone:
- [ ] Is it implemented?
- [ ] Does it meet the acceptance criteria?
- [ ] Is anything missing?

Report: "X of Y milestones complete. Missing: [list]"

If milestones are missing, ask the user whether to fix them now or skip them.

## Step 3 — Code Quality Review

Read `docs/CONVENTIONS.md` and review all code created or modified in this phase.

Check:
- [ ] **File structure** — matches conventions in CONVENTIONS.md
- [ ] **Naming** — files, variables, functions follow conventions
- [ ] **Architecture** — follows prescribed patterns (not inventing new ones)
- [ ] **Error handling** — errors are caught and handled as specified
- [ ] **Security** — no hardcoded secrets, input validation where needed
- [ ] **Code style** — formatting, imports, file length within limits

Report issues found with specific file paths and line references.

## Step 4 — Over-Engineering Detection

Look for:
- Abstractions that aren't needed yet (YAGNI)
- Complex patterns where simple code would work
- Premature optimization
- Features that weren't in the phase scope (scope creep)
- Unnecessary dependencies added

If found, suggest simplifications.

## Step 5 — Edge Case Review

For each feature in this phase:
- What happens with empty/null/undefined input?
- What happens with very large input?
- What happens if a network request fails?
- What happens if the user does something unexpected?
- Are there race conditions or timing issues?

Report any unhandled edge cases. Ask the user which ones to fix now vs. defer.

## Step 6 — Automated Checks

Run all applicable checks:
1. **Build**: Does the project build without errors?
2. **Lint**: Any linting issues? (include Rust clippy with zero warnings)
3. **Type check**: Any type errors? (if applicable)
4. **Tests**: Do all tests pass?

Report results. Fix any failures before proceeding.

## Step 7 — Follow-Up Questions

Ask the user:
- Did the manual testing pass? Any issues found?
- Are there any changes you want before we commit this phase?
- Should any edge cases from Step 5 be addressed now?

Wait for user responses and address any issues.

## Step 8 — Documentation Update

If anything changed during this phase that affects the docs:
- Update `docs/TIMELINE.md` — mark the phase as complete (add ✅)
- Update `docs/README.md` — if new docs were added
- Update `CLAUDE.md` — if project context changed
- Update `docs/CONVENTIONS.md` — if new patterns were established

Show doc changes to the user for approval.

## Step 9 — Commit

Once everything passes and the user confirms:

1. `git add -A`
2. `git commit -m "chore(phase): complete phase N - [phase title from timeline]"`

Replace `N` with the actual phase number and use the phase title from the timeline.

Tell the user: "Phase N committed. Run `/phase-start` when you're ready for the next phase."

## Critical Rules

- **Be honest about issues** — do not rubber-stamp bad code
- **Check every milestone** — do not skip acceptance criteria
- **Do not auto-fix without asking** — report issues and let the user decide
- **Do not commit with failing tests** — all automated checks must pass
- **Do not commit without user confirmation** — always ask before the final commit
- **Update docs** — the timeline MUST be updated to mark the phase complete
