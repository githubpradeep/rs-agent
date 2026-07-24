# Examples

Small fixtures for trying rs-agent features.

| Path | What it’s for |
|------|----------------|
| [`rlm_long_doc.md`](rlm_long_doc.md) | Large-ish document — load into REPL `context` / `load_file` and summarize via `llm_query` / `agent_query` |
| [`rlm_mapreduce.md`](rlm_mapreduce.md) | Sketch of a batched `llm_query_batched` map-reduce over chunks |
| [`snake_game.py`](snake_game.py) / [`snake.py`](snake.py) | Coding-agent playground (edit / run with tools) |

## Quick RLM try

```sh
cargo run --release -- --provider anthropic -p \
  "Use the repl tool: load_file('example/rlm_long_doc.md') into context, peek the first 500 chars, then llm_query a one-paragraph summary. Finish with FINAL(summary)."
```

Requires `python3` on `PATH` and a valid Anthropic API key.
