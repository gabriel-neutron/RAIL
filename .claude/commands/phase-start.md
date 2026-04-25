---
description: "Start the next development phase from the timeline. Commits current work, reviews the plan, generates a detailed breakdown, and builds the feature after user confirmation."
allowed-tools: Read Write Bash(*)
---

# Phase Start — Begin Next Development Phase

You are starting a new development phase. This is a structured process: plan first, then build.

## Step 1 — Save Current State

Before anything else:
1. Check if there are uncommitted changes: `git status`
2. If there are changes, commit them with a conventional message, for example:
   - `git add -A`
   - `git commit -m "chore(phase): checkpoint before starting next phase"`
3. If Git is not available, warn the user to save their work manually

## Step 2 — Read All Context

Read and align with ALL of these documents:
- `CLAUDE.md`
- `docs/README.md`
- `docs/PRD.md`
- `docs/TIMELINE.md`
- `docs/CONVENTIONS.md`
- `docs/TECH_STACK.md`
- `docs/ARCHITECTURE.md`
- `docs/HARDWARE.md`
- `docs/SIGNALS.md`
- `docs/DSP.md`
- `docs/PERF.md`

## Step 3 — Identify the Next Phase

From `docs/TIMELINE.md`:
1. Find all completed phases (marked with ✅ or noted as complete)
2. Identify the next uncompleted phase
3. If all phases are complete, tell the user and stop

## Step 4 — Plan (Before Coding)

Present to the user:

### Phase Overview
- Phase name and number
- Scope: what's being built
- Dependencies: what must exist (verify these are in place)

### Milestone Breakdown
Break the phase into small, concrete milestones. Each milestone should be:
- Independently verifiable
- Small enough to complete without losing context
- Ordered by dependency

### Missing Requirements Check
- Are there any ambiguities in the PRD for this phase's features?
- Are there any technical decisions that need to be made?
- Are there any dependencies that aren't yet installed?

If there are questions or missing requirements, **ask the user before proceeding**.

### Estimated Approach
- Which files will be created or modified?
- What patterns from `CONVENTIONS.md` apply?
- Any risks or edge cases to watch for?

## Step 5 — Get Confirmation

Tell the user: "Here's my plan for this phase. Should I proceed?"

**Do NOT start coding until the user confirms.**

## Step 6 — Build

After confirmation, work through each milestone:
1. Implement the milestone
2. Run relevant checks (lint, type check, tests) after each milestone
3. Report results
4. Move to the next milestone

## Step 7 — Request Testing

When all milestones are complete:
1. Run all automated checks (build, lint, tests)
2. Report results
3. If there are features that need manual testing, describe exactly what the user should test and what to look for
4. Ask the user to test and report back

**Do NOT commit after this step.** The `/phase-review` command handles the commit.

Tell the user: "Phase development is complete. Please test, then run `/phase-review` when ready."

## Critical Rules

- **Always plan before coding** — never jump straight into implementation
- **Ask clarification questions** before building if anything is unclear
- **Follow CONVENTIONS.md** — every file, name, and pattern must comply
- **Do not skip milestones** — work through them in order
- **Do not modify docs** unless a change is explicitly needed and user-approved
- **Report check/test results** — never silently skip failing tests
- **Do not commit the phase** — that's for `/phase-review`
