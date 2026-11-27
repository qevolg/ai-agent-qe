# Codex coordinator and independent stage attempts

The user approved implementation and remote publication on 2026-11-25 after
discussing the following design. One retained Codex coordinator window owns the
audit's decisions. It invokes a local Rust MCP harness to run any of the five
existing stage roles. Each run retains exactly one interactive coordinator TUI in
tmux, named `aaqe-<12 hex characters>`. Every stage invocation starts a fresh
independent background `codex exec` process without tmux, including retries.

The coordinator may repeat or revisit stages, request additional evidence, finish
with no candidates, or report a blocker. Rust enforces prerequisites, allowed
roles, patch protection, real execution evidence, independent validation and
finite call/time budgets. It does not choose the next stage. Workers return
structured artifacts; their errors and actual execution summaries return to the
coordinator as tool results. Product failures are retained as evidence, not
repaired into passing tests.

The first invocation retains `stages/<role>/` and all existing files. Later
invocations use `stages/<role>/attempts/<attempt>/`. Each contains input, prompt,
Skill, schema, output, process logs and a harness result. Role inputs select required
upstream artifacts, completed attempt summaries and recent environment actions.
Developer retries retain actual execution feedback; an artifact index points to
other accepted results for upstream revisits. Complete raw logs stream to disk;
process summaries retain bounded head/tail output and MCP payloads use smaller
excerpts. Machine-readable control commands keep full capture. Accepted
artifacts retain immutable per-attempt copies and the existing latest-artifact
paths. Re-running an upstream stage invalidates downstream active artifacts and
verdicts without deleting historical evidence. Developer retries replace the
previous accepted patch against the same recorded base and restore it if the new
patch is rejected. A persisted transition journal precedes patch changes and active
artifact invalidation; recovery reconciles the worktree and active files with the
committed harness snapshot. Explorer references are checked against regular files
and line bounds at the recorded base, excluding earlier generated tests. Each developer attempt runs a fresh baseline and target tests.

The coordinator has state, run-stage and finish tools. A persistent coordinator
interactive `codex --no-alt-screen --no-daemon` uses its native tool loop with
stdin, stdout and stderr inherited from the tmux terminal. A private stdio MCP
subprocess owns the harness state during that session.
Stage Codex calls cannot inherit the coordinator's dispatch tool. The outer CLI
always reloads state and writes partial reports after model failure, timeout or
interruption, and cancels active background stages while retaining their logs.
After a validated MCP `finish` response is saved and flushed, its acknowledgement
allows the outer CLI to generate the report and return. The coordinator remains
interactive, without the audit timeout; subsequent status calls reload the
finalized state, including later reproductions, and further dispatches are rejected. `terminal.log` records the
TUI terminal stream using tmux pipe-pane without redirecting Codex's streams.
The coordinator process record is written when the TUI actually exits.

Cancellation or a disappearing TUI cancels the MCP server's active process group
and stage descendants before the outer CLI recovers patches and writes its
partial report. The MCP server watches its parent and the coordinator cancellation
marker. Each background stage has a small AAQE supervisor in its own process
group, watching its original MCP parent and cancellation marker. MCP loss therefore
cancels the stage even if Codex kills MCP before its own shutdown. Advisory locks
let shutdown wait for live MCP/stage supervisors while releasing ownership after
a hard kill. Cleanup discovers the one new coordinator session and legacy nested
stage sessions with ownership checks. Saved older runs, reports and reproduction
remain readable.

The versioned `harness.json` stores only state, accepted artifacts, execution and
verdicts, and is the commit point. `run.json` and `execution.json` are compatibility
exports. Available stages and recovery actions are derived on query. Store updates
and MCP reloads use the same snapshot loader. An unversioned snapshot imports only
post-audit fields from its old run projection; Python runs without a harness keep
their legacy path. A persistent `operation.lock` inode serializes MCP and
post-audit operations. Reproduction/cleanup reject an active audit or a busy run.
Reproduction journals its attempt before startup and saves partial records and
failures. Reports rebuild from the snapshot without requiring a previous report;
resource cleanup and state persistence proceed independently of rendering.

Independent background workers preserve separate stage contexts and attempts.
The persistent TUI keeps the coordinator's conversation available after the audit.

Defaults: at most 20 stage calls, at most 4 attempts per role, and 14400 seconds
until audit finish, configurable under `loop`. Stage agent,
Docker and test timeouts remain separately enforced. A completed run requires
current Explorer and Hunter artifacts, and independent validation if candidates
exist; blocked finish is always available with a reason. No new role definitions
or arbitrary stage names are accepted.
