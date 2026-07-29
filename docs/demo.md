# Demo kit — everyday agent + RLM edge

Goal: in ~90 seconds, show that rs-agent is a **daily coding agent** that also wins when
context is huge — big text stays in a Python REPL; the agent slices and recurses; the parent
only sees a summary / `FINAL` result.

Use a **reliable paid model** for demos (Anthropic Sonnet / Bedrock Opus or similar). Free /
tiny models often dump broken tool XML and kill the vibe.

---

## 60-second talk track

1. **Open:** “rs-agent is my everyday coding agent — edit, bash, search, sessions — same job as
   other terminal agents.”
2. **Twist:** “Most agents stuff big docs into the context window. Past a few hundred KB they
   truncate, hallucinate, or burn tokens.”
3. **Idea:** “RLM — recursive language models. Context lives *outside* the model in a REPL.
   The agent peeks with Python and calls `llm_query` on slices. Parents only see summaries.
   That’s why it’s better for hard context without giving up daily coding.”
4. **Show:** run the script (or TUI). Point at: `repl` → load/slice → `llm_query` → `FINAL` →
   answer contains `RLM-TREE-42`. Hit `/tree` if in TUI.
5. **Close:** “Same agent for everyday work. RLM kicks in when the window isn’t enough —
   and the harness auto-escalates on huge reads (`[rlm_escalate]` → use `repl`).”

---

## Prep (once)

```bash
cd /path/to/rs-agent
cargo build --release
export ANTHROPIC_API_KEY=sk-ant-...   # or whatever provider you demo
# python3 must be on PATH
python3 --version
```

Optional: put `approve = true` in `~/.rs-agent/config.toml` so the REPL isn’t
blocked by permission prompts during a live talk.

---

## A. One-shot demo (best for recording / CI vibe)

From the repo root:

```bash
./scripts/demo-rlm.sh
# or:
./scripts/demo-rlm.sh --provider anthropic --model claude-sonnet-4-20250514
```

What it does:

1. Builds a ~100KB synthetic doc from `example/rlm_long_doc.md` (keeps the
   secret token `RLM-TREE-42` once).
2. Runs `rs-agent -a -p '…'` with a prompt that **requires** `repl` + slice +
   `llm_query` / `FINAL` (not “read the whole file into chat”).
3. Prints pass/fail hints (look for the token in the answer).

Record the terminal with [asciinema](https://asciinema.org/) or a screen
capture. Crop to ~60–90s; overlay the talk-track lines if you post to X/HN.

---

## B. Interactive TUI demo (best live)

```bash
cargo run --release -- -a --provider anthropic
# status bar should show your model; type i then paste:

Use the repl tool only — do NOT paste the full file into chat.
1) load_file('example/rlm_demo_corpus.md') into context
2) print len(context) and context[:200]
3) find the secret token in Section C by slicing/searching in Python
4) llm_query a one-paragraph summary of rivers+mountains (not the whole doc)
5) FINAL({"token": "...", "summary": "..."})
Then show /tree
```

Generate the corpus first if missing:

```bash
./scripts/demo-rlm.sh --prepare-only
```

During the run:

| Key / command | Show |
|---------------|------|
| Tool spinner on `repl` | Work is outside the chat transcript |
| `/tree` | Call tree (parent → llm_query children) |
| Final answer | Contains `RLM-TREE-42` |

---

## C. Contrast slide (optional, strong)

Same corpus, two narratives — this is why everyday rs-agent is *better*, not niche:

| Typical agent | rs-agent |
|---------------|----------|
| Everyday edit/bash/search | Same (first-class) |
| Big file → stuff into context | Big file → `repl` + `context` |
| Hits limits / truncates | Peeks `context[:200]`, searches in Python |
| One flat transcript | Tree of `llm_query` / `agent_query` when needed |

You do **not** need to run a competitor live — saying the contrast is enough
if time is short.

---

## Success criteria

- [ ] Demo uses `-a` (or trusted project) so permissions don’t interrupt
- [ ] Model actually calls `repl` (not only `read` of the whole file)
- [ ] Answer includes secret token **`RLM-TREE-42`**
- [ ] You can point at `/tree` or tool blocks showing sub-calls
- [ ] Install one-liner or `cargo run --release` works on a clean machine story

---

## Failure modes (have a backup)

| Symptom | Fix |
|---------|-----|
| Model dumps `<tool_call>` as text | Switch off free/tiny models; use Anthropic/OpenAI |
| `python3` missing | Install Python 3; restart |
| Permission prompt freezes the room | Restart with `-a` |
| Agent `read`s the whole file | Harness should emit `[rlm_escalate]`; if not, re-prompt “repl only” |
| Empty /tree | Run `/tree` after the turn; or show repl tool blocks |

Backup clip: pre-record `./scripts/demo-rlm.sh` so a live fail isn’t fatal.

---

## What to publish with the demo

1. Short video / asciinema + link to repo
2. One command from README (`install.sh` or `cargo run`)
3. This line: *Everyday coding agent — better when context doesn’t fit (RLM)*
4. Paper: https://arxiv.org/abs/2512.24601


---

## Auto-escalate (Wave C)

When a `read` (or truncated tool dump) exceeds ~10K chars (configurable:
`rlm_escalate_chars` / `--rlm-escalate-chars`), the harness returns a short preview
plus an `[rlm_escalate]` footer that tells the model to use `repl` + `load_file`
instead of stuffing chat. The next model turn also gets a one-shot system note.

Demo this by asking the agent to open a large file without mentioning RLM — you should
see the escalate marker, then a `repl` turn.
