# AAQE

**AI Agent Quality Engineering** — Describe a product or module. AAQE coordinates Codex agents to inspect code, add tests and validate findings in Docker, then submits the tests as a GitLab merge request.

English · [简体中文](README.zh-CN.md)

```text
Request → inspect → find candidates → design tests → implement → validate
                                         ↓
                     Docker evidence + module coverage + GitLab MR
```

AAQE protects production code, preserves execution evidence and retains the environment for replay. Tests use the project's existing conventions. No specific language, framework or test-report format is required. Generated tests must help reach **80% production line coverage** of the audited module, counting existing tests too.

## Install

**Linux only.** Release binaries support x86_64:

```bash
curl -fsSL https://raw.githubusercontent.com/qevolg/ai-agent-qe/main/install.sh | bash
export PATH="$HOME/.local/bin:$PATH"
aaqe doctor
```

The installer downloads a Release binary and verifies SHA-256; it does not install other tools. It requires a published Release. Use `--version VERSION` or `--bin-dir DIRECTORY` with `bash -s --` to choose a version or destination.

Install Git, tmux, authenticated Codex CLI, Docker Engine and Compose v2 on the host. Product dependencies and tests run inside Docker. GitLab delivery also uses authenticated `glab` and your Git commit identity.

Source alternative, with Rust **1.88+**:

```bash
git clone https://github.com/qevolg/ai-agent-qe.git
cd ai-agent-qe
cargo install --path . --locked
```

## Use

From the product repository:

```bash
aaqe run "Inspect the import module, focusing on recovery and duplicate writes"
```

AAQE fetches `origin/main` into an isolated worktree; local uncommitted product changes are excluded. It prints the coordinator's tmux attach command so you can follow the audit.

The final output includes the saved report and GitLab MR link. Tests that expose product bugs remain failing evidence. Below-threshold or unavailable coverage produces a blocked result and a Draft MR. Publication failures retain local work and can be retried.

## Configure

Configuration is optional. AAQE reads `aaqe.yaml` from the invocation directory, or the file selected by `--config PATH`.

```yaml
git:
  remote: origin
  branch: main

codex:
  model: gpt-6.1-sol

timeouts:
  agent: 7200
  test: 7200
  docker: 3600

loop:
  timeout: 43200
```

These are the defaults: two hours per agent/test command, one hour per environment operation and twelve hours for the whole audit. Environment health waiting defaults to ten minutes. The coordinator discovers project-specific build and test requirements.

See [the annotated example](aaqe.yaml.example) and [configuration reference](docs/configuration.md) for containers, paths, evidence and GitLab authentication. AAQE coverage does not automatically join GitLab CI coverage.

## Results

```bash
aaqe report RUN_ID
aaqe reproduce RUN_ID
aaqe publish RUN_ID
aaqe cleanup RUN_ID
```

Reports, logs, coverage and publication status live in `~/.aaqe/runs/RUN_ID/`. Reproduction reuses the saved environment and commands without an AI call. Cleanup removes owned containers and tmux sessions, retaining reports and test worktrees.

[Development and operation](docs/configuration.md#development) · [MIT License](LICENSE)

## Continue an audit and upgrade AAQE

After `blocked`, attach to the printed tmux session, repair the concrete blocker,
and tell the existing Codex conversation: “继续，从 test-validator 开始”. The
coordinator uses the harness `resume` tool; no new TUI is needed.

From another terminal:

```bash
aaqe resume --run-dir ~/.aaqe/runs/RUN_ID
aaqe resume --run-dir ~/.aaqe/runs/RUN_ID --from-stage test-validator
```

Stages are `code-explorer`, `bug-hunter`, `test-designer`, `test-developer` and
`test-validator`. Omitting the stage chooses the next actionable role. A live
coordinator receives the continuation in its original conversation; a replacement
TUI starts only when that coordinator is gone. Resume uses saved configuration,
the same base commit and worktree, and archives prior checkpoints under
`resumes/`. Attempts and raw evidence remain; each resumed round gets fresh active
budgets. Changed upstream analysis/tests invalidate dependent results. Matching
execution can be reused for Validator; missing evidence triggers execution again.
A policy refusal remains terminal.

New Runs copy AAQE into `RUN_DIR/runtime/aaqe` and record its SHA-256 and version.
All internal workers use the pinned copy. The installer verifies and flushes a
complete temporary file, then atomically switches the installed entry. Existing
Runs continue on their pinned executor. To explicitly adopt an installed fix:

```bash
aaqe resume --run-dir ~/.aaqe/runs/RUN_ID --from-stage test-validator --takeover
```

Takeover checks snapshot/protocol compatibility and saves a new executor alongside
all previous copies. Modern MCP relays route subsequent requests to it without
restarting the TUI. Legacy Runs acquire their first snapshot on resume; a live
legacy MCP cannot hot-load the fix, so AAQE delivers an explicit terminal-tool
bridge instruction to the same conversation. If that conversation cannot execute
the bridge, retain its history and close the old coordinator before retrying resume
to launch a replacement. Installing a file alone never repairs an old live MCP.

Only test code and necessary test fixtures in the audited project may change.
Tests may live anywhere, including inline source tests. Developer and independent
Validator review this semantic rule; AAQE enforces safe paths, recorded-base and
accepted-patch integrity. Test design considers relevant normal, boundary,
invalid-input, lifecycle, concurrency, dependency-failure and representation cases.
