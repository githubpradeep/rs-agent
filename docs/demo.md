# Demo kit — overnight factory + Deep Context

Goal: ~90 seconds. Stranger understands: work continues after you close the
laptop, and huge files never eat the chat.

Use a **reliable paid model** (Anthropic Sonnet / similar). Tiny/free models
often break tool calls and kill the vibe.

---

## 90-second talk track

1. **Open (5s):** “Local overnight coding factory. Leave a backlog; workers
   implement; you review in the morning.”
2. **Deep Context (45s):** “Here’s a ~180KB checkout outage log. When did it
   start, root cause, red herring?” Show `[rlm_escalate]` / `repl`, `[D]`,
   `/tree` — chat did not eat the file.
3. **Overnight (30s):** `wish "…"` → `fleet up` (or `c` then `u` in the TUI).
   Point at a worker heartbeat / last tool. “Morning is `/beads` + review.”
4. **Close (10s):** “Same binary for daily edit/bash. YOLO overnight — read
   the trust page before `-a`.”

Do **not** open with office names, RLM-the-paper, or a 1000-model catalog.

---

## Prep (once)

```bash
cd /path/to/rs-agent
cargo build --release
export ANTHROPIC_API_KEY=sk-ant-...
python3 --version
```

Optional: `approve = true` in `~/.rs-agent/config.toml` so permissions don’t
interrupt a recording. That is YOLO — [`docs/trust.md`](trust.md).

---

## A. Deep Context clip (public / HN)

```bash
# One-shot (asciinema / CI-ish)
./scripts/demo-deep-context.sh --provider anthropic

# Interactive TUI (best for video — shows [D] and /tree)
./scripts/demo-deep-context.sh --tui --provider anthropic
```

Corpus: `example/demo/outage.log` — checkout-api logs, buried
`DB_POOL_MAX=2`, Stripe red herring. [`example/demo/README.md`](../example/demo/README.md).

**Do not** prescribe `repl` / `FINAL` in the user prompt.

| Look for | Why |
|----------|-----|
| `[rlm_escalate]` or `repl` | Work left the chat transcript |
| `/tree` + `[D]` | Visible Deep Context |
| Answer cites `DB_POOL_MAX=2` / 14:31 deploy / 503s | Real understanding |
| Stripe called out as red herring | Didn’t fall for the decoy |

---

## B. Overnight clip (30s)

Same checkout, YOLO:

```bash
export BIN=./target/release/rs-agent
$BIN -a wish "triage the outage: root cause vs red herring" --auto
$BIN -a fleet up --seats Fleet-1 --budget-minutes 20
# second terminal:
$BIN fleet status
# TUI in the project: c  → select Fleet-1 → f follow
```

Show: wish became a bead, worker heartbeat, then stop with `fleet down`.
Do not wait for a perfect close in the recording — the point is the factory
loop, not finishing the bead on camera.

Two seats in one worktree can collide ([`trust.md`](trust.md)). For a demo,
one seat is enough.

---

## C. Everyday coding (optional third beat)

In a real repo: “fix this failing test” — `edit` / `bash` / diff. Proves the
daily on-ramp still exists.

---

## Contrast (say it; don’t need a live competitor)

| Typical agent | rs-agent |
|---------------|----------|
| Everyday edit/bash | Same |
| Fat log → stuff into context | Fat log → Deep Context / `repl` |
| Session dies when you quit | Fleet keeps claiming beads |

---

## Success criteria

- [ ] Natural Deep Context prompt (no “use repl only”)
- [ ] Log large enough to matter (~100KB+)
- [ ] Answer hits pool-size deploy root cause
- [ ] Recording shows `[D]` and `/tree`
- [ ] Wish → fleet status visible
- [ ] Install or `cargo run --release` story is honest (no 404 curl)

---

## Failure modes

| Symptom | Fix |
|---------|-----|
| Model `read`s entire log into chat | Escalate should fire; raise file size or re-ask once |
| Misses root cause | Stronger model; or regenerate log (`--prepare-only`) |
| Permission prompts | Run with `-a` |
| `python3` missing | Install Python 3 |
| Fleet looks idle with open beads | Beads not **ready** (deps/gated) — `/beads ready` |

---

## Internal harness (not the public face)

`./scripts/demo-rlm.sh` is the **scripted** RLM smoke (secret token / forced
`repl`). CI only. Do **not** lead distribution with it.

---

## What to publish

1. 90s screen recording (Deep Context TUI + 30s fleet)
2. Repo link; **build from source** until a `v*` release exists
3. Line: *You close the laptop. Work continues. Big context never blows the window.*
4. Link [`docs/trust.md`](trust.md)
