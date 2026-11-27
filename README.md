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
aaqe doctor
```

The installer downloads the latest Release binary to `~/.local/bin` and verifies SHA-256; it does not install other tools. A published Release with the binary and checksum is required. If the installation directory is missing from PATH, the installer prints instructions for adding it. Use `--version VERSION` or `--bin-dir DIRECTORY` with `bash -s --` to choose a version or destination.

Install Git, tmux, authenticated Codex CLI, Docker Engine and Compose v2 on the host. Product dependencies and tests run inside Docker. For GitLab delivery, authenticate with `glab auth login --hostname YOUR_GITLAB_HOST` and configure your Git commit identity.

Source alternative, with Rust **1.100.0+**:

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

AAQE selects the current branch's remote, falling back to `origin` or the only available remote, and fetches its default branch into an isolated worktree; local uncommitted product changes are excluded. It prints the coordinator's tmux attach command so you can follow the audit.

The final output includes the saved report and GitLab MR link. Tests that expose product bugs remain failing evidence. Below-threshold or unavailable coverage produces a blocked result and a Draft MR. Publication failures retain local work and can be retried.

## Configure

Configuration is optional. AAQE reads `aaqe.yaml` from the invocation directory, or the file selected by `--config PATH`.

```yaml
codex:
  model: gpt-6.1-sol
```

The default model is `gpt-6.1-sol`; set `model` to `""` to inherit Codex settings. Override the Git target, containers, references, baseline commands, allowed test paths or GitLab project only when needed; see [the configuration example](aaqe.yaml.example). If a Git target cannot be determined, AAQE asks you to set `git.remote` or `git.branch`.

Execution limits are built in: two hours per agent/test command, one hour per environment operation and twelve hours for the audit. Environment health waiting defaults to ten minutes. AAQE coverage does not automatically join GitLab CI coverage.

## Results

```bash
aaqe report RUN_ID
aaqe reproduce RUN_ID
aaqe publish RUN_ID
aaqe cleanup RUN_ID
```

Reports, logs, coverage and publication status live in `~/.aaqe/RUN_ID/`. Records in the previous `~/.aaqe/runs/RUN_ID/` directory remain accessible. Reproduction reuses the saved environment and commands without an AI call. Cleanup removes owned containers and tmux sessions, retaining reports and test worktrees.

[MIT License](LICENSE)
