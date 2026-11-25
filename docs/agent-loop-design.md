# Codex coordinator and independent stage attempts

The user approved implementation and remote publication on 2026-11-25 after
discussing the following design. One retained Codex coordinator window owns the
audit's decisions. It invokes a local Rust MCP harness to run any of the five
existing stage roles. Every invocation starts a fresh independent Codex process
in its own retained tmux session, including repeated invocations of a role.

The coordinator may repeat or revisit stages, request additional evidence, finish
with no candidates, or report a blocker. Rust enforces prerequisites, allowed
roles, patch protection, real execution evidence, independent validation and
finite call/time budgets. It does not choose the next stage. Workers return
structured artifacts; their errors and actual execution results return to the
coordinator as tool results. Product failures are retained as evidence, not
repaired into passing tests.

The first invocation retains `stages/<role>/` and all existing files. Later
invocations use `stages/<role>/attempts/<attempt>/`. Each contains input, prompt,
Skill, schema, output, process logs, tmux ownership and a harness result. Accepted
artifacts retain immutable per-attempt copies and the existing latest-artifact
paths. Re-running an upstream stage invalidates downstream active artifacts and
verdicts without deleting historical evidence. Developer retries replace the
previous accepted patch against the same recorded base and restore it if the new
patch is rejected. A persisted transition journal precedes patch changes and active
artifact invalidation; recovery reconciles the worktree and active files with the
committed harness snapshot. Explorer references are checked against regular files
and line bounds at the recorded base, excluding earlier generated tests. Each developer attempt runs a fresh baseline and target tests.

The coordinator has state, run-stage and finish tools. A persistent coordinator
`codex exec` uses its native tool loop; no model decision loop is recreated in
Rust. A private stdio MCP subprocess owns the harness state during that session.
Stage Codex calls cannot inherit the coordinator's dispatch tool. The outer CLI
always reloads state and writes partial reports after model failure, timeout or
interruption, and stops any still-running owned stage worker while retaining its
window and logs. Cleanup discovers coordinator and nested attempt sessions with
the existing ownership checks. Saved older runs, reports and reproduction remain
readable.

Alternatives considered were one worker session for all roles (less isolation and
does not meet the per-stage-window request) and serial resume in successive
windows (shares stage context and moves the decision loop into the host). The
coordinator plus independent workers meets the agreed requirements.

Defaults: at most 20 stage calls, at most 4 attempts per role, and 14400 seconds
for the complete coordinator session, configurable under `loop`. Stage agent,
Docker and test timeouts remain separately enforced. A completed run requires
current Explorer and Hunter artifacts, and independent validation if candidates
exist; blocked finish is always available with a reason. No new role definitions
or arbitrary stage names are accepted.
