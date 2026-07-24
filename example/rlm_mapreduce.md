# RLM map-reduce sketch

Use this as a prompt outline when exercising `llm_query_batched` from the REPL.

## Idea

1. `load_file` / `load_dir` into `context` (or a Python list of chunks).
2. Split into N chunks (e.g. by heading or fixed size).
3. `summaries = llm_query_batched([f"Summarize:\n{c}" for c in chunks])`
4. `FINAL(llm_query("Merge these summaries:\n" + "\n---\n".join(summaries)))`

## Sample REPL session (conceptual)

```python
text = load_file("example/rlm_long_doc.md")
chunks = [text[i:i+2000] for i in range(0, len(text), 2000)]
parts = llm_query_batched([f"Key points only:\n{c}" for c in chunks[:8]])
FINAL(llm_query("Combine into 5 bullets:\n" + "\n".join(parts)))
```

Parent agent only sees the `FINAL(...)` result — not every chunk transcript.
