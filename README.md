# AAQE

**AI Agent Quality Engineering** — Describe a product feature or module. AAQE coordinates Codex agents to document scenarios, exercise the real product and investigate unexpected results, then delivers the steps, execution evidence and an independent report.

English · [简体中文](README.zh-CN.md)

```text
Request → explore product → design scenarios → execute → reproduce → review
                                  ↓
                        steps.md + evidence + report.md/report.json
```

Each `aaqe run` is one testing task. AAQE expands the requested feature into normal, abnormal and boundary scenarios, with state transitions, permissions, data correctness and interactions between entry points where relevant. It tests all applicable SQL, API, Explorer and CLI entry points. Explorer scenarios operate the actual browser page; API calls alone do not establish that a page works. Source code and documentation help explain expected behavior and diagnose failures; testing can begin without a suspected code defect.

The coordinator keeps five stages:

1. `product-explorer` identifies the feature, applicable entry points, references and required services.
2. `scenario-designer` writes the scenarios and expected results to `steps.md` before execution.
3. `product-executor` performs the documented operations and inspects the actual responses, continuing browser or interactive sessions as needed.
4. `issue-reproducer` investigates unexpected behavior and records reproducible issues. New clues within the task's scope first go back into the scenario document; clues outside the scope are recorded for later work.
5. `result-reviewer` checks every current scenario and each issue against the saved evidence, real services and expected behavior. A process exit code alone does not establish product correctness.

Scenario revisions preserve their history and sources. Changing a scenario invalidates that scenario's earlier evidence while preserving evidence for unchanged scenarios. AAQE protects product code and retains evidence and the environment for replay. New product tasks deliver documents and results without a code coverage threshold or an automatically generated test MR.

**Real services are required; mocks are prohibited.** For each service, an address explicitly supplied in the task prompt takes priority. Every service without such an address must run as a real Docker instance. A task can use an external TDengine and a Docker source or sink at the same time. An unavailable external address blocks the affected scenarios; AAQE must not silently substitute a local service. The persistent Docker runner executes the operations in either deployment mode. Service protocol checks and, for Docker services, container/image identities are retained with the evidence. Mocks, stubs, spies, fake clients/servers, replacement transports and response replay cannot substitute for product validation.

## Install

**Linux only.** Release binaries support x86_64:

```bash
curl -fsSL https://raw.githubusercontent.com/qevolg/ai-agent-qe/main/install.sh | bash
aaqe doctor
```

The installer downloads the latest Release binary to `~/.local/bin` and verifies SHA-256; it does not install other tools. A published Release with the binary and checksum is required. If the installation directory is missing from PATH, the installer prints instructions for adding it. Use `--version VERSION` or `--bin-dir DIRECTORY` with `bash -s --` to choose a version or destination.

Install Git, tmux, authenticated Codex CLI, Docker Engine and Compose v2 on the host. Product dependencies and operation tools, including browser tools when needed, run inside Docker. Publishing tests from a historical audit additionally requires `glab auth login --hostname YOUR_GITLAB_HOST` and a Git commit identity.

Source alternative, with Rust **1.100.0+**:

```bash
git clone https://github.com/qevolg/ai-agent-qe.git
cd ai-agent-qe
cargo install --path . --locked
```

## Use

From the product repository:

```bash
aaqe run "Verify the import feature, including normal imports, recovery, duplicate writes and boundary inputs. Test every applicable SQL, API, Explorer and CLI entry point."
```

AAQE selects the current branch's remote, falling back to `origin` or the only available remote, and fetches its default branch into an isolated worktree; local uncommitted product changes are excluded. It prints the coordinator's tmux attach command so you can follow the task.

To use an existing service, name its address explicitly in the prompt:

```bash
aaqe run "Verify query behavior using TDengine at https://db.example:6041. Start other required services in Docker. Cover valid, invalid and boundary queries through all applicable entry points."
```

The final output identifies the saved report. `steps.md` records the operation document; `report.md` summarizes all scenarios and lists issues separately; `report.json` provides schema version 2.0 data and links to raw evidence. Results distinguish passed, failed, blocked, not executed and flaky scenarios. The report records service sources, endpoints and observed versions separately from the source checkout's commit. Interrupted tasks preserve partial evidence and do not count missing execution as a pass.

## Configure

Configuration is optional. AAQE reads `aaqe.yaml` from the invocation directory, or the file selected by `--config PATH`.

```yaml
codex:
  model: gpt-6.1-sol
```

The default model is `gpt-6.1-sol`; set `model` to `""` to inherit Codex settings. Override the Git target, containers or references only when needed; see [the configuration example](aaqe.yaml.example). Explicit external service addresses belong in the task prompt so their intended use is recorded. Baseline, test-path and publication settings remain available for historical test-generation audits. If a Git target cannot be determined, AAQE asks you to set `git.remote` or `git.branch`.

Execution limits are built in: two hours per agent/operation command, one hour per environment operation and twelve hours per task. Environment health waiting defaults to ten minutes.

## Results

```bash
aaqe report RUN_ID
aaqe reproduce RUN_ID
aaqe cleanup RUN_ID
```

Each task lives in `~/.aaqe/RUN_ID/`, including its steps, reports, logs, execution evidence and history. Records in the previous `~/.aaqe/runs/RUN_ID/` directory remain accessible. Reproduction reuses the saved environment and operations without an AI call and appends replay records while preserving the original review. Cleanup removes owned containers and tmux sessions, retaining reports, documents and worktrees; external services remain under their owners' control.

Historical audits retain their original report and test replay behavior. `aaqe publish RUN_ID` remains available to publish their saved test patches; product scenario tasks do not automatically create a test MR.

[MIT License](LICENSE)
