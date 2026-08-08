# Demo kit — everyday agent + Deep Context

Goal: in ~90 seconds, show a **real job** (incident in a fat log), not a CTF.
Big context stays outside the chat; the agent peeks/slices; you show `[D]` + `/tree`.

Use a **reliable paid model** (Anthropic Sonnet / similar). Tiny/free models often
break tool calls and kill the vibe.

---

## 60-second talk track

1. **Open:** “rs-agent is an everyday coding agent — edit, bash, search, sessions.”
2. **Setup:** “Here’s a real-ish prod log from a checkout outage — about 180KB.”
3. **Ask (natural):** “When did it start, what’s the root cause, what’s a red herring?”
4. **Show:** `[rlm_escalate]` / `repl`, status `[D]`, then `/tree` — chat didn’t eat the file.
5. **Close:** “Same agent for daily work. Deep Context kicks in when the window isn’t enough.”

---

## Prep (once)

```bash
cd /path/to/rs-agent
cargo build --release
export ANTHROPIC_API_KEY=sk-ant-...
python3 --version
```

Optional: `approve = true` in `~/.rs-agent/config.toml` so permissions don’t interrupt.

---

## A. Public demo (use this for recordings / HN)

```bash
# One-shot (good for asciinema / CI-ish check of the story)
./scripts/demo-deep-context.sh --provider anthropic

# Interactive TUI (best for video — shows [D] and /tree)
./scripts/demo-deep-context.sh --tui --provider anthropic
```

What the corpus is: `example/demo/outage.log` — generated checkout-api logs with a
buried bad deploy (`DB_POOL_MAX=2`) and a Stripe red herring. Details:
[`example/demo/README.md`](../example/demo/README.md).

**Do not** prescribe `repl` / `FINAL` in the user prompt. Let escalate + the agent work.

| Look for | Why |
|----------|-----|
| `[rlm_escalate]` or `repl` | Work left the chat transcript |
| `/tree` + `[D]` | Visible Deep Context |
| Answer cites `DB_POOL_MAX=2` / 14:31 deploy / 503s | Real understanding |
| Stripe called out as red herring | Didn’t fall for the decoy |

---

## B. Everyday coding clip (30s, optional second beat)

In a real repo: “fix this failing test” or “add a log line” — `edit` / `bash` / diff.
Proves you’re not only a long-context toy.

---

## C. Contrast (say it; don’t need a live competitor)

| Typical agent | rs-agent |
|---------------|----------|
| Everyday edit/bash | Same |
| Fat log → stuff into context | Fat log → Deep Context / `repl` |
| Truncate or blow tokens | Peek, search, summarize in tree |

---

## Success criteria

- [ ] Natural prompt (no “use repl only” coaching)
- [ ] Log is large enough to matter (~100KB+)
- [ ] Answer hits pool-size deploy root cause
- [ ] TUI recording shows `[D]` and `/tree`
- [ ] Install / `cargo run --release` works on a clean machine story

---

## Failure modes

| Symptom | Fix |
|---------|-----|
| Model `read`s entire log into chat | Escalate should fire; raise file size or re-ask once |
| Misses root cause | Stronger model; or check log generated (`--prepare-only`) |
| Permission prompts | Run with `-a` |
| `python3` missing | Install Python 3 |

---

## Internal harness (not the public face)

`./scripts/demo-rlm.sh` remains the **scripted** RLM smoke (secret token / forced
`repl`). Use it for CI. Do **not** lead distribution with it.

---

## What to publish

1. 60–90s screen recording of the TUI path (`--tui`)
2. Repo link + install one-liner
3. Line: *Everyday coding agent — better when context doesn’t fit (Deep Context)*
