# Container test dependencies

AAQE, Git, tmux, Codex and Docker/Compose continue to run on the host. The audited project's build tools, package installation, services, fixtures and tests run in Docker. Ordinary missing dependencies must be installed automatically without asking the operator to enter the container or approve an installation.

The selected approach extends the existing Compose harness. `environment.setup` declares idempotent argv commands, executed as container user `0` after Compose startup and before readiness. The coordinator can append commands using `prepare_environment(setup: [...])` after startup, without changing the immutable Compose snapshot. Accepted repair recipes and per-command process evidence survive failure and are replayed by preparation and reproduction. Commands run only in the owned runner. They share the preparation deadline and existing action budget.

Tools and endpoint probes default to the runner at the MCP boundary. Explicit host probes remain available for control-plane diagnostics; Docker and SSH inventory remain host operations. Probe evidence identifies the execution target and service. An unprepared runner yields a preparation requirement, never a silent host fallback.

The coordinator infers versions from repository evidence, chooses an appropriate image, installs missing dependencies, verifies readiness, and retries affected tests. Dependency failures are execution prerequisites; product assertions remain evidence. Permanent network/authentication/version failures or exhausted budgets produce a concrete blocked report. Installation commands must be idempotent and independent of generated tests because baselines withdraw the test patch.

Installing dependencies invalidates current execution and Validator conclusions while preserving historical attempts. A readiness failure still permits runner probes and further setup. Reproduction reruns the saved setup before test-specific preparation. Existing configurations and saved runs without setup remain readable.

Alternatives considered: documentation alone leaves the host-probe and recovery gaps; containerizing AAQE exceeds the explicitly confirmed scope. The chosen extension preserves the existing host runtime and test evidence gates.

Acceptance includes setup ordering and failure recovery, immutable snapshot reuse, runner-scoped probes, append validation, stale verdict invalidation, replay, and a real Docker installation of a dependency absent from the runner. No new actual-model product audit or remote CI pass is implied by deterministic fixtures.
