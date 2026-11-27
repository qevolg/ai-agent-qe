# Container Test Dependencies Implementation Plan

> **For agentic workers:** Use executing-plans to implement this plan task by task in the current session.

**Goal:** Automatically install and verify audited-project test dependencies inside Docker, with recoverable execution and replay.

**Architecture:** Extend Compose startup with saved, idempotent setup argv; route test probes to the runner; let the coordinator submit repair recipes through the existing preparation tool. Keep AAQE and its control tools on the host.

**Tech Stack:** Rust 1.88+, Docker Compose, existing JSON schemas and Codex MCP.

**Spec:** `docs/superpowers/specs/2026-11-27-container-dependencies-design.md`

## Global Constraints

- Host AAQE runtime remains unchanged. Product dependencies install only in the owned runner.
- Preserve immutable Compose ownership, production patch protection, bounded probe capture, deadlines and budgets.
- Existing configurations and saved runs without setup remain readable.
- Automatic installation needs no operator approval; unavailable credentials/network/versions produce recorded blockers after bounded recovery.

## Review Focus

- A runner whose readiness failed still accepts probes and dependency repair.
- Invalid repair arguments must not mutate recipes, readiness, execution or verdicts.
- Recipe replay must survive runner recreation and avoid generated-test dependencies.
- Root setup must retain container-only execution and source protection.
- Environment mutation must invalidate current execution and independent conclusions, including on failure.

### Task 1: Compose setup and scoped probes

**Files:** `src/environment.rs`, `src/environment_probe.rs`, `tests/environment_runtime.rs`, `tests/environment_probe.rs`, configuration and Explorer schemas.

**Interfaces:** `ComposeEnvironment::record_setup(&[Vec<String>]) -> Result<()>`; initial `environment.setup` and saved extra recipes execute before readiness. `ComposeEnvironment::inspect(&Value, u64) -> Result<Value>` records runner probes using existing bounded capture.

- [x] Add failing regressions for initial/repair setup ordering, failure retry, snapshot reuse, invalid commands and runner probe routing.
- [x] Run the focused tests and observe the expected missing behavior.
- [x] Implement saved setup, validation and runner probe wrapping; extend schemas.
- [x] Run `cargo test --locked --test environment_runtime --test environment_probe` and commit the deliverable.

### Task 2: Harness recovery, replay and instructions

**Files:** `src/harness/environment.rs`, `src/harness.rs`, `src/workflow.rs`, `src/mcp.rs`, embedded roles/Skills, `tests/agent_loop.rs`, `tests/workflow.rs`, model fixtures, README and validation documentation.

**Interfaces:** `prepare_environment_with_setup(Option<Value>, Option<Vec<Vec<String>>>)` preserves `prepare_environment(Option<Value>)`. `TestRuntime::inspect` routes tools/endpoint probes into the factory's runtime. MCP preparation accepts optional setup argv.

- [x] Add failing recovery/invalidation and replay coverage, including invalid repair preserving current evidence.
- [x] Implement MCP preparation, persisted recipes, readiness reset and Validator/execution invalidation; replay setup in reproduction.
- [x] Update coordinator and stage instructions to infer, install, verify and retry without human approval.
- [x] Extend real Docker coverage to install a genuinely absent package, repair after startup, probe the runner, recreate it and reproduce.
- [ ] Run focused and full tests, formatting, strict Clippy, MSRV checks and real Docker validation; record actual results.
- [ ] Obtain one independent whole-change review, address actionable findings, commit, fast-forward the original main checkout and push origin/main; verify the remote SHA.
