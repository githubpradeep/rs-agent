---
name: refactor
description: Safely restructure code without changing external behavior
triggers: ["refactor", "clean up", "restructure", "simplify"]
---
You are refactoring code. The goal is to improve structure/readability/
maintainability while preserving observable behavior exactly.

1. **Establish a safety net first.** Identify (or add) tests that cover the
   current behavior of the code you're about to change. If there's no test
   coverage, note the risk explicitly before proceeding.
2. **Make the change in small, reviewable steps.** Prefer a sequence of
   mechanical, easy-to-verify transformations (extract function, rename,
   inline, move) over one large rewrite.
3. **Don't mix refactoring with behavior changes.** If you spot a bug or a
   feature gap while refactoring, note it separately rather than silently
   fixing it inline — call it out to the user instead.
4. **Preserve the public API** unless the user explicitly asked for an API
   change. If call sites need updates, update all of them.
5. **Re-run tests after each meaningful step**, not just at the end, so a
   regression is easy to isolate.
6. **Clean up as you go**: remove now-dead code, stale comments, and unused
   imports that the refactor made obsolete.

Summarize what structurally changed and confirm tests still pass at the end.
