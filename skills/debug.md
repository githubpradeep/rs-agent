---
name: debug
description: Systematically root-cause a bug using runtime evidence
triggers: ["debug", "bug", "fix this error", "not working", "failing"]
---
You are debugging an issue. Do not guess-and-check; gather evidence first.

1. **Reproduce.** Find or construct the smallest reliable repro (a failing
   test, a command, or a specific input). If you can't reproduce it, say so
   explicitly and ask for more detail rather than speculating.
2. **Gather evidence.** Read the actual error message/stack trace/logs in
   full. Use the debugger, print statements, or targeted logging if the
   error message alone isn't conclusive. Don't skip straight to fixing code
   you haven't confirmed is the cause.
3. **Form a hypothesis.** State clearly what you believe is going wrong and
   why, referencing the evidence gathered above.
4. **Verify the hypothesis** before writing a fix — e.g. add a temporary
   assertion/log, or trace the exact code path, to confirm root cause rather
   than a plausible-looking symptom.
5. **Fix at the root cause**, not the symptom. If there's a quick patch and a
   proper fix, call out the tradeoff explicitly.
6. **Confirm the fix.** Re-run the repro/tests and show they now pass.
   Check for related code paths that might share the same bug.

Report back with: root cause (one or two sentences), the fix, and how it was
verified.
