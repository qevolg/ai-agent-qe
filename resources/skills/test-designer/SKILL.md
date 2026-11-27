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
6. Set blocked_reason only when a reliable oracle or implementable design is missing. An unprepared runner, a missing runner dependency or an unavailable service belongs in optional execution_blocked_reason when the test can still be implemented. Use harness-provided environment, environment_setup, environment_ready and recorded environment_actions. Host build/test tools are not execution prerequisites.
7. Missing host build/test tools are irrelevant: compilation, dependency installation, fixtures and tests execute in Docker. For missing runner dependencies, specify required packages/versions and readiness checks so the coordinator can automatically install them with prepare_environment setup, without asking the operator to install tools. Until runner evidence shows a real unresolved prerequisite, do not claim execution is blocked merely because a host tool is absent.
8. Split independent unit, integration and E2E experiments into separate test IDs so a runtime prerequisite does not block unrelated tests. Describe concrete prerequisites and how readiness can be checked. Do not change intended behavior to force a test.

# Outputs

Plan enough meaningful tests to reach at least 80% production line coverage across Explorer's coverage_files, counting existing and new tests together. Coverage is an additional delivery requirement; it does not replace a behavioral oracle or require a particular language, framework, or report exporter.

Return the test-designer schema with expectation, oracle, steps, reproduction and blocked_reason, plus execution_blocked_reason when needed. Use null for an unblocked design; an execution blocker does not prevent Developer.

# Completion

Every candidate is represented. Every unblocked plan has an observable distinguishing assertion and reproducible prerequisites.
