# Rust migration

## Compatibility contract

Replace the Python AAQE implementation with a native Rust CLI. Preserve the
`aaqe` commands, optional `aaqe.yaml`, Stage 0–7 behavior, model-facing schemas,
five bundled roles and Skills, run/artifact/report JSON fields, retained tmux and
Docker resources, reproduction, cleanup ownership checks and existing run data.
Linux and WSL2 remain the supported runtime platforms. Example products may use
Python; AAQE itself must run without a Python interpreter or package.

The existing CLI is the public interface. Python implementation imports are
replaced by Rust modules. Keep version 0.1.0 and installation changes explicit.
Use the existing Python implementation and regression cases as the behavior
reference while migrating, then remove the Python runtime and its packaging.

## Approach

A native Rust port preserves the existing responsibility boundaries. A wrapper
around Python would retain the interpreter dependency; redesigning the workflow
would make behavioral compatibility harder to verify. Use serde JSON/YAML,
JSON Schema validation, tree-sitter-rust, an XML parser, and ordinary Git,
tmux, Codex and Docker subprocesses. Embed roles and Skills in the executable.
Persist path strings and JSON records in their existing format.

## Implementation plan

- [x] Port configuration, atomic private state, process groups, contracts and Git
  initialization to `src/{config,state,process,contracts,gitops}.rs`; test strict
  configuration, artifact links, timeout results and dirty-caller isolation.
- [x] Port patch and JUnit evidence gates to `src/{patchgate,evidence}.rs`; test
  production edits, traversal, symlinks, Rust mixed files, skipped/duplicate XML
  cases and failed-process/report disagreement.
- [x] Port live retained tmux sessions and structured Codex calls to
  `src/{tmux,codex}.rs`; test argument/environment transport, logs, timeouts,
  child termination, interruption, ownership and embedded resources.
- [x] Integrate Compose, workflow, reports and CLI in their matching Rust modules;
  test all stages, early termination, blocked/error reporting, baseline reversal,
  saved-run compatibility, reproduction and idempotent explicit cleanup.
- [x] Replace installation/CI instructions, remove Python implementation, run
  formatting, Clippy, full regression suite, real Docker integration and an
  installed-binary smoke check. Review the result and record unavailable
  external checks accurately in [validation.md](validation.md).

Publication follows validation: commit the reviewed changes, push to
`origin/main`, and compare the remote branch SHA with the local commit. The
user's request authorizes this publication; its result is reported separately
from local validation and remote CI.

## Review focus

Never copy caller changes into the audit worktree. Never loosen production
protection or source/evidence identity checks. Keep missing/skipped/ambiguous
JUnit results from confirming a defect. Never delete unowned tmux sessions or
Docker resources. Preserve initial evidence across reproduction and all terminal
statuses, including Ctrl+C. Reject missing or altered Compose snapshots during
resource inspection or cleanup. Keep the model-facing schema subset distinct
from stricter local validation.

The user requested implementation and remote submission; this plan is executed
within that authorization without an additional approval round.
