from .state import now, read_json, write_json


def refresh_report(directory, state):
    previous = read_json(directory / "report.json")
    artifacts = {path.stem: read_json(path) for path in (directory / "artifacts").glob("*.json")}
    return generate_report(directory, state, artifacts, previous["execution"], previous["verdicts"])


def generate_report(directory, state, artifacts, execution, verdicts):
    candidates = artifacts.get("bug-hunter", {}).get("candidates", [])
    by_candidate = {verdict["candidate_id"]: verdict for verdict in verdicts}
    for candidate in candidates:
        by_candidate.setdefault(
            candidate["id"],
            {
                "candidate_id": candidate["id"],
                "test_ids": [],
                "status": "blocked",
                "rationale": state.get("error") or "No complete validation was available",
                "evidence": [],
            },
        )
    document = {
        "schema_version": "1.0",
        "run_id": state["id"],
        "generated_at": now(),
        "prompt": state["prompt"],
        "status": state["status"],
        "error": state.get("error"),
        "workspace": state.get("workspace"),
        "stages": state["stages"],
        "scope": artifacts.get("code-explorer", {}).get("scope", []),
        "uncertainties": artifacts.get("code-explorer", {}).get("uncertainties", []),
        "excluded_areas": artifacts.get("bug-hunter", {}).get("excluded_areas", []),
        "candidates": candidates,
        "verdicts": list(by_candidate.values()),
        "execution": execution,
        "environment": state.get("environment"),
        "environment_retained": state.get("environment_retained", False),
        "environment_cleaned_at": state.get("environment_cleaned_at"),
        "tmux_cleaned_at": state.get("tmux_cleaned_at"),
        "reproductions": state.get("reproductions", []),
    }
    write_json(directory / "report.json", document)
    lines = [
        "# AI Agent QE report",
        "",
        f"Run: `{state['id']}`",
        "",
        f"Status: **{state['status']}**",
        "",
        "## Request",
        "",
        state["prompt"],
        "",
        "## Code and scope",
        "",
    ]
    if state.get("workspace"):
        lines += [
            f"Base commit: `{state['workspace']['commit']}`",
            "",
            f"Branch: `{state['workspace']['branch']}`",
            "",
            f"Worktree: `{state['workspace']['path']}`",
            "",
        ]
    lines += ["- " + path for path in document["scope"]]
    lines += ["", "## Findings", ""]
    if not candidates:
        lines += [
            "No candidates were produced in the examined scope. This does not establish complete product coverage.",
            "",
        ]
    for candidate in candidates:
        verdict = by_candidate[candidate["id"]]
        lines += [
            f"### {candidate['id']}: {candidate['title']}",
            "",
            f"Validation: **{verdict['status']}**",
            "",
            "Expected: " + candidate["expected"],
            "",
            "Observed: " + candidate["observed"],
            "",
            "Trigger: " + candidate["trigger"],
            "",
            verdict["rationale"],
            "",
            "Evidence: " + ", ".join(verdict["evidence"]),
            "",
        ]
    lines += ["## Unresolved and coverage limits", ""]
    lines += ["- " + note for note in [*document["uncertainties"], *document["excluded_areas"]]]
    if state.get("error"):
        lines += ["", state["error"], ""]
    lines += ["", "## Reproduction and environment", ""]
    if state.get("environment_retained"):
        lines += ["Docker resources are retained. External containers remain externally owned.", ""]
    if state.get("environment_cleaned_at"):
        lines += ["Owned Docker resources cleaned at " + state["environment_cleaned_at"] + ".", ""]
    if state.get("tmux_cleaned_at"):
        lines += ["Owned tmux sessions cleaned at " + state["tmux_cleaned_at"] + ".", ""]
    if state.get("reproductions"):
        lines += ["### Reproduction attempts", ""]
        for attempt in state["reproductions"]:
            lines += ["Attempt: " + attempt["at"], ""]
            for record in attempt["records"]:
                lines += [
                    f"- `{record['test_id']}`: exit {record['result']['returncode']}; "
                    f"evidence: `{record['directory']}/execution.json`",
                ]
            lines += [""]
    lines += [
        f"`aaqe reproduce {state['id']} --case <test-id>`",
        "",
        f"`aaqe env status {state['id']}`",
        "",
        f"`aaqe cleanup {state['id']}`",
        "",
        "Use the same --state-dir when a custom run directory was configured. Cleanup retains reports and the Git worktree.",
        "",
        "Raw stage artifacts, test patch, baseline and per-attempt process/JUnit records are stored beside this report.",
        "",
    ]
    (directory / "report.md").write_text("\n".join(lines), encoding="utf-8")
    return document
