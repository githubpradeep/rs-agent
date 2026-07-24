# Examples

Small fixtures for trying rs-agent features.

| Path | What it’s for |
|------|----------------|
| [`rlm_long_doc.md`](rlm_long_doc.md) | Seed long-doc fixture (secret token `RLM-TREE-42`) |
| `rlm_demo_corpus.md` | Generated locally (~100KB) via `./scripts/demo-rlm.sh --prepare-only` — gitignored |
| [`rlm_mapreduce.md`](rlm_mapreduce.md) | Sketch of a batched `llm_query_batched` map-reduce over chunks |
| [`snake.py`](snake.py) | Coding-agent playground (edit / run with tools) |

## Quick RLM try

```sh
# Full USP demo (grows corpus + one-shot -p with -a). See docs/demo.md.
./scripts/demo-rlm.sh --provider anthropic

# Minimal seed-only prompt:
cargo run --release -- -a --provider anthropic -p \
  "Use the repl tool: load_file('example/rlm_long_doc.md') into context, peek the first 500 chars, find the secret token by searching context, then FINAL the token + a short summary."
```

Requires `python3` on `PATH` and a valid provider API key. Talk track + recording notes: [`docs/demo.md`](../docs/demo.md).
