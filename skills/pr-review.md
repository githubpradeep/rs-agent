---
name: pr-review
description: Review a pull request or diff for bugs, regressions, and style issues
triggers: ["pr", "pull request", "review", "code review"]
---
You are reviewing a pull request or diff. Work through it systematically:

1. **Understand the intent.** Read the PR description/commit messages first, then
   the diff. Summarize in one sentence what the change is trying to accomplish.
2. **Correctness.** Look for logic errors, off-by-one mistakes, incorrect
   error handling, race conditions, and edge cases (empty input, nil/None,
   unicode, large inputs, concurrent access).
3. **Regressions.** Check whether the change could break existing callers,
   tests, or public APIs. Look for behavior changes not mentioned in the
   description.
4. **Style & consistency.** Flag deviations from the surrounding code's
   conventions (naming, error handling patterns, module layout). Don't
   nitpick pure taste unless it hides a real ambiguity.
5. **Tests.** Confirm new behavior has test coverage, and that changed
   behavior has updated tests. Note obviously missing edge case tests.
6. **Security & safety.** Watch for injection, unsafe deserialization,
   unchecked user input, secrets in code, and unsafe blocks without
   justification.

Output format:
- A short summary of what the change does.
- A bulleted list of concrete issues, each with file:line references and a
  suggested fix. Mark severity as `blocking`, `should-fix`, or `nit`.
- End with an explicit verdict: `Approve`, `Approve with nits`, or `Request changes`.

Be direct and specific — cite exact lines rather than vague concerns.
