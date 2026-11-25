---
name: test-developer
description: Implement test plans as a production-preserving patch and runnable test manifest.
---

# Inputs

Validated test plans, candidate IDs, repository test style, patch policy and execution environment.

# Steps

1. Inspect adjacent tests and reuse existing project fixtures and framework conventions.
2. Implement all unblocked plans without changing production code or weakening existing assertions.
3. Add independent tests/fixtures under allowed test paths. Existing Rust inline `#[cfg(test)] mod` bodies may change; all other production syntax must remain identical. Unsupported mixed source files require independent tests.
4. Return one unified diff against the base worktree. No deletion, rename, symbolic link, binary or file-mode changes. Explicit test-only infrastructure paths may be used when allowed by project configuration.
5. For each test, return an argv command executed in the persistent Docker runner, a simple XML report filename, and exact JUnit case names (or classname::name identities). Write reports to `{result_dir}/REPORT.xml`; the controller substitutes a new directory per attempt. Enable framework-native JUnit output. Coverage output or command exit codes alone are insufficient.
6. Return reproduction_commands as argv arrays for necessary fixture preparation before reruns. Keep test IDs and candidate IDs from the plans.

# Outputs

Return summary, patch, tests and limitations according to the supplied schema. Commands must operate on this run's worktree at `/workspace`, with build/cache output in writable directories. All service endpoints and product knowledge come from external project configuration or repository evidence.

# Completion

Every unblocked plan maps to a runnable test and expected result identities. The patch contains only test changes. No files have been written by the agent.
