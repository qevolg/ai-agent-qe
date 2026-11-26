# AAQE Environment Preparation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prepare reproducible test environments independently of test generation and recover scheduling without weakening runtime evidence.

**Architecture:** Add bounded environment probes and inline Compose support to the existing runtime. Expose explicit preparation and execution operations from AuditHarness, separate implementation blockers from execution prerequisites, and report actual availability and candidate-specific evidence.

**Tech Stack:** Rust 1.88+, serde_json, existing process runner, Docker Compose v2, existing test fixtures.

**Spec:** `docs/superpowers/specs/2026-11-26-environment-preparation-design.md`

## Global Constraints

- Workers remain read-only; only the harness writes environments and applies protected test patches.
- Existing project configurations, artifacts and saved runs remain readable; new schema fields are optional.
- Generic code contains no product-specific versions, hosts, credentials or behavioral defaults.
- No complete execution and healthy applicable baseline means no confirmed product failure.
- Once resources have an ownership snapshot, the environment definition is immutable for that run.
- Worktree source remains read-only in runners; build and result output is writable outside it.

## Review Focus

- Interrupted startup must preserve the owned snapshot and support retry without orphaning resources.
- Probe authentication must not publish environment-variable secrets in saved argv or diagnostics.
- Per-test baselines must not accidentally confirm a test using another test's baseline.
- Revised upstream artifacts and repeated execution must invalidate stale verdicts.
- Legacy runs without new optional fields must preserve reproduction and cleanup behavior.

### Task 1: Environment runtime and inventory

**Files:** `src/environment.rs`, new `src/environment_probe.rs`, `src/lib.rs`, environment portions of `src/resources/schemas/{config,code-explorer}.json`, new runtime tests.

**Interfaces:** `environment_probe::inspect(workspace: &Path, directory: &Path, probe: &Value, timeout: u64) -> Result<Value>`; `environment_probe::schema() -> Value`; environment specs gain optional `compose` object and `readiness` argv arrays. Compose retries reuse the owned snapshot.

- [x] Add failing tests for generated Compose outside the worktree, no source writes, snapshot retry and tamper rejection, rejected ambiguous sources, and bounded local/remote probe validation with secret redaction.
- [x] Implement the interfaces with recorded process evidence, argv execution and fixed SSH inventory commands.
- [x] Run focused runtime/probe tests and existing orchestration tests.

### Task 2: Harness preparation, execution and scheduling

**Files:** `src/harness.rs`, `src/mcp.rs`, `src/contracts.rs`, `src/workflow.rs`, Designer/Developer schemas, harness and MCP tests/fixtures.

**Interfaces:** `AuditHarness::{inspect_environment(&Value), prepare_environment(Option<Value>), execute_tests(Option<Vec<String>>)}` returning `Result<Value>`; existing `run_stage` signature remains compatible. New status fields expose stage availability, recovery actions and recorded rejections.

- [x] Add failing scenario tests for preparation before Developer, environment-blocked but implementable plans, immutable startup/readiness retries, selected test reruns, independent baselines, and Validator without Developer.
- [x] Implement independent preparation and execution using Task 1's environment spec; keep automatic execution for legacy flows.
- [x] Add optional `execution_blocked_reason` to plans and `baseline` argv arrays to tests; preserve old artifacts and global configuration precedence.
- [x] Tie verdict evidence to each target's applicable baseline and invalidate current verdicts on rerun.
- [x] Expose MCP tools, structured recovery and dispatch rejection records; update scripted fixture compatibility.
- [x] Run focused harness, contract, MCP and workflow tests.

### Task 3: Prompts, reports and complete validation

**Files:** stage roles/skills, `src/reporting.rs`, README, validation documentation and report tests.

**Interfaces:** Coordinator instructions describe the new tools and environment recovery; reports include environment actions and per-candidate plan/execution state.

- [x] Add failing report tests for candidate-specific blockers and rejected versus launched attempts.
- [x] Update roles/skills to distinguish design from execution blockers, use real environment probes, and retry via changed prerequisites rather than identical dispatches.
- [x] Document inline Compose, readiness, selective reruns, per-test baselines, inventory authentication and limits.
- [x] Run `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, and `cargo test --locked`; run the real Docker smoke test if the daemon is available.
- [x] Obtain an independent review, address material findings, and record verification and remaining limits.
