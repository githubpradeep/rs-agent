# Deep Context product demo

Realistic checkout-api logs with a buried outage. Used by
`./scripts/demo-deep-context.sh` (public demo) — not the CTF-style RLM harness.

## Generate

```bash
./scripts/demo-deep-context.sh --prepare-only
# writes example/demo/outage.log (~180KB, gitignored)
```

## Ground truth (spoiler)

| Time (UTC) | What happened |
|------------|----------------|
| 13:44 | Stripe latency blip — **red herring**, recovered |
| 14:31:08 | Deploy `checkout-api@2.14.0` sets **`DB_POOL_MAX=2`** (bad) |
| 14:32:41 | `db_pool_wait` warnings start |
| 14:33:15 | Cascade of **503** `db_pool_timeout` on checkout |

Root cause: post-deploy DB pool starvation, not payments.

## Prompt to paste (TUI)

```
We had elevated checkout errors on 2026-03-15. Look at example/demo/outage.log
and tell me: (1) when the incident started, (2) the root cause, (3) what was a
red herring. Be specific — cite timestamps and config values from the log.
```

Do **not** tell the agent to use `repl` — Deep Context should escalate on its own.
