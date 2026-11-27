# 验证记录

## 中文报告、上海时间编号与双语文档（2026-11-27）

### 验证摘要

| 验证对象／检查项 | 验证结论 | 简要说明 |
| --- | --- | --- |
| 最新代码复核 | 已完成 | 从远程 `main` 的 `03ae3a1` 开始；保留快照重建、运行互斥和报告错误下的清理语义 |
| 运行编号与时间 | 通过 | `YYYYMMDDHHmmss-xxxxxxxx`、八位随机后缀、Asia/Shanghai；覆盖跨日、跨年及旧编号读取 |
| tmux 主窗口 | 通过 | 会话为 `aaqe-<RUN_ID>`；验证结束后保留、继续交互、归属检查与重复清理 |
| 中文验证报告 | 通过 | 开头汇总表及参考报告的八个主题；五种判定、表格转义、未完成分析、历史与复现记录分别处理 |
| 普通测试（Rust 1.96.0） | 通过 | 118 项通过、零失败；1 项 Docker 测试默认忽略并单独执行 |
| 最低支持版本（Rust 1.88.0） | 通过 | 同样 118 项通过、零失败；严格 Clippy、Release 与发布包编译均通过 |
| 真实 Docker 流程 | 通过 | 1 项显式启用测试同时覆盖文件 Compose 和 early inline Compose，模型输出使用确定性夹具 |
| 文档与配置 | 通过 | 9 个帮助入口及多层 help；19 个相对链接；两种语言的 Mermaid 架构图；配置示例实际加载并通过 doctor |
| 独立只读审查 | 通过 | 审查 `03ae3a1..81fa332`，Critical、Important、Minor 均无；真实模型质量、其他宿主平台和远程 CI 另列验证边界 |
| MIT 与发布包 | 通过 | Cargo 声明 `license = "MIT"`；发布包包含 LICENSE、双语 README、配置模板和运行参考文档 |

### 验证结论

本轮变更在下述本地环境和执行范围内通过验证。运行状态 `completed` 表示审计流程完成，候选可以仍为受阻或已确认缺陷；报告不将其计为产品验收通过。汇总只使用当前候选及有效判定，历史重试和独立复现不重复累计。缺少 Hunter 分析产物与完成分析后没有候选使用不同说明。

### 验证范围与进度

新编号同时用于运行目录、`aaqe/<RUN_ID>` 分支、`aaqe-<RUN_ID>` Docker 项目和主 tmux 会话；时间记录带 `+08:00`。编号和 `created_at` 共享同一次时钟读取。历史编号、时间值、Python 版运行记录和旧 tmux 会话继续按原值读取与清理。

报告保留现有 JSON 结构与状态枚举，Markdown 和新阶段自然语言字段使用简体中文。原始日志、错误、命令、标识符和历史自由文本保留原值。快照重建、基线/目标测试证据、清理状态刷新及复现追加记录均有回归覆盖。

默认英文 README 与中文 README 按相同结构说明安装、快速开始、架构、全部公开 CLI 参数、配置字段与默认值、路径和优先级、报告及资源生命周期。详细 MCP/环境说明移入 `docs/runtime.md`；根目录新增 `aaqe.yaml.example`。配置仍为可选，只从调用目录或显式 `--config` 读取。

### 环境与证据

本地使用 Linux、Rust 1.96.0 / 1.88.0、Codex CLI 0.172.0、tmux 3.4、Docker Engine 29.6.1、Compose v5.3.0、Python 3 及 Python 3.12 slim 容器。开发与验证在独立 Git Worktree 和独立 Cargo 构建目录中执行。

可重跑的主要检查：

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --locked --release
cargo package --locked --allow-dirty --target-dir target/package-check
cargo +1.88.0 clippy --locked --all-targets -- -D warnings
cargo +1.88.0 test --locked --all-targets
cargo +1.88.0 build --locked --release
cargo +1.88.0 package --locked --allow-dirty --target-dir target/msrv-package-check
cargo test --locked --test workflow real_docker_workflow_reproduction_and_owned_cleanup -- --ignored --nocapture
```

另执行 `git diff --check`、所有公开命令的 `--help`、多层 `help env shell`、`aaqe --config aaqe.yaml.example doctor`、Markdown 相对链接检查、Mermaid 解析和 `cargo package --list --allow-dirty`。

### 验证结果

普通测试使用真实临时 Git 仓库、真实 tmux 和确定性的 Codex/Docker 边界夹具。统计按顶层测试用例计数，子进程打印的嵌套测试结果不重复相加。专项报告测试覆盖所有五种 verdict、带竖线/换行的摘要单元格、受阻原因、当前执行与历史轮次、快照派生文件缺失或损坏后的重建。

单独启用的 Docker 用例运行真实容器、只读 `/workspace`、可写结果目录、健康基线、已知失败的目标断言及精确 JUnit 证据，并验证选中重跑、测试补丁保护、离线依赖安装/超时恢复、恢复后复现及本轮资源清理。两种 Compose 场景均通过。

### 问题记录

验证阶段未发现阻断交付的问题。独立只读审查结论为可以合入，无 Critical、Important 或 Minor 发现；审查者没有重复执行实施方的测试。编号与创建时间的取时已统一。MIT 授权使用标准文本，版权署名为当前仓库所有者 `qevolg`。

### 验证路径调整

共享原检出目录的旧 Cargo 构建缓存曾使编译器读取过期接口；切换至独立 Worktree 的独立构建目录后，专项与完整检查均通过。该过程未更改产品依赖或测试结论。

### 验证限制

本轮没有执行新的真实模型业务产品审计；确定性模型夹具验证的是调度、执行、持久化与资源管理。自然语言采用中文通过提示词约束，历史文本与原始证据不自动翻译。本次没有新增原生 Windows 或 macOS 支持，运行范围仍为 Linux／WSL2。本地检查通过与远程 CI 状态分别记录，不能据此认定 GitHub Actions 已通过。

### 环境保留与再次验证

本轮回归使用的临时容器和 tmux 资源由测试清理。实际审计仍保留环境，用户可按报告中的命令 `reproduce`、`env status`、`env shell` 与 `cleanup` 操作；清理保留报告、日志、分支和 Worktree。

以下为既有验证历史，保留当时的结果、工具版本和范围。

## Container test dependency recovery — 2026-11-27

AAQE and its control tools remain host processes. Project dependencies, builds,
fixtures and tests execute in the owned Docker runner. Setup recipes install as
container user 0 before health waiting/readiness; coordinator repairs need no
operator installation approval. Tools/endpoint MCP probes default to the runner.
Recipes, process evidence and recovery state are saved for retry and reproduction.

Final local validation used Rust 1.96.0 and minimum-supported Rust 1.88.0,
Docker Engine 29.6.1, Compose v5.3.0, tmux 3.4 and the Python 3.12 slim image.

| Check | Result |
|---|---|
| Formatting and `git diff --check` | Passed |
| Strict Clippy, all targets | Passed |
| Complete ordinary suite, Rust 1.96.0 | 103 passed, zero failures; one opt-in Docker test ignored |
| Complete ordinary suite, Rust 1.88.0 | 103 passed, zero failures; one opt-in Docker test ignored |
| Opt-in real Docker workflow | Passed, file Compose and early inline Compose |
| Release build | Passed |
| Cargo package verification | Passed; packaged crate compiled |

The real Docker fixture deliberately starts without a required Python package,
records failed readiness, probes Python in the unready runner, creates an offline
wheel and installs it using actual pip through the MCP setup operation. It then
executes baseline/target tests, removes the runner, and reproduces after replaying
the saved installation recipe in a new runner. Read-only source and writable
result mounts, selected reruns, evidence preservation and owned cleanup remain
covered. The fixture also executes a delayed installer under a short deadline,
checks that its runner stops, and verifies that its delayed marker is absent after
subsequent recovery/reproduction. No package mirror is needed for this regression.

Independent read-only review found three Important issues: preparation retries
could retain an obsolete Validator artifact, timed-out installers could survive
the host Docker client, and a new fixture instruction described standalone replay
preparation as if ordinary execution consumed it. The first two were reproduced
by failing harness and real Docker tests, then corrected and verified with the
green suites above. Every actual preparation retry invalidates execution and
validation; a persisted installer marker enforces owned-runner stop recovery.
A further regression covers failed stop blocking probes/tests until recovery.
The Developer instruction now states that normal test commands include required
fixture setup, with reproduction_commands reserved for additional CLI replay
preparation. Review reported no Critical or Minor findings.

The retained boundaries are project-provided Compose trust and existing general
process capture for setup (probe output remains bounded). Complete replacement
recipes must retain semantic prerequisites and remain independent of generated
tests; arbitrary argv cannot establish those properties. Standalone reproduction
preserves the original audit verdict and appends its own execution history.
Readiness and installation recipes must tolerate replay.

These checks use deterministic model fixtures, not a new actual-model business
product audit. Local results do not establish a remote CI pass.

## Environment preparation and recoverable scheduling — 2026-11-26

The harness supports recorded environment probes, inline Compose and runner readiness,
early preparation before test generation, selected execution rounds, scoped baselines
and recovery-aware dispatch. Reports distinguish each candidate's design and runtime
blockers, generated tests, current execution and historical attempts. Rejected
dispatches are separate from launched workers. See README for API examples and exact
limits; the environment backend remains local Docker Compose.

Local validation used Linux, Rust 1.96.0 and 1.88.0, Docker Engine 29.6.1,
Compose v5.3.0, tmux 3.4 and Python 3.12.3. Each complete ordinary suite passed
**94 tests, zero failures, with one opt-in Docker test ignored**. These counts
include two new report regressions; four isolated child-process checks print nested
results and are not additional top-level tests.

The opt-in real Docker workflow passed separately: one test runs both the existing
file Compose scenario and an inline Compose scenario prepared before Explorer and
Developer. It checks real Docker inventory, readiness in the persistent runner,
read-only source mounts (write rejected with EROFS), writable result mounts,
healthy baseline and a known failing target assertion with complete JUnit evidence,
a Developer retry, selected execution without a new Developer call, reproduction,
and explicit owned Docker/tmux cleanup including repeated cleanup. Both scenarios
use the deterministic model boundary; they do not establish actual model reasoning
or a business product audit. The inline scenario has no Compose file in the product
worktree, and the generated Compose and readiness evidence stay in the run directory.

Formatting, strict Clippy and Release builds passed on both Rust 1.96.0 and 1.88.0;
Cargo package verification passed with the commands in `.github/workflows/tests.yml`.
The Rust 1.88.0 Clippy check required replacing an unnecessary owned string lookup
with an equivalent borrowed lookup; the affected selected-execution regression passed. Cargo package emits
the existing missing-license metadata warning. These are local results; this change
has not established a new remote CI result. SSH/password behavior uses isolated
regressions; no live remote host or credentials were used. Prompt/Skill instructions
received static inspection and fixture integration, not a new actual-model audit.

Independent task reviews verified repairs for tool-specific SSH/Go version arguments,
bounded probe output capture, and stopping execution when baseline patch restoration
requires recovery. The final independent whole-branch review found no Critical or
Important issue and assessed the change as ready to merge. Its sole Minor finding
was the README raw-evidence path: corrected to `executions/<attempt>/`, matching the
runtime's UUID directory directly under `executions`. This final documentation-only
correction passed `git diff --check`; no additional runtime tests were needed.

## TUI exit polling CI repair — 2026-11-26

GitHub Actions run [45730201256](https://github.com/qevolg/ai-agent-qe/actions/runs/45730201256)
failed the Rust 1.88.0 TUI timeout regression. Its Docker job passed, while the
stable job was cancelled by matrix fail-fast. The worker could write its exit
result and finish while the parent was querying tmux, causing the parent to
report a dead pane instead of the recorded timeout exit code 124.

When a pane is dead or missing, the parent now rechecks the completion markers
before reporting an unexpected exit. Existing validation then reads either the
acknowledged audit status or the recorded process result. A deterministic test
uses real tmux and holds a pane query until the worker has exited, covering
both exit code 23 and an acknowledged completed audit. Both cases failed before
the fix and passed afterwards. Complete ordinary suites and strict Clippy passed
locally on Rust 1.88.0 and 1.100.0: 68 tests passed per suite, with the opt-in
Docker test ignored. Formatting, Release build and Cargo package verification
also passed. Independent review found no actionable issues.

## Retained coordinator TUI — 2026-11-26

Each new audit now retains one interactive Codex TUI session named
`aaqe-<12 hex characters>`. Stage attempts run as fresh background `codex exec`
processes without tmux. A guarded MCP finish acknowledgement completes the audit
and releases the outer CLI while the TUI stays interactive.

- The ordinary suite passed: 67 tests, with the opt-in Docker test ignored.
  Regressions check terminal stdin/stdout/stderr, post-report input after the
  audit timeout has elapsed, independent attempts without stage tmux metadata,
  live stage logs, and cleanup of both new and legacy session names.
- The real Docker workflow passed with deterministic coordinator/stage model
  fixtures, all five roles, a Developer retry, actual baseline/target execution,
  reproduction and owned resource cleanup.
- Strict Clippy passed on the default toolchain and Rust 1.100.0. The declared
  minimum Rust 1.88.0 passed `cargo check --locked --all-targets`.
- Formatting, Release build and Cargo package verification passed.
- A bounded actual Codex 0.172.0 TUI check called the private MCP `status` and
  `finish` tools, intentionally finished blocked with reason
  `interactive TUI transport smoke complete`, and responded to a subsequent
  terminal message after the outer CLI returned. Its test session was cleaned.
  This validates interactive transport, not an actual-model product audit.

The actual-model test used a separate temporary Codex home with the scratch
repository trusted. The user's global trust settings were not changed. Native
Codex login/folder-trust prompts remain available through the retained TUI; a
first run in an untrusted repository may require the user's input there.

Independent review exposed cancellation leaks when the coordinator's direct
child exited before its descendants, or killed MCP before MCP could cancel its
stage. New regressions reproduced both. TUI exit now cleans its remaining process
group; stages have independent AAQE supervisors that watch their original MCP
parent and cancel the stage group on parent loss. Advisory locks synchronize
shutdown with live MCP/stage supervisors and release automatically after a hard
kill, avoiding stale-file waits and overlapping harness finalization.
The independent follow-up review reproduced the formerly leaking scenario with
the fixed implementation and found no remaining actionable issues.

The sections below record earlier implementations and their verification; their
per-stage tmux descriptions are historical.

## Codex coordinator agent loop — 2026-11-25

The fixed scheduler has been replaced by one persistent Codex coordinator with a
private stdio MCP harness. Every stage invocation starts a separate retained tmux
session. Local verification used Linux, Rust 1.88.0 / 1.96.0 / 1.100.0, tmux 3.4,
Codex CLI 0.172.0, Docker Engine 29.6.1 and Compose v5.3.0.

- Formatting, strict Clippy and the complete ordinary suite passed; 65 tests passed
  with one opt-in Docker test ignored. Rust 1.88.0 and 1.100.0 were checked too.
- The real Docker integration passed with a subprocess model fixture acting as
  coordinator and workers. The fixture launches the actual private MCP server,
  dispatches all five roles, retries Developer in a seventh distinct retained
  session (coordinator plus six stage calls), reruns baseline and failing target,
  reproduces with a fresh execution directory, then cleans owned resources.
- Release build and Cargo package verification passed. New roles/resources and the
  native MCP entrypoint are embedded in the ordinary executable.
- A bounded actual Codex check successfully called the native MCP `status` and
  `finish` tools through its own tool loop and tmux transport. It intentionally
  finished blocked with the reason `native Codex MCP transport smoke complete`.
  This establishes actual-model tool transport, not an actual-model product audit.

New regressions cover stage prerequisites and explicit completion, retries and
upstream invalidation, immutable attempt evidence, current evidence paths, call
and per-role budgets, protected patch replacement/rollback, baseline restoration
after coordinator loss, MCP protocol/errors, nested ownership-checked cleanup,
symlink rejection, and coordinator cancellation/timeout with independent stage
processes. Earlier saved-run, reproduction, production-boundary and JUnit checks
remain covered. A final message without a successful `finish` cannot mark an audit
completed. A completed audit may include blocked candidates when evidence gates
reject their requested conclusions.

The actual Codex check first exposed a default MCP approval restriction under
`approval_policy="never"`. The coordinator now explicitly approves only its
private harness server's tools using the Codex server configuration. Worker
invocations do not receive this server. A fixture regression checks this setting.

The independent whole-branch review found two Important issues: an interrupted
patch replacement/invalidation could disagree with the committed snapshot, and
an Explorer retry could reference generated files or lines removed by invalidation.
Four regressions failed before the fixes and passed after them. A persisted patch
transition journal now recovers the committed patch and active artifact aliases;
a fifth regression covers interruption after the patch has already been restored.
Explorer references must name regular files and valid lines at the recorded base.
The review reported no Critical or Minor findings. Its execution-validation and
audit-quality exclusions are covered only to the extent stated above.

One run retains one owned Compose snapshot; changing the discovered environment
after startup requires a new audit. Stage ordering is a coordinator decision,
while dependency/evidence checks remain Rust responsibilities. Runtime remains
Linux/WSL2 with `codex exec` in tmux, without an interactive Codex TUI. Local
verification does not establish a new remote CI result.

## Rust 1.100 CI repair — 2026-11-25

GitHub Actions run [45588021885](https://github.com/qevolg/ai-agent-qe/actions/runs/45588021885)
failed in stable Clippy on Rust 1.100.0. The new
`chunks_exact_to_as_chunks` lint rejected UTF-16 decoding under `-D warnings`.
The decoder now uses `as_chunks::<2>()`, available in the minimum supported
Rust 1.88, with the existing even-length check and decoding behavior preserved.

Running the full suite with Rust 1.100 also exposed a worker lookup assumption:
Cargo now places integration test executables under
`{profile}/build/{package}/{hash}/out`, rather than `{profile}/deps`. Worker
lookup now recognizes both layouts and launches the ordinary `{profile}/aaqe`
binary. The affected transport test failed before the fix and passed after it.

After both corrections, formatting, strict Clippy and the complete ordinary
test suite passed locally on Rust 1.88.0 and 1.100.0. Each suite had 49 passing
tests and one opt-in Docker test ignored. The original failed Actions run's
Docker job passed; its minimum-version job was cancelled by matrix fail-fast.

## Rust rewrite — 2026-11-25

The native Rust implementation was checked on Linux with Rust/Cargo 1.96.0,
the declared minimum Rust 1.88.0, Git 2.43.0, tmux 3.4, Codex CLI 0.172.0,
Docker Engine 29.6.1 and Compose v5.3.0. Before replacing the implementation,
the Python baseline at `3cb983e1827e510e601df64362b22a188bde1895` passed its
62 ordinary tests with the opt-in Docker test skipped.

### Local checks

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed |
| `cargo test --locked --all-targets` | 49 passed; 1 opt-in Docker test ignored |
| `cargo +1.88.0 test --locked --all-targets --target-dir target/msrv` | 49 passed; 1 opt-in Docker test ignored |
| `cargo test --locked --test workflow real_docker_workflow_reproduction_and_owned_cleanup -- --ignored --nocapture` | 1 passed using real Git, tmux and Docker |
| `cargo build --release --locked` | Passed |
| `cargo package --locked --allow-dirty --target-dir target/package-check` | Passed; packaged crate compiled successfully |
| `cargo install --path . --locked --root <temporary-directory>` | Installed binary passed a smoke check outside the source checkout |
| `target/release/aaqe doctor` | Git, Codex, tmux, Docker daemon and Compose available |

The ordinary tests cover configuration validation and YAML merge keys, exact
fetched Git revisions and isolation from caller changes, private atomic state,
artifact identities, one-shot model calls, embedded resources, live tmux logs,
argument/environment transport, ownership checks, bounded control commands,
timeouts, process-group termination, interruption and partial reports. Patch
and evidence checks cover production boundaries, Rust test modules, traversal,
links, Python-compatible path patterns, exact JUnit identities, declared XML
encodings, internal entities, skipped/error/duplicate cases and process/report
disagreement. Interrupting baseline execution restores the generated patch and
still collects environment logs.

Saved `run.json` and `report.json` fixtures were generated by the original
Python implementation. The Rust CLI reads and refreshes these records and
performs idempotent cleanup. Separate workflow and real Docker checks verify
reproduction of saved tests without model calls while preserving the original
evidence and verdicts. Integral numeric values such as timeout `3.0` retain
JSON Schema integer semantics in both YAML configuration and saved JSON.

The real Docker case exercises all five model stages, a passing baseline, a
failing regression with complete JUnit evidence, the confirmed verdict,
retention, reproduction in a fresh directory and repeated explicit cleanup of
the owned environment and five tmux sessions. Its model output comes from the
deterministic `tests/fixtures/scripted_codex.py` boundary fixture; this checks
native orchestration and contracts without claiming a new actual-model audit.

The installation smoke check ran Explorer and an empty-candidate Hunter stage,
then cleanup, from outside the checkout. The installed binary's `PATH`
contained only Git, tmux and basic shell tools, with no Python executable.
Roles, Skills and schemas were available from the native executable. The
minimal example product and opt-in Docker model fixture still use Python;
the AAQE runtime does not depend on it.

An independent source comparison identified five compatibility regressions
during migration: integral reference lines, unbounded tmux control calls,
declared non-UTF-8 XML, internal XML entities and YAML merge keys. Each was
corrected and covered by regression checks. The reviewer found no unresolved
actionable regression in the reviewed paths. The final ordinary suites and
Docker/installation checks were rerun after those corrections.

Package verification uses a separate target directory: verification of an
unpacked crate must not replace the working tree's build fingerprints. The
Cargo package allowlist excludes local Python build artifacts and caches.

### External checks and limits

GitHub Actions now checks Rust 1.88.0 and stable, with a separate real Docker
job. The results above are local; no result from the new remote Rust workflow
is claimed here. The historical Python CI result below applies only to that
earlier implementation. Runtime support remains Linux and WSL2; native Windows
and macOS were not validated in this migration. No new actual-model business
product audit was performed.

## Historical Python validation

The initial release was checked in a Linux execution environment with Python 3.12.3, Git 2.43.0, Codex CLI 0.172.0, Docker Engine 29.6.1 and Compose v5.3.0.

### Automated checks

| Check | Result |
|---|---|
| `python -m pytest -q` | 43 passed; the opt-in Docker case skipped |
| `AAQE_DOCKER_TESTS=1 python -m pytest -q tests/test_docker_smoke.py` | 1 passed with actual Docker containers |
| `python -m ruff check .` | Passed |
| `python -m ruff format --check .` | Passed |
| `python -m compileall -q src` | Passed |
| `python -m build` | Wheel and source distribution built |
| Wheel installed into a separate virtual environment | CLI and five packaged roles/Skills available outside the source checkout; a structured subprocess call passed |
| `aaqe doctor` | Git, Codex, Docker daemon and Compose available |

The Docker integration case creates a temporary Git remote and project, fetches main, applies a test-only patch, runs the baseline and a known failing assertion in a real container, retains the environment, reproduces using a fresh result directory, and explicitly cleans its owned fixture resources. Its five model responses are deterministic subprocess fixtures. They exercise orchestration and contracts without claiming real model analysis.

Regression checks cover production edits, Rust inline test boundaries, path traversal, symlinks, unknown source references, fabricated evidence paths, missing/skipped/ambiguous JUnit results, model failures, changed base commits and modified or missing Compose snapshots. Reproduction and cleanup refresh reports while preserving original execution evidence and verdicts.

### Actual model checks

Actual independent `codex exec` calls completed Code Explorer, Bug Hunter and Test Designer on the generic example. The first Test Developer request exposed that strict structured output rejects JSON Schema `uniqueItems`. The model-facing schema now omits this keyword while Python still validates uniqueness; a focused regression test checks both behaviors. The initial run correctly saved a blocked report.

With the corrected schema, a separate actual Test Developer call produced a valid test-only patch. Docker execution passed the original baseline (exit 0) and ran the new negative-input regression (exit 1) with complete, precisely matched JUnit evidence. The independent actual Validator exceeded the explicit 180-second smoke-test timeout; no actual-model final confirmation is claimed. The normal configurable agent timeout defaults to 1800 seconds. This temporary compatibility environment was explicitly cleaned after the check.

### Published CI

GitHub Actions run [45572301164](https://github.com/qevolg/ai-agent-qe/actions/runs/45572301164) passed on implementation commit `7ca0424d7bf1752f84e1c1309690337ce1ce1d05`: Python 3.11, 3.12 and 3.13 unit/lint/format/build jobs and the real Docker integration job all completed successfully.

### Limits

Native Windows paths and process termination have not been accepted on Windows. The Python version matrix above ran on Ubuntu; these results do not establish native Windows support.

No real business product audit is claimed by these checks. Projects must supply their own behavioral references, runnable environments and version-matched software. Reproduction and cleanup of the same run should be performed sequentially.

An attempted independent agent review was unavailable because its service rejected authentication. The implementation received local inspection and regression fixes; an independent review result is not claimed.

### tmux launch update

The tmux launch change was checked locally on Linux with Python 3.12.3 and tmux 3.4:

- `python -m pytest -q`: 62 passed, with the opt-in Docker case skipped.
- `AAQE_DOCKER_TESTS=1 python -m pytest -q tests/test_docker_smoke.py`: 1 passed using real tmux sessions and Docker containers with deterministic model fixtures.
- Ruff lint and formatting checks passed; wheel and source distribution built successfully.
- The built wheel was installed into a temporary directory and exercised from outside the source checkout. Packaged roles/Skills, tmux worker startup and structured stage output passed.
- `aaqe doctor` found Git, Codex, tmux, Docker Engine and Compose.
- A real `codex exec` call returned the requested structured output through the tmux worker, and its output was visible in the retained pane. This checks transport and structured output; it does not establish a real product audit.

Real tmux regression cases cover live logs before process exit, retained panes, attach announcements, argument quoting, current caller environment without stale server variables, nonzero exits, missing executables, timeouts with child-process termination, AAQE interruption including progress callbacks, and deletion of an active session. Test-owned tmux sessions are explicitly removed after each check. The updated GitHub Actions configuration installs tmux; this local verification does not claim a new remote CI run.

Stage 7 cleanup is covered with real tmux sessions: only the selected run's recorded sessions are removed after checking their stage-directory ownership; another run's session remains. Cleanup without Docker, repeated cleanup, invalid targets, ownership mismatch, active audits and Docker failure after tmux cleanup are also covered. Stage logs and evidence remain, and `tmux_cleaned_at` is saved in both the run record and report. The Docker integration invokes CLI cleanup and checks removal of all five stage sessions together with the owned Docker environment. An independent read-only review found no actionable issues in this cleanup change.
