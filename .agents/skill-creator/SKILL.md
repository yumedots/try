---
name: skill-creator
description: Create new project skills and update existing ones under a project's skills folder. Use whenever the user states a new rule, convention, or workflow, asks to turn something into a skill, or a change establishes a reusable convention worth capturing.
---

# Skill creator

Keep a project's binding instructions as skills — one folder per skill, each containing a `SKILL.md` — under whatever skills directory the project uses. When a new rule or convention appears, capture it as a skill so top-level project docs stay minimal.

## When to create or update

A new rule, convention, or "always do X" → new skill if it's a distinct domain, otherwise fold into the existing skill it belongs to. A reusable procedure worth repeating → capture it. A stale, contradictory, or incomplete skill → update in place rather than duplicate.

## Before writing

Check the skills directory first. Read existing skills so the new one matches the project's style and doesn't duplicate one that exists. Prefer updating an existing skill over creating a new one when the rule belongs to an existing domain.

## SKILL.md structure

`<skills-dir>/<name>/SKILL.md`, with frontmatter (`name`, `description`) plus a title and imperative instructions.

`name`: lowercase letters, digits, hyphens only; must match the directory name. `description`: the triggering mechanism — what the skill does and the specific situations that should trigger it, slightly over-specified so it isn't under-triggered.

Write imperative instructions. Keep `SKILL.md` short — move long reference material into a subfolder and point to it. Explain the "why" behind hard rules instead of stacking mandates. Match the project's existing tone.

## Workflow

Capture intent from the conversation — the user's exact wording, corrections. Draft the `SKILL.md`. Confirm the name matches the folder. Leave other project docs unchanged — skills are discovered by scanning the folder, not by being listed elsewhere.
