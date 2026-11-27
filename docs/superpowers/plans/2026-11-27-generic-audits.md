# Generic audits and Linux distribution

> Execute inline using executing-plans and test-driven-development. This plan implements the user's approved discussion from this session.

**Goal:** Make AAQE framework independent, require measured module line coverage of at least 80%, publish accepted tests as GitLab MRs, and simplify Linux installation and documentation.

**Architecture:** Rust retains process evidence and coverage measurements; independent Validator assesses assertions. A separate finalization step publishes an immutable accepted test patch through Git and the authenticated GitLab CLI. Release installation downloads a verified Linux x86_64 binary.

**Constraints:** Linux only; Rust 1.88 minimum; default model gpt-6.1-sol; agent/test 7200 seconds, Docker 3600, readiness 600, audit 43200. No project GitLab CI coverage integration. Function spacing is adjusted manually with stable rustfmt, without additional automation. No new framework dependency checks.

## Tasks

- [x] 1. Generic execution evidence and defaults. Update src/config.rs, workflow.rs, stage schemas, embedded skills and execution summaries. Tests must accept execution without XML and reject interrupted, fabricated or unrelated evidence. Baseline failure must remain blocking. Preserve historical XML records when reading reports.
- [x] 2. Coverage. Add src/guards/coverage.rs and collect a Developer-provided coverage command in the owned runner. Read a fresh normalized JSON report with raw report references and per-file executable/covered line counts. Reject unsafe paths, duplicate/missing files, impossible totals and unexecuted measurements. Check every declared production file and require >=80%; missing or failed measurement is blocked. Independent Validator checks measurement scope and meaningful assertions. Test 79%, 80%, missing/invalid reports and replay.
- [x] 3. Publication. Add src/runtime/publication.rs and a publish retry CLI. Verify the accepted patch against the recorded base, commit exactly that tree, push a dedicated run branch, query/reuse/create an MR with glab, and journal each stage. Use GitLab HTTPS API via glab with inherited authentication, no saved tokens. Low or missing coverage produces Draft. Preserve audit branch/base for reproduction and record publication failure separately. Tests use a local bare Git remote and a deterministic GitLab API double to verify push SHA, retry, draft and production protection.
- [x] 4. Distribution and docs. Add install.sh and x86_64 musl Release workflow with mandatory checksums. Exercise the installer against a local Release fixture. Rewrite both READMEs to concise installation, usage, minimal config and output documentation; move detailed configuration/development/reference material to docs. Update aaqe.yaml.example. Manually space adjacent Rust functions/methods throughout src and tests.
- [x] 5. Verify and publish. Run fmt, clippy, full Rust tests, real Docker workflow, release build, package and installer checks. Review complete diff, commit task changes and push to the existing origin; verify remote SHA. Creating a GitHub PR for AAQE itself is outside this request.

## Review focus

- Coverage counts must describe the current scoped production files and have original measurement evidence; no zero-denominator success.
- Verdicts must never equate a failed process with a demonstrated product defect.
- Publication must not change the base or HEAD of the retained audit worktree, include unrelated files, or publish on denied/interrupted audits.
- Retried publication must recover after push or MR creation succeeded but the local journal was not saved.
- Installation must reject absent or incorrect checksums and preserve an existing executable on failure.

## Progress

- Baseline: full test suite passed before implementation.
- Generic evidence accepts executions without framework reports; missing assessment, interrupted execution, unhealthy baseline and fabricated paths block confirmation. Historical reports remain readable.
- Native coverage summary checks 79%, 80%, scope completeness, unsafe paths, impossible/duplicate counts and original report existence. Successful delivery additionally requires the current independent Validator to approve scope, measurement and meaningful assertions.
- Publication tests use a real local bare Git remote and a deterministic GitLab API double. They verify exact test/ignored-fixture trees, production and HEAD protection, actual MR identity/SHA, Draft coverage failures and cross-second recovery after journal loss.
- Installer tests cover valid archives, paths containing spaces, invalid/missing checksums and preservation of an existing executable.
- Independent code review completed. Fixed coverage approval, lost-journal recovery, MR identity and ignored accepted fixture findings; no actionable findings remain.
- Stable fmt and clippy passed. Full suite: 128 passed, zero failed, one opt-in Docker test ignored. Rust 1.88 all-targets check, release build, package verification and shell syntax passed.
- Final real Docker replay passed both Compose and early inline environment scenarios, including measured LLVM line coverage, coverage replay, preparation failure retention and owned cleanup (660.97 seconds).
- Linux x86_64 musl Release build passed under Rust 1.88 in a disposable container. The static binary ran on the host and installed successfully from a checksummed local Release archive through install.sh.
- Publication proceeds as one commit pushed to the existing origin/main, followed by remote SHA verification. No force push or GitHub PR is required by the user's instruction.
- GitHub Actions, a Release tag and live audited-project GitLab publication are not claimed as executed. Binary installation requires the first published Release; source installation is available.
