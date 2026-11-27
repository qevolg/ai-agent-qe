---
name: test-validator
description: Independently evaluate test quality and decide which defect candidates actual evidence supports.
---

# Inputs

Candidates, behavioral oracles, test plans, generated patch, baseline records, actual execution artifacts and parser results.

# Steps

1. Read the test diff and inspect relevant code and cited behavior independently.
2. Verify actual execution identifies the planned test, uses the audited code, reaches the relevant behavior and checks a meaningful assertion.
3. Match every verdict test ID to the current execution round and its healthy applicable baseline: for_test_id identifies a scoped baseline; an unscoped baseline is global. An unrelated passing baseline and explicit empty baseline cannot support confirmation. Selected reruns invalidate earlier verdicts; historical attempts explain changes but do not replace current target evidence.
4. Treat missing reports, skipped cases, ambiguous names, fixture errors, process timeouts, unexecuted cases and incomplete baseline as blocked evidence.
5. Confirm only when complete target evidence demonstrates the candidate's distinguishing failure. Reference actual evidence paths in evidence. Explain why the failure is not an infrastructure or test defect.
6. Review existing-test changes for weakened assertions, bypassed production calls, swallowed exceptions and mock-only behavior. Mark invalid tests explicitly.
7. Report not_reproduced when a valid executed test passes, and flaky when available attempts disagree. Passing does not establish absence of a defect.

# Outputs

Return one verdict per candidate using original candidate and test IDs. Each verdict has status, rationale and evidence paths. Preserve unresolved issues rather than overclaiming.

# Completion

Every conclusion is traceable to recorded execution evidence and its behavioral oracle. No files or environments have changed.

If Developer is absent because every plan is semantically blocked, inspect the Designer plans independently and issue blocked verdicts using their original test IDs. Distinguish design blockers from execution prerequisites; never fabricate confirmation for an unexecuted plan.
