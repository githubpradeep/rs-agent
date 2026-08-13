---
name: assess-rs-agent
description: >-
  Reassess rs-agent as a product: features, reliability, City/factory honesty,
  Deep Context uniqueness, distribution, pitch, and funding value. Use when the
  user asks to assess, revisit, review, productize, pitch, or judge funding
  value of rs-agent; when they ask what is good vs what to improve; or when they
  mention the product review canvas.
---

# Assess rs-agent

Repeatable product review of **this repo**, not a recap of the last chat.

Honesty over cheerleading. Grade shipped behavior, not docs checkboxes. Two
products in one binary (daily TUI vs overnight factory) is a finding, not a
feature list.

## When this runs

User asks to assess / revisit / productize / pitch / funding-value the agent.
Do not wait for extra confirmation.

## Output

1. Short chat verdict (one paragraph + the 3–5 actions that matter now).
2. Refresh the canvas at
   `~/.cursor/projects/<workspace>/canvases/rs-agent-product-review.canvas.tsx`
   using the Cursor canvas skill. Same tabs: Overview, Already good, Needs work,
   How to pitch, Funding. Date the snapshot. Re-score from **today's tree**,
   do not copy prior numbers.

Read `~/.cursor/skills-cursor/canvas/SKILL.md` before writing the canvas.

## Workflow

Copy and track:

```
Assess:
- [ ] Gather snapshot (script + git/gh)
- [ ] Probe core loop, Deep Context, City honesty
- [ ] Check distribution and competitors
- [ ] Score rubric
- [ ] Write canvas + chat verdict
```

### 1. Gather snapshot

From the repo root, run [scripts/gather.sh](scripts/gather.sh). Then skim
`README.md`, `PLAN.md`, `TODO.md`, `docs/city-roadmap.md`, `docs/overnight.md`.

Also collect (script may miss some):

- `git log --oneline -20`, first commit date, tags, `git status -sb`
- `gh repo view` / `gh release list` if available
- Whether `scripts/install.sh` would work (tags + artifacts exist)
- Test density: `#[test]` in `src/worker`, `src/tui`, `src/rlm`, `src/beads`
- Live evals: `evals/README.md` and CI `rlm-demo` (`continue-on-error`?)

### 2. Probe honesty (required)

Do not trust roadmap “implemented” marks. Open the code:

| Claim | Probe |
|---|---|
| Deep Context USP | Is RLM a sidecar `repl` tool or the action space? Auto-escalate still real? Sandbox? (`src/rlm/repl.rs`) |
| Overnight factory | `worker --loop` leases, heartbeat, crash recover (`src/worker`, `src/agent/loop.rs`) |
| Fleet | `fleet up` spawns processes? Worktree isolation enforced? |
| City offices | Beadle / Gargoyle / Drawbridge / Scryer: real watchers or one-shot stubs? (`src/roles/mod.rs`) |
| Permissions | YOLO required for unattended? Path allow + danger heuristics? |
| Distribution | Tag, screenshots, Windows, name collision (`Protocol-Lattice/rs-agent`) |

Facade test: if `/fleet` or a role exists, it must start/stop/observe real
processes or mutate the bead graph. A status reprint is a **Fail**.

### 3. Competitors (re-check, do not freeze)

Re-search current public state. At minimum:

- Daily TUI: Claude Code, Cursor, Codex CLI, OpenCode, pi
- RLM: Prime Agent (Prime Intellect) + the RLM paper
- Overnight / factory: anything shipping local unattended implement+review

Deep Context is **not unique by default** after Prime Agent. Only score it
unique if the tree still has a wedge they do not ship (typically: RLM + local
bead factory together).

### 4. Score

Use [rubric.md](rubric.md). Four headline stats on the canvas:

- Engineering maturity /10
- Product readiness /10
- Funding readiness /10
- Public traction (stars / releases / known dogfooders)

Capability rows: Have / Thin / Fake / Fail — with a one-line honest read.

### 5. Productize + funding

Default wedge unless evidence changed:

> Local overnight coding factory. Leave a backlog; workers implement; review
> in the morning. Big context stays out of the window.

- Daily TUI = on-ramp, not the headline (checklist war vs Claude Code).
- Do not lead with Beadle / Gargoyle / Seneschal / Moot / Laurels.
- Do not sell “Cursor killer” or an RLM library.
- Funding: **latent until** working install + 3 outsider dogfooders + one eval
  that is not a unit test. Raise-now default is **No**.

90-day list: only what is still undone. Drop completed items; do not keep a
stale “tag v0.1.0” if it shipped.

## Voice

Lead with the answer. Be specific (file/module names, commands). No emoji.
Do not inflate scores because the last review was kind.
