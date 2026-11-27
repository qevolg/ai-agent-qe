# State and Evidence Implementation Plan

> **For agentic workers:** Use executing-plans to implement task by task.

**Goal:** Make JSON state consistent and post-audit operations recoverable while bounding model/log payloads.

**Architecture:** Versioned authoritative snapshots with compatibility exports, per-run advisory locking, journaled reproduction attempts, role-specific inputs and streamed evidence logs.

**Tech Stack:** Existing Rust, serde, JSON and libc flock; no new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-27-state-and-evidence-design.md`

## Global Constraints

- Preserve legacy Python runs and unversioned Rust snapshots.
- Keep complete raw logs, original audit evidence, JUnit and patch recovery contracts.
- Do not add SQLite or change stage selection ownership.
- Finish tests, review, commit and push to the existing remote as requested.

## Review Focus

- A corrupt compatibility projection must not override a versioned snapshot.
- Lock contention must reject an operation before environment side effects.
- Failed preparation must retain partial reproduction records and original verdicts.
- Large/invalid UTF-8 output must remain complete on disk and bounded in memory.
- Missing reports must not block Docker cleanup or replace original audit verdicts.

## Task 1: State and reproduction

**Files:** `src/state.rs`, `src/state/snapshot.rs`, `src/harness.rs`, `src/mcp.rs`, `src/workflow.rs`, `src/cli.rs`, `tests/state_flow.rs`, `tests/workflow.rs`.

**Interfaces:** `HarnessSnapshot::{load,save}`, `RunLock::acquire`, `Store::{load,save}`; existing reproduction signature retained.

- [x] Add regression tests for snapshot precedence, old post-audit metadata migration, invalid snapshots, operation contention, active/failed reproduction.
- [x] Run focused tests and observe the expected failures.
- [x] Implement snapshot validation and shared state writes, operation locks and incremental reproduction records.
- [x] Run focused state/workflow/MCP/agent-loop tests; commit the deliverable.

## Task 2: Bounded context and logs

**Files:** `src/harness/inputs.rs`, `src/harness.rs`, `src/mcp.rs`, `src/process.rs`, `src/environment.rs`, `src/resources/agents/coordinator.md`, `tests/agent_loop.rs`, `tests/core.rs`, `tests/environment_runtime.rs`.

**Interfaces:** `build_stage_inputs`, bounded model-facing evidence projection, `run_streamed`; complete machine-readable capture remains available.

- [x] Add regressions proving role-specific inputs retain retry feedback and that long outputs keep complete raw files but bounded summaries.
- [x] Run regressions and observe expected failures.
- [x] Implement input projection and stream/capture separation; route setup/test/log commands through streamed execution.
- [x] Run focused process/environment/agent-loop/MCP tests; commit.

## Task 3: Derived reports, docs and integration

**Files:** `src/reporting.rs`, `src/cli.rs`, `README.md`, `docs/design.md`, `docs/agent-loop-design.md`, `tests/reporting.rs`, `tests/workflow.rs`.

**Interfaces:** `refresh_report` uses snapshots for current runs and previous report for legacy runs; cleanup persists independently of rendering.

- [x] Add missing/corrupt report and cleanup regressions; observe failures.
- [x] Implement regeneration and independent cleanup/report outcomes; align documentation.
- [x] Run formatting, clippy and the complete locked test suite; run real Docker coverage if available.
- [x] Review the whole diff with a fresh reviewer, resolve material findings and complete final validation.

Delivery: commit and push the validated changes to `origin/main`; report the verified remote SHA in the final delivery message.
