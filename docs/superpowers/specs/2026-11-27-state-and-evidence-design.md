# State and evidence flow

The user approved the eight improvements from the source review at `5099640`.
Keep JSON and existing evidence contracts; do not add SQLite or a workflow framework.

Use a versioned, validated `HarnessSnapshot` containing state, accepted artifacts,
execution and verdicts. New snapshots are authoritative; run/execution JSON files
are compatibility projections. Import post-audit fields from unversioned runs once.
Keep Python runs without a harness readable and mutable through `Store`.

Serialize operations on one run with a persistent advisory lock. MCP calls and
audit finalization use the same lock as reproduction/cleanup/report regeneration.
Reject reproduction during initializing/running audits. Record reproduction before
environment startup and persist preparation, partial results and failures separately
from original audit evidence. A crashed running reproduction is identified on retry.

Build stage-specific inputs with bounded diagnostics and evidence paths, preserving
Developer retry feedback and Validator inputs. Summarize large tool payloads while
keeping full evidence on disk. Long-running processes stream full output to files
and retain bounded head/tail summaries in memory; machine-readable Docker commands
keep complete capture. Do not weaken JUnit or evidence-link checks.

Reports are derived from the authoritative snapshot, with legacy report fallback.
Missing/corrupt derived reports are rebuildable. Rendering failure must not prevent
owned resource cleanup. Correct obsolete tmux/process documentation.

Alternatives considered: keeping independent run/harness writers retains lost-update
risks; SQLite adds migration and deployment work without resolving input/log volume.
The approved approach keeps small file-based boundaries and existing recovery.
