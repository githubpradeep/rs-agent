# Writing a skill

A skill is a single markdown file that teaches rs-agent a specific workflow — a debugging
checklist, a PR-review rubric, how your team writes commit messages, a niche tool's CLI, etc. It's
the same idea as Claude/Cursor "skills": plain markdown, no code, no plugin runtime required.

Use `/skills` to list and `/skill <name>` to inject one into the current conversation.

## Where skills live

rs-agent looks for skills in, in priority order (later overrides earlier on name collision):

1. `~/.rs-agent/skills/*.md` — user-global, available in every project
2. `.rs-agent/skills/*.md` — project-local, checked into the repo and shared with your team
3. `skills/*.md` at the repo root (this repo ships a few starter skills this way)

Each `*.md` file is one skill. The filename (minus `.md`) is the default skill name if the
frontmatter doesn't specify one.

## Format

```markdown
---
name: pr-review
description: Review a pull request diff for correctness, tests, and risk before approving.
triggers:
  - "review this PR"
  - "review the diff"
  - pr review
---

# PR review

When asked to review a PR or diff:

1. Read the full diff before commenting — don't review file-by-file in isolation.
2. Check: does it have tests for new behavior? Does it change public API/CLI flags without
   updating docs?
3. Flag anything touching auth, permissions, or destructive shell commands for extra scrutiny.
4. Summarize as: what changed, risk level, specific line comments, a clear approve/request-changes
   call.
```

### Frontmatter fields

| Field | Required | Description |
|-------|----------|--------------|
| `name` | No | Skill identifier used by `/skill <name>`. Defaults to the filename. |
| `description` | Yes | One-line summary shown in `/skills` and used to help the agent decide relevance. |
| `triggers` | No | Phrases/keywords that hint when this skill applies. Informational for now — matching is manual via `/skill <name>`, automatic trigger-matching is not yet implemented. |
| `tools` | No | **Skills 2.0.** Allow-list of built-in tool names (e.g. `[read, grep, ls]`). While the skill is active, only these tools are offered to the model (still subject to `/mode`). Omit or leave empty for no restriction. Cleared on `/new`. |

Example with a tool allow-list:

```yaml
---
name: explore-only
description: Read-only codebase exploration
tools: [read, grep, ls, find, webfetch, websearch]
---
```

Everything after the frontmatter is plain markdown instructions — write it the way you'd write
guidance for a new teammate. Keep it focused: one skill, one workflow. Prefer several short skills
over one sprawling one.

## Using a skill

- `/skills` — list all discovered skills (name + description), grouped by source (global /
  project / repo).
- `/skill <name>` — inject that skill's instructions into the current conversation before your
  next message.

Until automatic trigger-matching ships, `/skill <name>` is the reliable way to invoke a skill —
don't rely on the agent noticing `triggers` on its own yet.

## Tips for writing good skills

- Write imperative, checklist-style steps rather than prose explanations.
- Call out the exact tools to use (`read`, `grep`, `bash`, `repl`) when it matters — the agent
  already prefers `read`/`grep`/`edit` over raw `bash`, so only mention tools when you need to
  override or narrow that default.
- If the skill is about a large-context workflow (long docs, big diffs, whole-repo sweeps),
  mention the Deep Context `repl` tool (`llm_query`/`agent_query`) explicitly — that's the mechanism for
  keeping bulk content out of the main chat.
- Keep it under ~1-2 screens. If it's longer, split it into multiple skills.

## Sharing a skill

Project skills in `.rs-agent/skills/` are just files — check them into your repo like any other
source file so the whole team gets them. For personal skills you use across projects, keep them
in `~/.rs-agent/skills/`.

### Skill packs (zip)

Share a bundle without a registry:

```text
/skill-pack export              # all discovered skills → skills-pack.zip
/skill-pack export pr-review    # named skill(s) → pr-review-skills.zip
/skill-pack import ./pack.zip   # unpacks into ~/.rs-agent/skills/<pack>/
/reload                         # rediscover after import
```

Each zip includes `manifest.json` plus one `.md` per skill. A full marketplace is still future
work; packs are the portable unit for now.
