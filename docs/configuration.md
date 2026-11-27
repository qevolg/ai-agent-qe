# Configuration and operation

AAQE supports Linux. Its control tools run on the host; all product builds, dependencies, fixtures, coverage and tests run in an owned Docker environment.

## Configuration file

Installation does not create a configuration. AAQE reads `aaqe.yaml` only in the invocation directory; it does not search parents. Missing default configuration uses built-in defaults. An explicit missing `--config PATH` is an error. Unknown fields are rejected.

Configuration is read before creating the audit worktree. Local uncommitted configuration can therefore be used. The source baseline is fetched from the configured remote branch; local product changes are excluded.

| Field | Default | Meaning |
| --- | --- | --- |
| `state_dir` | `~/.aaqe/runs` | Evidence, retained worktrees and reports |
| `git.remote` / `git.branch` | `origin` / `main` | Remote baseline and publication target |
| `codex.command` | `[codex]` | Executable and leading arguments |
| `codex.model` | `gpt-6.1-sol` | Coordinator and independent workers; explicit empty string inherits Codex settings |
| `references` | `[]` | Project behavior/interface documentation |
| `baseline` | `[]` | Trusted global baseline argv arrays; nonempty values override discovered/per-test baselines |
| `timeouts.agent` | `7200` | Seconds per worker call |
| `timeouts.test` | `7200` | Seconds per test or coverage command |
| `timeouts.docker` | `3600` | Seconds per environment operation, shared across its steps |
| `loop.timeout` | `43200` | Seconds for the entire audit; does not limit later coordinator interaction |
| `loop.max_stage_calls` | `20` | Worker budget; environment/execution actions have their own equal-size budget |
| `loop.max_stage_attempts` | `4` | Attempts per role |
| `publication.project` | inferred from Git remote | GitLab HTTPS project URL, useful with SSH aliases |
| `publication.glab_command` | `[glab]` | Authenticated GitLab CLI command |

All times use seconds. Environment and test commands are bounded by the remaining audit budget. Timeouts and attempt counts are separate limits.

## Environment

When absent, the coordinator discovers the runner from repository evidence. Example:

```yaml
environment:
  compose_files: [tests/compose.yaml]
  services: [database]
  runner_service: runner
  runner_workdir: /workspace
  readiness_timeout: 600
  setup:
    - [sh, -lc, "your-project-idempotent-dependency-setup"]
  readiness:
    - [sh, -lc, "your-project-readiness-check"]
```

Commands and image versions must come from the actual project. Use either `compose_files` or an inline `compose` object, never both. The runner must remain running. Source mounts read-only at `/workspace`; build output, caches and reports use `/aaqe-results` or dedicated writable volumes. `{result_dir}` and `AAQE_RESULT_DIR` identify a fresh execution directory.

Additional fields: `external_containers: []` inspects existing containers without owning them; `env_file` names a private environment file. Relative `state_dir` and `env_file` paths are relative to the configuration file. Source, reference and Compose paths refer to the fetched worktree. CLI paths use the invocation directory.

Environment precedence: project YAML, explicit preparation input, Explorer inference. Ownership fixes the Compose definition. Setup runs as user 0 and must be idempotent, noninteractive and independent of generated tests. Failed setup stops the runner; later preparation recovers it. Reproduction replays the saved setup recipe. SSH is read-only inventory, not deployment.

## Test evidence and coverage

No Java, JUnit, XML exporter or specific test framework is required. Rust records argv, exit status, timeout state and logs. Independent Validator checks the actual test, native output and behavioral oracle. Baseline failures, compilation errors, skipped tests and infrastructure failures cannot establish a defect.

Explorer enumerates every production source file in the requested module. Existing and generated tests must together cover at least **80% of executable production lines**, excluding tests, dependencies and generated code. Developer runs the project's native tool and writes a normalized JSON summary alongside its original reports:

```json
{
  "files": [{"path": "src/module.rs", "covered_lines": 80, "executable_lines": 100}],
  "raw_reports": ["native-report.lcov"]
}
```

Rust checks file scope, counts, original-report existence and threshold; Validator records an explicit independent assessment of scope, measurement integrity and meaningful assertions. All must pass before delivery. Missing or below-threshold coverage blocks successful completion. The coordinator can add meaningful tests or repair tooling within its budgets. This does not configure or merge GitLab CI coverage.

## GitLab delivery

Authenticate on the host with `glab auth login --hostname YOUR_GITLAB_HOST` and configure Git commit identity. Authentication stays in Git/glab; AAQE does not save tokens.

When accepted tests exist, finalization creates an immutable test commit, pushes `aaqe-tests/RUN_ID` to the configured remote and creates/reuses an MR targeting `git.branch`. The original `aaqe/RUN_ID` audit worktree remains at its base with the accepted patch for replay. Failed product assertions can be legitimate regression evidence. Coverage below 80%, unavailable measurement or blocked audits produce Draft MRs.

Reports retain the commit, publication status, MR URL and failure reason. Retry without another AI call:

```bash
aaqe publish RUN_ID
```

The command uses the saved run configuration. GitLab permissions, reachable remote and authenticated glab are required only for publication. Non-GitLab repositories can still be audited but publication reports failure. Interrupted or policy-denied runs are not automatically published. Remote publication is separate from audit verdicts.

## Commands and retained output

`aaqe` prompts for a request; `aaqe run "REQUEST"` starts directly. Commands also include `doctor`, `report RUN_ID`, `reproduce RUN_ID [--case TEST_ID]`, `publish RUN_ID`, `env status RUN_ID`, `env shell RUN_ID [--service SERVICE]` and `cleanup RUN_ID`.

Global `--config PATH` and `--state-dir PATH` work before or after the subcommand. Use the same state directory for later operations.

Run directories contain authoritative `harness.json`, human/machine reports, stage history, execution logs, coverage, `tests.diff`, environment ownership/setup and publication journals. New IDs use Shanghai time plus a random suffix. Attach using the printed tmux command; detach with Ctrl+b then d. Successful finish leaves the interactive coordinator available.

`reproduce` verifies the base and accepted patch, restores the saved runner and setup, replays tests and coverage, and preserves original verdicts. `cleanup` removes owned containers and tmux sessions while retaining reports, branch and worktree. Concurrent operations and active-audit cleanup are rejected.

Exit codes: 0 command success; 1 incomplete audit or failed doctor/status; 2 argument/operation error. A completed audit may contain confirmed product defects. Publication failure is reported separately. Reproduction execution records retain individual test outcomes. Files can contain private source and logs; keep the run directory private.

## Development

Rust 1.88+, Git and tmux are required; Docker enables the real integration scenario.

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --example aaqe-test-worker
cargo test --locked --all-targets
cargo test --locked --all-targets real_docker_workflow_reproduction_and_owned_cleanup -- --ignored --nocapture
cargo build --locked --release
cargo package --locked --allow-dirty --target-dir target/package-check
```

Production modules live under `src/`; roles and schemas under `resources/`; integration tests and native subprocess fixtures under `tests/`. Function spacing is maintained manually, with stable rustfmt for standard formatting.

Release tags `vVERSION` build Linux x86_64 musl archives and mandatory SHA-256 files. Publish a Release before recommending the binary installer to users. Source installation works independently.

Test patches may contain tests in any repository path, including inline source tests. Developer and independent Validator enforce the test-only rule. AAQE checks safe paths, the recorded Git base and exact accepted-patch integrity before and after execution and publication.
