# Productize plan (90 days)

**North star:** a local overnight coding factory. Leave a backlog; workers
implement; you review in the morning. Deep Context keeps huge files out of the
window. Daily TUI is the on-ramp, not the headline.

**Do not raise. Do not add City offices.** Reassess with the `assess-rs-agent`
skill after each sprint.

Competitor gap list (pi / OpenCode) lives in [`PLAN.md`](../PLAN.md) as history.
This file is what we execute now.

---

## Freeze (until v0.2 dogfood)

Do not start:

- New standing roles (Beadle-class offices)
- Desktop / Tauri
- Plugin marketplace / SDK product
- Windows (unless a dogfooder is blocked)
- MCP OAuth
- Selling Deep Context as a library
- README as a City glossary

---

## Sprint 0 — This week: make it installable and understandable

Goal: a stranger can install, see `/tree`, and repeat the one-liner.

| # | Task | Done when |
|---|---|---|
| 0.1 | Rewrite README opener + one-liner to the overnight-factory pitch. City vocabulary in `docs/city-ops.md` only. | First screen of README does not mention Beadle/Gargoyle/Moot | **done** |
| 0.2 | Capture `docs/img/` shots from [`screenshots.md`](screenshots.md): `/tree` mid-Deep-Context, idle TUI, City follow (optional). | PNGs in repo, linked from README | **needs TUI recording** |
| 0.3 | 90s demo: existing [`demo.md`](demo.md) Deep Context clip **plus** 30s `wish` → `fleet up` → `/beads` morning. | One asciinema or mp4 linked from README | talk track done; recording pending |
| 0.4 | Trust one-pager: YOLO overnight, unsandboxed `repl`, what the binary may touch. | `docs/trust.md` linked from README | **done** |
| 0.5 | Tag **v0.1.0** (ask before `git tag`). Release notes = pitch + known limits. Confirm `scripts/install.sh` downloads. | `curl \| bash` works on macOS aarch64 and Linux x86_64 | **blocked on ask** |

If time is short, order is **0.1 → 0.5 → 0.2**. A working install beats a pretty README. A 404 install script is worse than no script.

---

## Sprint 1 — Overnight must not clobber itself

Goal: two fleet seats can run a night without sharing one dirty worktree.

| # | Task | Done when |
|---|---|---|
| 1.1 | `fleet up` creates or requires a worktree per seat (git worktree or documented fail). | Two seats cannot write the same files by default | **done** (`--shared-worktree` opt-out) |
| 1.2 | Worker integration tests (claim → heartbeat → recover/reclaim). | `src/worker` is not 0 tests |
| 1.3 | REPL trust: document clearly; if cheap, optional Docker/isolated python. Do not block the tag on Docker. | README + trust.md state the model; sandbox remains explicit later |

---

## Sprint 2 — Default UI is the factory, not the mythology

Goal: day-1 path is chat → wish → workers → review.

| # | Task | Done when |
|---|---|---|
| 2.1 | Default TUI: wish, workers, beads, Deep Context tree. Marshal/mail/moot/laurels behind `/ops` or docs. | New user `/help` fits on one screen of real work |
| 2.2 | Soften slash catalog; keep operator commands, stop leading with them. | Palette is not 40 peers of equal rank |
| 2.3 | Gargoyle/Drawbridge: either project-aware (detect test command) or hide from default docs. | No hardcoded `cargo test --lib` as “the city” |

---

## Sprint 3 — Proof, then people

Goal: evidence a stranger can believe; then three outsiders.

| # | Task | Done when |
|---|---|---|
| 3.1 | Publish one eval chart: Deep Context vs dump-into-chat on the outage log; overnight close-rate on a fixed bead set. | Not a unit test; live provider; checked in or linked |
| 3.2 | Dogfood **3 people who are not the author**. Capture install friction. | `TODO.md` dogfood box checked with names/notes |
| 3.3 | Tag **v0.2.0** only after 3.2. | Release notes include what outsiders broke |

---

## Definition of done for “productized enough to talk funding”

All of:

1. `curl | bash` works.
2. 90s demo exists.
3. Two-seat overnight does not share a worktree.
4. Three outsiders ran it for a week.
5. One eval that is not `cargo test`.

Until then: craft/portfolio, not a seed deck.

---

## First action after this file lands

Sprint 0.1 + 0.4 + 1.1 are in tree. Next: **0.5 tag v0.1.0** (ask), then TUI
screenshots / 90s recording (0.2–0.3). Worker tests (1.2) after that.
