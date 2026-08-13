# rs-agent assessment rubric

Score **shipped behavior**. Docs and PLAN checkboxes are evidence of intent only.

## Headline scores (0–10)

| Score | Engineering | Product | Funding |
|---|---|---|---|
| 8–10 | Loop/recovery/tests you would leave running | Strangers install and return | Users + evals + a wedge incumbents lack |
| 5–7 | Serious core, thin edges (worker/TUI tests, sandbox) | Works for the author; install/demo gaps | Interesting IP, no traction |
| 3–4 | Features exist; several are facades | Two products, vocabulary tax, no release | Deck would be a repo tour |
| 0–2 | Demo-quality loop | Cannot curl-install; 0 outsiders | Do not raise |

**Public traction** is a raw count (stars, tags, dogfooders), not a /10.

## Capability grades

Use exactly: **Have** · **Thin** · **Fake** · **Fail**

| Layer | Have | Thin | Fake | Fail |
|---|---|---|---|---|
| Interactive loop + tools | Retries, compact, abort/steer, repair, permissions | Tools exist, recovery flaky | — | Loop dies on stream drop |
| Overnight claim loop | Leases, heartbeat, recover, honest idle | Worker runs, no live observability | Status file, no processes | Cannot leave it overnight |
| Deep Context | REPL + tree + auto-escalate, parent sees summaries | Sidecar only; models forget `repl` | Rename-only “RLM” | Broken / no python3 story |
| Beads / fleet / marshal | Graph + spawn + assign + caste routing | JSON factory, no worktree isolation | `/fleet` reprints marshal | Missing |
| Standing roles | Cron watcher that unsticks/gates from evidence | One function, hardcoded `cargo test` | Prompt file, no runner | Missing |
| Distribution | Tagged release install.sh works | Source-only, docs say curl | install.sh 404s | No README path to run |
| Audience | Outsiders dogfooding | Author-only but public | — | 0 stars/issues, stale public README |

## Reliability P0/P1/P2

Always re-check these; add new ones if the tree grew:

**P0** — blocks product or a pitch

- Unsandboxed REPL (`exec` + full builtins)
- Overnight requires YOLO / no policy
- Fleet shares one worktree by default
- Install one-liner cannot work (no tag/artifacts)
- No screenshot or 90s demo of `/tree` + overnight

**P1** — blocks trust

- Worker / TUI untested
- Live evals optional or `continue-on-error`
- Deep Context still sidecar vs incumbents’ native RLM
- Vocabulary tax (city offices in default UI)
- Two-product README

**P2** — later

- Windows / Intel Mac
- MCP OAuth / remote
- JSON vs real DB
- SDK, desktop, marketplace

## Pitch rules

Lead with overnight factory + Deep Context **together**.

Do not lead with: city job titles, “Recursive Language Models” as slide 1,
1000-model catalog, desktop, marketplace, Cursor-killer.

## Funding paths (pick one as current)

1. **Open source first** — most honest until ~1k stars and a weekly night demo
2. **Lab / acquihire** — only if a published eval beats compaction-only agents
3. **B2B factory** — only after sandbox, worktrees, audit; not a 2026 slide 1
4. **Do not raise** — default

Treat IP as craft/portfolio until install + 3 design partners + one eval chart.
