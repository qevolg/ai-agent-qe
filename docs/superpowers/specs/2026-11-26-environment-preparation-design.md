# AAQE environment preparation and recoverable scheduling

The InfluxDB audit stopped before generating tests because missing host Maven was classified as an undesignable test, while the harness could only start Docker after accepting a test patch. The selected approach improves AAQE itself, rather than adding a taosX-specific configuration.

## Behavior

- Read-only workers continue to inspect code and propose structured artifacts. The harness owns environment writes, Docker execution, test patches and evidence.
- The coordinator can inspect local tools, Docker, HTTP endpoints and SSH container inventories through bounded, recorded probes. SSH inventories use key authentication or a password named by an environment variable; secrets are not tool arguments or saved records. Supplied hosts are inventory sources, not implicit remote deployment targets.
- A `prepare_environment` tool starts a persistent runner before test development. It accepts project Compose files or a generated inline Compose object, saved under the run directory outside the worktree, plus runner readiness commands. Project configuration takes precedence over inferred settings.
- Failed configuration can be corrected before resources exist. Once a Compose ownership snapshot exists or the environment started, its definition is immutable for this run. Retry the same definition to recover readiness or startup; preserve resources and evidence.
- `blocked_reason` means the test cannot be designed or implemented reliably. Optional `execution_blocked_reason` records missing execution prerequisites without preventing test development. Split independent unit, integration and E2E experiments into separate plans.
- Accepted tests can be executed again through `execute_tests`, optionally selecting test IDs, without another Developer call. Each test can supply only its necessary baseline; legacy global baseline configuration remains supported. A failing scoped baseline blocks that test and does not prevent independent tests from running.
- Validator may examine blocked plans without a Developer artifact. Only matching complete execution, a healthy applicable baseline and independent validation can confirm or rule out reproduction. No execution remains blocked.
- Status exposes actual stage availability and recovery actions. Rejected dispatches are recorded separately from launched attempts. Tool failures identify their gate and the next available actions. Reports show candidate-specific blockers, environment actions, and generated versus executed tests.

## Compatibility and boundaries

Existing project configurations, stage artifacts and saved runs remain readable. New schema fields are optional. Existing file-based Compose execution and automatic execution after Developer acceptance remain available. No product names, versions, hosts or credentials enter generic code or prompts. Existing production patch protection and run-owned cleanup remain enforced. No automatic CI execution, publication or Feishu writes are added.

The environment backend remains local Docker Compose; remote machines supply read-only inventory. A remote Docker deployment backend and automatic migration of a finished audit into a new running audit are outside this change.

## Alternatives

Project-only configuration would unblock one task but leave the generic lifecycle gap. Giving agents unrestricted Docker or shell access would bypass existing ownership and evidence boundaries. The chosen design adds harness operations and preserves read-only workers.

## Acceptance

Regression coverage must demonstrate preparation before Developer, a missing-tool execution blocker allowing test generation, generated Compose without worktree changes, startup/readiness retries with immutable ownership, independent baseline failures, blocked-only validation without fabricated confirmation, structured dispatch recovery, saved probe evidence, and compatibility with existing runs. Run formatting, linting and all local tests; exercise real Docker when available and report its result separately from deterministic harness tests.
