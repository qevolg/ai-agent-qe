# AAQE Implementation Plan

> Execute inline with focused failing tests before each implementation slice.

**Goal:** Publish an installable, prompt-driven CLI implementing the accepted Stage 0–7 workflow.

**Architecture:** Python controls fresh Codex stage calls, isolated Git worktrees and retained Docker Compose projects. Structured artifacts link candidates, test plans, patches, execution evidence and final verdicts.

**Tech stack:** Python 3.11+, argparse, PyYAML, jsonschema, tree-sitter-rust, pytest, Codex CLI, Git, Docker Compose.

**Spec:** `docs/design.md`.

## Global constraints

- No product-specific business terminology or services in runtime code or Skills.
- Never modify the caller's worktree; never delete attached external containers.
- Preserve reports and environments on success, blocked execution, timeout and interruption.
- Exactly one independent Codex call per reached AI stage; no hidden model retries.
- Never confirm a bug using missing, skipped, ambiguous or process-only results.

## Review focus

- Traversal, symbolic links and mixed Rust files cannot bypass the production-code gate.
- Remote main moves independently of the caller's dirty branch; use the fetched SHA.
- Fresh report directories prevent stale XML from validating an unexecuted case.
- Cleanup must not affect another run or external resources.
- Packaged Skills and schemas must survive wheel installation outside the checkout.

## Tasks

1. [x] Write failing configuration and real-Git tests; implement validated config, process runner, state storage and Git initialization.
2. [x] Write failing patch and JUnit tests; implement production boundaries, test-only Rust regions and execution evidence.
3. [x] Write failing Codex/Compose tests; implement one-shot structured calls, resource packaging and retained environments.
4. [x] Write failing workflow/CLI tests; implement stages, reports, reproduction and explicit cleanup.
5. [x] Add installation/usage documentation, CI and a generic real-Docker smoke fixture; run the full suite, installed-wheel and Docker validation.
6. [x] Review and fix verified findings; prepare the private GitHub repository for publication.

## Progress

- Design implements the behavior approved in the conversation. The user explicitly requested implementation and remote publication; continue without another design approval round.
- GitHub authenticated account: `qevolg`. Destination: `project/ai/ai-agent-qe`.
- Private GitHub repository created: `qevolg/ai-agent-qe`. Publication uses the user-authorized initial commit on `main`; verify the remote SHA after pushing.
- Local checks and remaining platform limitations are recorded in `docs/validation.md`.
