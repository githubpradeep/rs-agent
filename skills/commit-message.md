---
name: commit-message
description: Write a clear, conventional git commit message for the staged changes
triggers: ["commit message", "write a commit", "commit this"]
---
You are writing a git commit message for the currently staged (or described)
changes.

1. **Look at the actual diff**, not just file names, to understand what
   changed and why — don't guess from context alone.
2. **Summary line**: imperative mood, no trailing period, ideally under 72
   characters (e.g. "Add retry logic to HTTP client", not "Added..." or
   "Adds..."). Focus on *why* the change was made when it's not obvious from
   *what* changed.
3. **Body (if needed)**: wrap at ~72 columns, explain motivation and
   context, call out any non-obvious tradeoffs or follow-up work. Skip the
   body entirely for small, self-explanatory changes.
4. **Don't restate the diff line-by-line** — a reviewer can read the diff;
   the message should add context the diff can't convey.
5. **Reference issues/tickets** if the surrounding context mentions them.
6. **Never include** secrets, credentials, or generated noise (lockfile
   churn, build artifacts) in the description.

Output just the commit message (summary line, blank line, optional body) —
no surrounding commentary unless asked.
