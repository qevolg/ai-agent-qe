---
name: test-designer
description: Design discriminating tests and explicit validation conditions for defect candidates.
---

# Inputs

Candidate list, analysis, behavioral references, existing fixtures and available environment.

# Steps

1. Give every candidate at least one plan with a unique test_id and the original candidate_id.
2. Define a behavioral oracle independent of the suspicious implementation. Cite its source in oracle.
3. Specify initial state, real service requirements, actions, observable outcome and a target assertion.
4. Explain how the test distinguishes the proposed defect from setup, build, timeout and cleanup failures.
5. Specify reproduction and data reset/preparation steps. Preserve a failure scene where needed.
6. If a reliable oracle or required setup is missing, set blocked_reason and describe the gap. Do not change intended behavior to force a test.

# Outputs

Return the test-designer schema with expectation, oracle, steps, reproduction and blocked_reason. Use null for an unblocked plan.

# Completion

Every candidate is represented. Every unblocked plan has an observable distinguishing assertion and reproducible prerequisites.
