# AI Agent QE v0.1

AAQE is a generic Rust tool for product-wide or module-level quality engineering. The user starts `aaqe` inside a Git repository and describes an audit in natural language. `aaqe run "prompt"` provides the same workflow for automation. No named target is required.

## Accepted behavior

- Stage 0 discovers the caller's repository, fetches `origin/main`, records its exact SHA, and creates a unique branch and worktree. Remote and branch can be configured. Caller changes are neither copied nor changed.
- Stages 1–5 each use one fresh `codex exec` call in a named, retained tmux session with an explicitly supplied generic role, Skill, JSON Schema, and validated preceding artifacts. The CLI announces the attach command and saves it as `stages/<stage>/tmux.json`. A native Rust worker mirrors stdout/stderr to the pane and live log files, records the real Codex exit status, and terminates the process group on timeout or interruption. Explorer interprets the prompt and discovers scope and execution requirements. Skills describe inputs, work, outputs, and completion conditions, without product knowledge.
- Stage 6 always saves a Markdown and JSON report, including partial/failed runs. An empty candidate set is an ordinary early finish.
- Docker Compose provides real environments using existing images; explicitly configured external containers are inspected and never deleted. Container execution uses argv arrays. Product service definitions, credentials, fixtures, and terminology remain in the audited project.
- Test Developer returns a unified patch. Rust validates its paths and changes before applying it. Test files, fixtures, and explicitly trusted test infrastructure may change. Production changes are rejected. Rust `#[cfg(test)] mod` regions use syntax-tree checks to preserve all code outside these regions; unsupported mixed-language files require independent test files.
- Stage 5 builds/starts the environment, runs configured baseline commands without the generated patch, applies the validated patch, executes each planned test inside the runner container, and parses JUnit evidence. A failing process or XML placeholder is insufficient to confirm a bug. Required case names must actually occur, and Validator conclusions must reference recorded evidence.
- Each execution has fresh report paths. Attempts, stdout/stderr, exit codes, expected and actual case names, skipped/missing cases, and reproduction steps are retained. Reproduction reuses saved tests without another AI call.
- Containers, networks, data, tmux sessions, branch and worktree remain after reporting. Stage 7 is an explicit `aaqe cleanup RUN_ID`, restricted to that run's recorded tmux sessions and owned Compose project. Tmux cleanup checks each session's stage-directory ownership, skips absent sessions and works even if the run never created a Docker environment. Active audits cannot be cleaned. Reports, stage logs and worktrees remain. No global Docker prune or global tmux cleanup.

## Distribution and boundaries

The independent repository ships the CLI, workflow, five roles/Skills, schemas, patch policy, Compose controller and JUnit parser. Optional `aaqe.yaml` supplies operational settings only. The prompt determines scope. Environment and test setup may be discovered by Explorer; missing executable environment details yield a blocked report rather than a claimed verification.

The tool runs on Linux or WSL2 with tmux and Docker Engine / Docker Desktop Linux containers. Native Windows execution is unsupported because Codex stages require tmux. Unit/integration tests cover local Git repositories, real tmux sessions and a fake Codex executable; separate opt-in Docker smoke tests exercise real containers. Static and simulated evidence is reported separately from a real model audit against a product.

Codex read-only sandboxing is required. The tool performs host-side writes and Docker orchestration; it does not grant agents Docker access. Project hooks/MCP and Docker definitions remain trusted local configuration; read-only agents and Patch Gate are not a hostile-code sandbox. Existing test assertions are independently reviewed by Validator. Native test-only config paths must be explicitly allowed before mixed build files may change.

## Decisions

Use a deterministic Rust workflow rather than an agent deciding stages. Keep reusable stage Skills embedded in the Rust executable, rather than copying them into every product. Keep per-project operational configuration optional and never ship product-specific data source logic in the package. Use private GitHub visibility for initial publication.
