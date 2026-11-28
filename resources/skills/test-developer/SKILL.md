---
name: test-developer
description: Implement test plans as a production-preserving patch and runnable test manifest.
---

# Inputs

Validated test plans, candidate IDs, repository test style, patch policy and execution environment.

# Steps

1. Inspect adjacent tests and reuse existing project fixtures and framework conventions.
   Reuse only fixtures that seed real data or configure real implementations. Never reuse mocks, stubs, spies, fake clients/servers/transports or recorded service responses. No SDK, network, IPC or database behavior may be replaced, even if the assertion also runs production code. These rules include baselines and coverage runs.
2. Implement all plans whose blocked_reason is null/empty without changing production code or weakening existing assertions. execution_blocked_reason describes runtime prerequisites and does not prevent implementation. Use supplied environment, environment_ready and recorded environment_actions instead of assuming host capabilities describe the runner.
3. Add independent tests/fixtures under allowed test paths. Existing Rust inline `#[cfg(test)] mod` bodies may change; all other production syntax must remain identical. Unsupported mixed source files require independent tests.
4. Return one unified diff against the base worktree. No deletion, rename, symbolic link, binary or file-mode changes. Explicit test-only infrastructure paths may be used when allowed by project configuration.
5. For each test, return an argv command executed in the persistent Docker runner. Preserve native framework output and meaningful assertion messages. Optional report filenames and expected_names are descriptive only; no XML format or report exporter is required. A failed command alone is insufficient evidence of a defect.
6. Optionally return baseline argv arrays for each test's own prerequisites. Missing/null per-test baseline uses Explorer baseline; explicit [] runs no baseline and cannot establish healthy evidence. Nonempty trusted project baseline overrides per-test choices and blocks the whole batch on failure. Scoped baseline failures skip their target while independent targets may continue. Keep setup commands deterministic and independent from generated tests because baseline runs temporarily withdraw the accepted patch.
7. All project dependency installation/build/fixture preparation runs in the runner. Report missing runner packages and project-evidenced versions to the coordinator for automatic prepare_environment setup repair; ordinary installation needs no operator approval. Host tools do not describe runner capabilities. Setup must be idempotent and independent of generated tests. The normal test command must perform or invoke all required fixture preparation for initial execution and MCP execute_tests reruns; reproduction_commands only adds preparation for standalone CLI reproduction. Use writable container paths for build/cache output. Distinguish dependency failures from product assertions and preserve real assertion failures.
8. Return reproduction_commands as argv arrays for necessary fixture preparation before reruns. Keep test IDs and candidate IDs from the plans.
9. Return coverage with command (runner argv), report (a JSON basename), and files (exactly Explorer's coverage_files). The command runs existing and generated tests with the project's coverage tool, retaining original reports under the fresh {result_dir}. Write normalized JSON as {"files":[{"path":"src/module.rs","covered_lines":80,"executable_lines":100}],"raw_reports":["native-report.lcov"]}. Counts must come from the actual original report, include untested scoped files, and exclude tests/dependencies/generated code. Never write hardcoded, estimated, or fabricated measurements. At least 80% aggregate module production line coverage is required. Preserve assertion failures while producing coverage; return 0 or 1 only when measurement completed. If measurement is unavailable, return coverage:null and a concrete limitation for coordinator recovery. Missing coverage cannot count as a pass.
10. Return service_checks for every test, exactly one per planned real_services entry. Each entry has service (the owned Docker Compose service name, never the runner) and command (runner argv performing a real protocol operation and reporting/verifying server version). The harness checks container/image ownership and runs probes before each target and reproduction. A Python/Java version check, an open TCP port or a no-op is not a real service protocol check. Use the same service endpoints in the actual test, assert real results, and verify final persistence when the claim concerns migration. Missing service infrastructure blocks execution; never simulate it. Do not reuse historical mock tests in a continued audit.

# Outputs

Return summary, patch, tests and limitations according to the supplied schema. Commands must operate on this run's worktree at `/workspace`, with build/cache output in writable directories. All service endpoints and product knowledge come from external project configuration or repository evidence.

# Completion

Every unblocked plan maps to a runnable test and expected result identities. The patch contains only test changes. No files have been written by the agent.
