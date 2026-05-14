---
name: roadmap-planning
description: Use when creating, updating, validating, or executing delegable roadmap plans under a repo's roadmap directory, especially for Gode feature phases and document-first planning workflows.
metadata:
  short-description: Create and maintain roadmap plans
---

# Roadmap Planning

Use this skill when the user asks to turn an idea into a roadmap, add a roadmap phase, split work for agents, or work through an existing roadmap document.

## Workflow

1. Inspect the current roadmap index and nearby phase plans before writing.
   - In Gode, start with `roadmap/00-feature-inventory-and-sequencing.md`.
   - Read the closest existing phase plans and relevant source files so the new plan is grounded in the repo.
2. Produce durable artifacts, not chat-only planning.
   - Add or update `roadmap/{NN}-{feature-slug}.md`.
   - Update the phase map and any phase-count checklist in the roadmap index.
3. Use the standard implementation-plan header:

```markdown
# [Feature Name] Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** [one sentence]
**Architecture:** [2-3 sentences]
**Tech Stack:** [key technologies]

---
```

4. Make the plan delegable.
   - Include `## Owned Paths`, `## Dependency Checks`, `## Tasks`, per-task `Run:` commands, and `Acceptance:` criteria.
   - Split tasks by ownership boundary so separate agents can work without shared write sets.
   - Name exact files and packages to inspect or modify.
   - Include local-only verification paths; guard live provider or network checks behind explicit env vars.
5. Keep the roadmap document as the source of truth.
   - When executing a roadmap, mark task checkboxes as work completes.
   - If implementation changes the plan, update the document before reporting completion.
   - Do not replace the roadmap with an untracked scratch note.
   - Treat threads as execution lanes that can attach to roadmap tasks; transcripts are evidence, not the roadmap state.
6. Keep language repo-owned.
   - Mention external references only when the user asks for direct parity or source comparison.
   - Avoid making plans read like copied product requirements from another project.

## Validation

Run focused checks after editing roadmap files:

```sh
for f in roadmap/*.md; do
  printf '%s\t' "$f"
  rg -q '^# ' "$f" && printf 'heading '
  rg -q '^\\*\\*Goal:\\*\\*' "$f" && printf 'goal '
  rg -q '^\\*\\*Architecture:\\*\\*' "$f" && printf 'arch '
  rg -q '^## Owned Paths' "$f" && printf 'owned '
  rg -q '^## Tasks' "$f" && printf 'tasks '
  rg -q '^Run:' "$f" && printf 'run '
  rg -q '^Acceptance:' "$f" && printf 'accept '
  printf '\n'
done
```

Also scan for stale phase counts, placeholder text, unintended reference-project names, and unchecked acceptance gaps.
