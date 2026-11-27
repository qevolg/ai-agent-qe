# Reports and Documentation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. The user has authorized implementation and remote publication.

**Goal:** Deliver Chinese validation reports, Shanghai run names and matching tmux sessions, MIT licensing and complete bilingual documentation.

**Architecture:** Keep the current snapshot/evidence/cleanup behavior. Change the shared naming source and coordinator session name, render Chinese Markdown from existing machine data, and reorganize documentation around quick start and reference tables.

**Tech Stack:** Rust 1.88+, chrono, tmux, Docker Compose, Markdown/Mermaid.

**Spec:** `docs/superpowers/specs/2026-11-27-reports-and-documentation-design.md`

## Global Constraints

- Run ID: `YYYYMMDDHHmmss-xxxxxxxx`, `Asia/Shanghai`, eight lowercase hexadecimal random characters.
- Coordinator session: `aaqe-<run-id>`; legacy cleanup, ownership checks, snapshots, locks and JSON contracts remain compatible.
- Default README English; Chinese README retained; MIT; configuration remains optional and caller-relative.
- No claims of product acceptance or real-model reasoning from deterministic fixtures.

## Review Focus

- UTC-to-Shanghai conversion across midnight and independence from host timezone.
- New and old tmux names, repeated cleanup, ownership mismatch and sessions from another run.
- Incomplete analysis with no candidates, completed audits with blocked candidates, historical retries and escaped table cells.
- Reproduction/cleanup report refresh preserving original evidence and verdicts.
- Complete public CLI/config documentation and distributable license/config/README files.

### Task 1: Shanghai naming and coordinator identity

**Files:** `src/state.rs`, `src/tmux.rs`, `src/codex.rs`, `tests/core.rs`, `tests/mcp.rs`, `tests/transport.rs`.

**Interfaces:** Store still creates a string run ID; `run_tui` receives that ID explicitly from the coordinator; legacy `run_tmux` naming remains supported.

- [x] Add a deterministic UTC fixture covering Shanghai midnight rollover, check stored `+08:00` timestamps and new Store identity; update real coordinator integration to assert `aaqe-<state.id>`.
- [x] Observe naming assertions fail, implement shared conversion/session identity and compatible cleanup patterns, then run core/MCP/transport tests.

### Task 2: Chinese validation report

**Files:** `src/reporting.rs`, `src/codex.rs`, `src/resources/agents/coordinator.md`, `tests/reporting.rs`, `tests/orchestration.rs`.

**Interfaces:** Existing `generate_report` and JSON fields remain stable; only Markdown presentation and natural-language generation instructions change.

- [x] Add report cases for all verdicts, escaped table values, missing Hunter versus empty analyzed scope, and historical records excluded from current summary; observe failure.
- [x] Render opening summary, conclusion, scope/progress, environment/evidence, results, issue records, path adjustments, limits and replay/retention sections in Chinese; retain source/error/command literals and snapshot refresh.
- [x] Run reporting/orchestration tests and inspect actual rendered fixture output.

### Task 3: Documentation, configuration and MIT

**Files:** `README.md`, `README.zh-CN.md`, `docs/runtime.md`, `aaqe.yaml.example`, `LICENSE`, `Cargo.toml`, `docs/validation.md`.

**Interfaces:** Human documentation matches existing CLI/config schema; package include paths contain both languages, config example and license.

- [x] Create aligned READMEs with Mermaid architecture, install/quick start, complete CLI/options, YAML fields/defaults/path precedence, report semantics and lifecycle; preserve advanced reference in runtime document.
- [x] Add parseable annotated example and MIT license, update package metadata; verify help, relative links, example loading and package contents.
- [x] Run formatting, strict Clippy and all tests, opt-in real Docker workflow, Rust 1.88 checks, release/package verification; write Chinese validation evidence.
- [x] Review full diff independently, address material findings, commit and publish to remote, verify remote SHA and report CI separately.
