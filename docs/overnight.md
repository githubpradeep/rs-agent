# Overnight factory

Run unattended bead work while you sleep. Phase 1 of the “chariot → city” loop:
**beads ready queue → claim with lease → implement → verify → close**, with crash recovery so a dropped provider stream does not wedge the factory.

## Prerequisites

1. Project cwd with credentials for your provider (same as interactive `rs-agent`).
2. Open beads whose deps are closed (ready queue).
3. Prefer YOLO/auto-approve for unattended runs (`worker` always runs approved).

Create work with the `bead` tool or by editing `.rs-agent/beads.json`. Ready = `open` (or reclaimable expired claim) + all `deps` closed + not `gated`.

## Headless worker

```bash
# One claim → turn → exit
rs-agent worker --once -a

# Overnight loop until budget or empty queue
rs-agent worker --loop --budget-minutes 480 -a

# Named seat (for diary / multi-process identity)
rs-agent worker --loop --budget-minutes 480 --seat Fleet-1 -a
```

| Flag | Meaning |
|------|---------|
| `--once` / default without `--loop` | Claim at most one ready bead |
| `--loop` | Keep claiming until budget or no ready beads |
| `--budget-minutes N` | Wall-clock stop (default 480) |
| `--seat NAME` | Claimant identity (default `worker-<pid>`) |
| `--fail-fast` | Exit the loop on first transport/tool failure |
| `--sleep-secs N` | Pause between claims when looping (default 5) |

Status for the TUI: `.rs-agent/worker-status.json` (read via `/worker`).

### Concurrency

Two workers cannot claim the same bead: claims use a lockfile beside `beads.json` and a **lease** (`claimant` + `lease_expires`). Stale leases are reclaimable (`bead reclaim` / worker reclaim on start).

## Beads v2 (ready queue)

| Concept | Behavior |
|---------|----------|
| `deps` | Bead is ready only when every dep is `closed` |
| `parent` | Optional epic/child link |
| `priority` | Lower sorts first among ready |
| `claim` / lease | Sets claimant + expiry; `heartbeat` extends |
| `gated` | Blocked until `ungate` (external gate) |
| Soft goals | Vague “keep implementing…” goals do **not** achieve while ready/open beads remain |

TUI:

```
/beads           # counts + full list
/beads ready     # ready queue only
/worker          # last headless worker status
```

Footer shows `beads:N ready` when the backlog is non-empty.

## Project brain

- Doctrine: `brain/*.md` (capped on wake)
- Facts: `brain/facts.jsonl` via `/brain remember <fact>` or `brain::remember`
- On wake (interactive resume or worker start), the session is primed with brain + ready beads + handoff notes (no full chat history required)

## Crash resilience

| Failure | Behavior |
|---------|----------|
| Provider stream drop | Retries with backoff; then settle dangling tools, restore turn snapshot, `[recover]` note, pause handoff |
| Mid-`write` / `edit` | Atomic temp + rename |
| Worker killed mid-claim | Lease expires → reclaim; next worker continues from wake/handoff |
| Soft `/goal` | Won’t declare victory while ready beads remain |

After a stream failure in the TUI: status shows recover/retry; type `continue` or `/handoff`. Session stays continuable with `-r`.

## Recommended overnight recipe

```bash
# Daytime: design beads (deps, parents), remember doctrine facts
rs-agent   # /beads, /brain remember, /goal if interactive

# Night
cd /path/to/project
rs-agent worker --loop --budget-minutes 480 --seat Fleet-1 -a

# Morning
rs-agent   # /beads ready, /worker, review closed notes
```

Hard stop goal for interactive sessions: `/goal no open beads` (not a soft “keep implementing…” phrase).

## Watching a worker (Phase A)

Workers write live state under the project:

```text
.rs-agent/fleet/<seat>.status.json   # bead, tool, heartbeat, session id, pid
.rs-agent/fleet/<seat>.log           # rolling transcript (tools/text)
.rs-agent/fleet/<seat>.pid           # launcher / worker pid
.rs-agent/fleet/<seat>.control.jsonl # TUI → worker pause/resume/abort/steer
```

```fish
# One command — two workers (Phase B); seals Fleet caste
$BIN -a fleet up --seats Fleet-1,Fleet-2 --budget-minutes 480

# Watch (CLI)
$BIN fleet status
$BIN fleet logs Fleet-1
$BIN marshal --once
# or overnight: $BIN marshal --loop --interval-secs 90

# Intake / roles (Phases F/H) — see docs/city-ops.md
$BIN wish "port Softmax" --auto
$BIN role --seat Beadle --once

# Stop
$BIN fleet down
```

### TUI city cockpit (Phase B)

In an interactive `rs-agent` session (same project cwd as the fleet):

| Command | Behavior |
|---------|----------|
| `/city` or `/seat` or `/fleet` | Seat board (state, bead, tool, heartbeat) |
| `/seat follow Fleet-2` | Live formatted log; worker keeps running |
| `/seat steer <text>` | Inject steer into followed/attached worker |
| `/seat abort` | Abort current worker turn (follow) or attached turn |
| `/seat open Fleet-2` | Inspect session + logs **without** pausing |
| `/seat attach Fleet-2` | Pause worker, load session, chat as the seat |
| `/seat detach` or `/detach` | Save + resume background worker |
| `/fleet up` / `/fleet down` | Launch / stop workers (unchanged) |

`/fleet follow|attach|…` remain aliases. Footer shows `FOLLOW` / `ATTACHING` / `ATTACHED` / `INSPECT`. Quitting while attached sends `resume` (pause also auto-expires ~10 minutes).

Identity commands still work: `/seat Fleet-1`, `/seat caste …`, `/seat model …`.

Or one worker in the foreground:

```fish
$BIN -a worker --loop --budget-minutes 480 --seat Fleet-1
```

Idle messages now say whether the backlog is **empty** vs **open but not ready** (deps/gated), so sleeping no longer looks like “working.”

Full city operator manual: [`docs/city-ops.md`](city-ops.md) · roadmap: [`docs/city-roadmap.md`](city-roadmap.md).

---

## Explicit non-goals (still)

Thunderdome as merge bot, Emacs cockpits, Max-account credential rotation — see city roadmap.

## Phase 2 — Producer / consumer / review

### Pipeline beads

Kinds: `design` → close spawns `implement` → close spawns `review`.

```
bead add kind=design title="Auth redesign"
# … design work …
bead close id=b1
# spawns Implement: Auth redesign
bead close id=b2
# spawns Review: …
bead fail id=b3 reason="missing tests"   # reopens implement
bead land id=b2                          # OK only after a passed review
```

TUI/tool: create with `kind`, close advances the pipeline; `fail` on review reopens implement.

### Thin fleet

Run multiple workers with different seats (leases prevent double-claim):

```bash
# Strong designer seat (optional model override on the seat profile)
rs-agent -a worker --loop --seat Fleet-1 --budget-minutes 480
rs-agent -a worker --loop --seat Fleet-2 --budget-minutes 480
```

In TUI: `/seat Fleet-1` then `/seat model claude-sonnet-4-20250514` (and `/seat role marshal` for the admin seat). Worker loads seat `model` / `provider` overrides.

### Marshal

```bash
rs-agent marshal --once
```

Reclaims stale leases and prints ready queue + fleet leases. TUI: `/marshal` (same) and `/fleet` (status only).

### Review enforcement

- Closing an implement bead with notes containing `land`/`ship` requires a passed review (`bead land`).
- Ordinary implement close still spawns a review (pipeline).
- Hook: `.rs-agent/hooks/before_bead_close` — stdin bead JSON; non-zero blocks close.
