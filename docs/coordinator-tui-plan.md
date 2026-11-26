# Coordinator TUI implementation plan

Approved behavior: one retained interactive Codex TUI per run, named
`aaqe-<12 hex characters>`. Stages use fresh background `codex exec` processes.
The main CLI reports the audit result after the guarded MCP `finish`; the TUI
continues accepting input until explicitly closed or cleaned up.

1. Add regression coverage in `tests/mcp.rs` and `tests/transport.rs` for a real
   terminal on all coordinator streams, input after report generation, one tmux
   session, independent stage attempts, streaming stage logs, and cancellation.
   Adapt the scripted Codex fixture to distinguish interactive and exec modes.
2. Split coordinator and stage command construction in `src/codex.rs`. Add a
   retained terminal worker in `src/tmux.rs`, a finish acknowledgement in
   `src/mcp.rs`, and supervised background process execution in `src/process.rs`.
   Cancel active stages on coordinator shutdown, interruption or audit timeout.
   Keep legacy tmux cleanup and ownership validation.
3. Update the coordinator prompt, README and existing agent-loop design to
   describe live TUI retention, terminal transcripts and stage process records.
   Run formatting, all tests, Clippy, the Docker integration when available,
   and a bounded real Codex TUI smoke check. Review the final diff, commit and
   push to the current remote branch, then verify the remote commit.

The implementation and regression tests are complete. Actual Codex TUI transport,
post-report follow-up, real Docker execution and cancellation with forced MCP
loss have passed. See `validation.md` for the scope of local verification.
