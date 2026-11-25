# Codex Agent Loop Implementation Plan

> **For agentic workers:** Use executing-plans to implement this plan inline,
> followed by requesting-code-review for one independent whole-branch review.

**Goal:** Replace fixed stage scheduling with a Codex coordinator that dispatches
independent retained stage windows through the Rust harness, then publish.

**Architecture:** Keep patch, Docker, JUnit, report and worktree boundaries.
Extract a persistent audit harness and expose its stage/state/finish operations
through stdio MCP to one coordinator Codex process.

**Tech Stack:** Rust 1.88+, existing dependencies, Codex CLI, tmux, Docker Compose.

**Spec:** `docs/agent-loop-design.md`

## Global Constraints

- Preserve the CLI, saved runs, first-attempt stage files and reproduction.
- Every stage invocation gets a new Codex process and retained tmux session.
- The coordinator chooses stages; Rust checks dependencies and evidence.
- Preserve production patch protection and all historical execution evidence.
- User authorization includes implementation, commit and push to origin/main.

## Review Focus

- Failed retries must not leave a half-replaced patch or a stale confirmed verdict.
- Upstream re-analysis must not silently reuse incompatible downstream results.
- Coordinator cancellation must also stop separately launched stage workers.
- Nested attempt cleanup must reject foreign ownership and directory symlinks.
- A model final message alone must not bypass finish/evidence requirements.

### Task 1: Persistent audit harness and dynamic dispatch

**Files:** `src/{workflow,harness,config,contracts}.rs`, configuration schema,
`tests/{workflow,agent_loop}.rs`.

**Interfaces:** `AuditAgent::coordinate(&mut AuditHarness)` drives the audit;
`AuditHarness::{run_stage,status,finish,reload,finalize}` manages persisted state.

- [ ] Add regression tests for stage revisit/retry, unique attempt files,
  stale-artifact invalidation, dependency rejection, finish gates and budgets.
- [ ] Run the new tests and observe failure before implementing dispatch.
- [ ] Extract the harness, keep real baseline/target execution, handle patch
  replacement/rollback, preserve reports and add backward-compatible budgets.
- [ ] Run `cargo test --locked --test agent_loop --test workflow`.
  Expected: new dispatch checks and existing workflow cases pass.

### Task 2: Native coordinator tool loop and retained sessions

**Files:** `src/{codex,mcp,cli,tmux}.rs`, coordinator resources,
`tests/{transport,agent_loop}.rs`, scripted model fixture.

**Interfaces:** `aaqe __harness-mcp RUN_DIRECTORY` speaks newline JSON-RPC MCP;
`Codex::coordinate` registers the local harness only for the main session.

- [ ] Add protocol and subprocess tests proving coordinator tool calls start
  independent workers, return failures, retain all attempts and enforce finish.
- [ ] Observe failing tests, then implement the stdio protocol and coordinator.
- [ ] Cover recursive ownership-checked cleanup and cancellation of active workers.
- [ ] Run `cargo test --locked --all-targets`.
  Expected: all ordinary tests pass, opt-in Docker test remains explicitly ignored.

### Task 3: Documentation, acceptance and publication

**Files:** `README.md`, `docs/{design,implementation-plan,validation}.md`.

- [ ] Document windows, tool feedback, attempt layout, budgets and limitations.
- [ ] Run formatting, strict Clippy, full tests, release build and package checks;
  run the real Docker integration and a bounded actual-Codex transport check.
- [ ] Request independent review; fix material findings with regression coverage.
- [ ] Commit reviewed changes, push to origin/main without force, verify remote SHA
  and report remote CI separately from local verification.
