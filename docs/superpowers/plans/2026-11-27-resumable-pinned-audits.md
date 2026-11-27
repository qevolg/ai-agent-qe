# Resumable Pinned Audits Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make active audits survive installation changes and resume safely in their retained conversation.

**Architecture:** Immutable per-run executors, a stateless request dispatcher behind the persistent MCP relay, and explicit recovery rounds in the authoritative snapshot. Patch integrity is independent from semantic test-only review.

**Tech Stack:** Rust, native subprocess tests, Git, tmux, stdio MCP.

**Spec:** docs/superpowers/specs/2026-11-27-resumable-pinned-audits-design.md

## Global Constraints

Only test code may change in the audited target project. AAQE implementation may change. Retain original base, cumulative patch, raw evidence and attempt numbering. No coordinator stage value. No new YAML settings for upgrade behavior. User has authorized commit and remote push.

## Review Focus

- Snapshot creation after the installed binary inode is unlinked must preserve that inode.
- Resume during another tool operation must reject without corrupting state.
- Rerun upstream analysis must invalidate patch, execution and downstream conclusions.
- A resumed finish without the original CLI must update reports and publication.
- Legacy coordinator servers cannot acquire new code implicitly.

### Task 1: Patch integrity and test planning

**Files:** src/guards/patch.rs, src/config.rs, call sites, resources, aaqe.yaml.example, docs.
**Interfaces:** PatchGate accepts any safe path; with_patch records exact expected worktree contents.

- [x] Write tests for inline/nonstandard tests and accepted-patch drift, observe failures.
- [x] Remove directory/AST policy, enforce exact worktree patch integrity in execution/reproduction/publication.
- [x] Simplify configuration and broaden Designer/Validator prompts.
- [x] Run focused patch/config/agent tests; expected all pass. Commit task.

### Task 2: Immutable executor and worker dispatch

**Files:** src/runtime/executor.rs, process.rs, tmux.rs, agents/codex.rs, agents/mcp.rs, CLI internal commands, installer tests.
**Interfaces:** capture returns immutable runtime metadata; resolve validates recorded hash; MCP dispatch uses active executor on every call.

- [x] Write upgrade, tamper and deleted-inode regression tests; observe failures.
- [x] Capture durable immutable per-run executable and pin worker/MCP/terminal launches.
- [x] Extract single-request harness dispatch and relay it through the active executor.
- [x] Verify focused transport/MCP/installer tests; expected all pass. Commit task.

### Task 3: Resume and finalization

**Files:** src/audit/harness/resume.rs, harness lifecycle, CLI, tmux, MCP, tests.
**Interfaces:** resume(from_stage) archives checkpoints and starts a fresh round; external resume reuses live TUI and explicit takeover selects compatible executor.

- [x] Write resume lifecycle, budgets, invalidation, stale evidence and concurrent operation tests; observe failures.
- [x] Implement stage selection, checkpoint archives, recovery integrity and budget offsets.
- [x] Add CLI resume and harness resume, retained-TUI delivery, replacement TUI fallback and explicit takeover. Review found legacy terminals cannot perform a writable bridge; reject live legacy recovery and require closing the old TUI first.
- [x] Factor idempotent finalization callable after persistent harness finish.
- [x] Verify focused and full suite; expected all pass. Commit task.

### Task 4: Documentation, review and publication

- [x] Document commands, lifecycle and upgrade limitations in both READMEs/configuration docs.
- [x] Run fmt, clippy, complete native test suite and release build.
- [x] Request one fresh-context whole-branch reviewer, fix important findings with failing regression tests.
- [x] Commit and push approved task branch, verify remote SHA.

## Completion evidence

The final native suite passed 134 tests with no failures, including five tests that also passed their isolated subprocess checks; the Docker integration is run separately. Stable and Rust 1.88 Clippy with warnings denied, formatting, release build and cargo package verification passed. One fresh-context reviewer completed the whole-branch review and focused follow-ups with no unresolved Critical or Important findings. The implementation was pushed to `origin/feat/resumable-pinned-audits`, and remote SHA `1ba91a2c8b4a9369002868a4b3b1f7d94505f523` matched the implementation commit.

Legacy coordinators cannot use a writable bridge through their read-only terminal sandbox. They require preserving the conversation history, closing the old TUI and resuming the same Run with a replacement; modern relays retain the original TUI. Abnormal resumed exits save local reports without waiting for remote publication; blocked timeout results retain explicit `aaqe publish RUN_ID` retry.
