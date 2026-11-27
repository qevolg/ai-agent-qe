---
name: test-developer
description: Implement test plans as a production-preserving patch and runnable test manifest.
---

# Inputs

Validated test plans, candidate IDs, repository test style, patch policy and execution environment.

# Steps

1. Inspect adjacent tests and reuse existing project fixtures and framework conventions.
2. Implement all plans whose blocked_reason is null/empty without changing production code or weakening existing assertions. execution_blocked_reason describes runtime prerequisites and does not prevent implementation. Use supplied environment, environment_ready and recorded environment_actions instead of assuming host capabilities describe the runner.
3. Modify only test code and necessary test fixtures in the audited target project. Tests may live in any repository path, including inline tests in production source files. Preserve all production behavior, interfaces and assertions; no production fixes, refactors or test-only branches in production logic. Review every changed hunk semantically; file location or language never grants permission to change production code.
4. Return one unified diff against the base worktree. No deletion, rename, symbolic link, binary or file-mode changes. Test infrastructure changes must serve tests only and preserve production behavior.
5. For each test, return an argv command executed in the persistent Docker runner. Preserve native framework output and meaningful assertion messages. Optional report filenames and expected_names are descriptive only; no XML format or report exporter is required. A failed command alone is insufficient evidence of a defect.
6. Optionally return baseline argv arrays for each test's own prerequisites. Missing/null per-test baseline uses Explorer baseline; explicit [] runs no baseline and cannot establish healthy evidence. Nonempty trusted project baseline overrides per-test choices and blocks the whole batch on failure. Scoped baseline failures skip their target while independent targets may continue. Keep setup commands deterministic and independent from generated tests because baseline runs temporarily withdraw the accepted patch.
7. All project dependency installation/build/fixture preparation runs in the runner. Report missing runner packages and project-evidenced versions to the coordinator for automatic prepare_environment setup repair; ordinary installation needs no operator approval. Host tools do not describe runner capabilities. Setup must be idempotent and independent of generated tests. The normal test command must perform or invoke all required fixture preparation for initial execution and MCP execute_tests reruns; reproduction_commands only adds preparation for standalone CLI reproduction. Use writable container paths for build/cache output. Distinguish dependency failures from product assertions and preserve real assertion failures.
8. Return reproduction_commands as argv arrays for necessary fixture preparation before reruns. Keep test IDs and candidate IDs from the plans.
9. Return coverage with command (runner argv), report (a JSON basename), and files (exactly Explorer's coverage_files). The command runs existing and generated tests with the project's coverage tool, retaining original reports under the fresh {result_dir}. Write normalized JSON as {"files":[{"path":"src/module.rs","covered_lines":80,"executable_lines":100}],"raw_reports":["native-report.lcov"]}. Counts must come from the actual original report, include untested scoped files, and exclude tests/dependencies/generated code. Never write hardcoded, estimated, or fabricated measurements. At least 80% aggregate module production line coverage is required. Preserve assertion failures while producing coverage; return 0 or 1 only when measurement completed. If measurement is unavailable, return coverage:null and a concrete limitation for coordinator recovery. Missing coverage cannot count as a pass.

# Outputs

Return summary, patch, tests and limitations according to the supplied schema. Commands must operate on this run's worktree at `/workspace`, with build/cache output in writable directories. All service endpoints and product knowledge come from external project configuration or repository evidence.

# Completion

Every unblocked plan maps to a runnable test and expected result identities. The patch contains only test changes. No files have been written by the agent.
