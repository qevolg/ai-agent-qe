# v0.1 validation

The initial release was checked in a Linux execution environment with Python 3.12.3, Git 2.43.0, Codex CLI 0.172.0, Docker Engine 29.6.1 and Compose v5.3.0.

## Automated checks

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

## Actual model checks

Actual independent `codex exec` calls completed Code Explorer, Bug Hunter and Test Designer on the generic example. The first Test Developer request exposed that strict structured output rejects JSON Schema `uniqueItems`. The model-facing schema now omits this keyword while Python still validates uniqueness; a focused regression test checks both behaviors. The initial run correctly saved a blocked report. A separate compatibility run checks Test Developer and Validator with the corrected schema.

## Limits

Native Windows paths and process termination have not been accepted on Windows. Python 3.11/3.13 coverage is configured in GitHub Actions and is not implied by the local Python 3.12 results. GitHub CI status must be checked on the published commit.

No real business product audit is claimed by these checks. Projects must supply their own behavioral references, runnable environments and version-matched software. Reproduction and cleanup of the same run should be performed sequentially.

An attempted independent agent review was unavailable because its service rejected authentication. The implementation received local inspection and regression fixes; an independent review result is not claimed.
