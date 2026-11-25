# AAQE Rust implementation plan

The current coordinator implementation plan is [agent-loop-plan.md](agent-loop-plan.md),
with its contract in [agent-loop-design.md](agent-loop-design.md). The earlier Rust
migration plan and compatibility baseline are in [rust-migration.md](rust-migration.md). The original Python implementation and
its plan are available at commit `3cb983e1827e510e601df64362b22a188bde1895`.

AAQE now has a native Rust CLI. The public commands, `aaqe.yaml`, model artifact
contracts, saved run/report JSON, and Stage 0–7 role identities remain compatible. Scheduling now uses the Codex
coordinator rather than a fixed Rust sequence; each stage invocation starts a fresh
process and retained tmux session.
The roles, Skills and schemas are embedded in the binary. Cargo replaces Python
packaging; Git, tmux, Codex and Docker remain external command boundaries.

Validation evidence for each implementation generation is recorded in
[validation.md](validation.md).
