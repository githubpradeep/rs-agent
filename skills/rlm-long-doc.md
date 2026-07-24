---
name: rlm-long-doc
description: Work through a document too large to fit in context using the REPL
triggers: ["long document", "large file", "rlm", "load_file", "big doc"]
---
You are working with a document too large to read directly into context.
Use the REPL/`load_file` tool-driven workflow instead of reading the whole
file at once:

1. **Load, don't read.** Use the REPL's `load_file` (or equivalent) to bring
   the document into a queryable session rather than dumping its full
   contents into the conversation.
2. **Map before you dive.** First get structure: headings, section
   boundaries, line/byte counts, table of contents. This tells you where to
   look instead of scanning linearly.
3. **Query narrowly.** Pull specific sections, ranges, or grep-style matches
   relevant to the task rather than paging through the whole document.
4. **Iterate incrementally.** Treat this like an investigation: form a
   question, query for the answer, refine the next query based on what you
   found. Avoid repeatedly re-loading the same large ranges.
5. **Summarize as you go.** Keep a running summary of what you've learned so
   you don't need to re-query the same regions later in the task.
6. **Cite locations, not full text.** When reporting findings, reference
   section/line locations rather than re-pasting large excerpts back into
   the conversation.

Only fall back to loading large raw spans verbatim if a targeted query
genuinely isn't possible.
