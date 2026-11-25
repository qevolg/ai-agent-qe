from pathlib import Path

from .codex import Codex
from .config import Config
from .contracts import validate, validate_links
from .environment import ComposeEnvironment
from .errors import AAQEError
from .evidence import parse_junit
from .gitops import create_workspace
from .patchgate import PatchGate, safe_path
from .process import run
from .reporting import generate_report, refresh_report
from .state import Store, now, read_json, write_json

STAGES = ["code-explorer", "bug-hunter", "test-designer", "test-developer", "test-validator"]


def collect_test(runtime, test, timeout):
    record = runtime.execute_test(test["command"], test["id"], timeout=timeout)
    report = Path(record["directory"]) / test["report"]
    record["candidate_id"] = test["candidate_id"]
    record["junit"] = parse_junit(report, test["expected_names"])
    process = record["result"]
    cases = record["junit"]["cases"]
    failed = any(case["status"] == "failed" for case in cases)
    if (
        process["timed_out"]
        or process["returncode"] in (124, 125, 126, 127, 137, 143)
        or process["returncode"] < 0
    ):
        record["junit"]["complete"] = False
        record["junit"]["errors"].append("execution_interrupted_or_start_failed")
    elif bool(process["returncode"]) != failed:
        record["junit"]["complete"] = False
        record["junit"]["errors"].append("process_and_target_results_disagree")
    write_json(Path(record["directory"]) / "execution.json", record)
    return record


def grounded_verdicts(artifact, execution):
    records = {record["test_id"]: record for record in execution.get("tests", [])}
    baseline = execution.get("baseline", [])
    healthy_baseline = bool(baseline) and all(
        record["result"]["returncode"] == 0 for record in baseline
    )
    verdicts = []
    for original in artifact["verdicts"]:
        verdict = dict(original)
        supporting = [records[test_id] for test_id in verdict["test_ids"] if test_id in records]
        complete = bool(supporting) and all(record["junit"]["complete"] for record in supporting)
        failed = any(
            case["status"] == "failed" for record in supporting for case in record["junit"]["cases"]
        )
        if verdict["status"] in ("confirmed", "not_reproduced"):
            allowed_evidence = {
                str(Path(record["directory"]) / "execution.json") for record in supporting
            }
            valid = complete and healthy_baseline and bool(verdict["evidence"])
            valid = valid and set(verdict["evidence"]) <= allowed_evidence
            valid = valid and (failed if verdict["status"] == "confirmed" else not failed)
            if not valid:
                verdict["agent_status"] = verdict["status"]
                verdict["status"] = "blocked"
                verdict["rationale"] = (
                    "Execution gate: incomplete, inconsistent or absent target/baseline evidence. Agent assessment: "
                    + verdict["rationale"]
                )
        verdict["execution_records"] = [
            str(Path(record["directory"]) / "execution.json") for record in supporting
        ]
        verdicts.append(verdict)
    return verdicts


def run_audit(
    prompt, caller, config, *, agent=None, environment_type=ComposeEnvironment, progress=None
):
    if not prompt.strip():
        raise AAQEError("An audit prompt is required")
    progress = progress or (lambda message: None)
    store = Store(config.state_dir)
    state = store.create(prompt, config, caller)
    directory = store.path(state["id"])
    artifacts = {}
    execution = {"baseline": [], "tests": []}
    verdicts = []
    runtime = None
    agent = agent or Codex(config)
    state["environment_retained"] = False
    try:
        progress(
            f"Run {state['id']}: Stage 0 — fetch {config.remote}/{config.branch} and create worktree"
        )
        workspace = create_workspace(caller, directory, config, state["id"])
        state["workspace"] = workspace.to_dict()
        gate = PatchGate(workspace.path, config.test_paths, base_commit=workspace.commit)
        state["status"] = "running"
        store.save(state)
        for number, stage in enumerate(STAGES, 1):
            state["stage"] = number
            entry = {"number": number, "name": stage, "status": "running", "started_at": now()}
            state["stages"].append(entry)
            store.save(state)
            progress(f"Stage {number} — {stage}")
            inputs = {
                "prompt": prompt,
                "base_commit": workspace.commit,
                "project_config": config.to_dict(),
                "artifacts": artifacts,
                "execution": execution if stage == "test-validator" else {},
                "artifact_directory": str(directory),
            }
            output = validate(
                stage, agent.invoke(stage, workspace.path, directory / "stages" / stage, inputs)
            )
            validate_links(stage, output, artifacts)
            if stage == "code-explorer":
                for reference in output["references"]:
                    source = safe_path(workspace.path, reference["path"])
                    if not source.is_file():
                        raise AAQEError("Source reference does not exist: " + reference["path"])
                    if reference["line"] > len(
                        source.read_text(encoding="utf-8", errors="replace").splitlines()
                    ):
                        raise AAQEError(
                            "Source reference line is outside the file: " + reference["id"]
                        )
            gate.verify_worktree()
            artifacts[stage] = output
            write_json(directory / "artifacts" / (stage + ".json"), output)
            entry.update(status="completed", finished_at=now())
            store.save(state)
            if stage == "bug-hunter" and not output["candidates"]:
                break
            if stage == "test-designer" and all(plan["blocked_reason"] for plan in output["plans"]):
                state["error"] = "All test plans are blocked; see test-designer artifact"
                state["status"] = "blocked"
                break
            if stage == "test-developer":
                (directory / "tests.diff").write_text(output["patch"], encoding="utf-8")
                changed = gate.apply(output["patch"])
                write_json(directory / "patch-check.json", {"accepted": True, "paths": changed})
                environment = {**artifacts["code-explorer"]["environment"], **config.environment}
                state["environment"] = environment
                runtime = environment_type(
                    workspace.path,
                    directory,
                    state["id"],
                    environment,
                    timeout=config.docker_timeout,
                )
                state["environment_retained"] = True
                store.save(state)
                progress("Stage 5 — start Docker environment and collect actual test evidence")
                runtime.start()
                baseline = config.baseline or artifacts["code-explorer"]["baseline"]
                if baseline:
                    reversed_patch = run(
                        ["git", "apply", "--reverse", "-"],
                        cwd=workspace.path,
                        input_text=output["patch"],
                    )
                    if reversed_patch.returncode:
                        raise AAQEError("Cannot restore the base code for baseline execution")
                    try:
                        for index, command in enumerate(baseline, 1):
                            record = runtime.execute_test(
                                command, f"baseline-{index}", timeout=config.test_timeout
                            )
                            execution["baseline"].append(record)
                            write_json(directory / "execution.json", execution)
                            if record["result"]["returncode"]:
                                raise AAQEError(
                                    "Baseline command failed; target verification is blocked"
                                )
                    finally:
                        gate.apply(output["patch"])
                for test in output["tests"]:
                    execution["tests"].append(collect_test(runtime, test, config.test_timeout))
                    write_json(directory / "execution.json", execution)
                    gate.verify_worktree()
            if stage == "test-validator":
                verdicts = grounded_verdicts(output, execution)
        if state["status"] == "running":
            state["status"] = "completed"
    except KeyboardInterrupt:
        state["status"] = "interrupted"
        state["error"] = "Interrupted by user; existing resources and evidence retained"
    except Exception as error:
        state["status"] = "blocked" if isinstance(error, AAQEError) else "failed"
        state["error"] = str(error)
        if state["stages"] and state["stages"][-1]["status"] == "running":
            state["stages"][-1].update(status="failed", finished_at=now())
    finally:
        if runtime is not None:
            try:
                runtime.logs()
            except Exception as error:
                state["log_collection_error"] = str(error)
        state["stage"] = 6
        state["finished_at"] = now()
        write_json(directory / "execution.json", execution)
        generate_report(directory, state, artifacts, execution, verdicts)
        store.save(state)
        progress(f"Stage 6 — {state['status']}; report: {directory / 'report.md'}")
    return state


def reproduce(store, state, case_id=None):
    directory = store.path(state["id"])
    if state.get("environment_cleaned_at"):
        raise AAQEError("This run's environment has already been cleaned")
    if not state.get("environment"):
        raise AAQEError("This run has no test environment")
    config = Config.from_dict(state["config"])
    tests = read_json(directory / "artifacts" / "test-developer.json")["tests"]
    selected = [test for test in tests if case_id is None or test["id"] == case_id]
    if not selected:
        raise AAQEError("Unknown test ID")
    gate = PatchGate(
        state["workspace"]["path"], config.test_paths, base_commit=state["workspace"]["commit"]
    )
    gate.verify_worktree()
    runtime = ComposeEnvironment(
        state["workspace"]["path"],
        directory,
        state["id"],
        state["environment"],
        timeout=config.docker_timeout,
    )
    records = []
    for test in selected:
        for index, command in enumerate(test["reproduction_commands"], 1):
            preparation = runtime.execute_test(
                command, test["id"] + f"-prepare-{index}", timeout=config.test_timeout
            )
            if preparation["result"]["returncode"]:
                raise AAQEError("Reproduction preparation failed; original evidence retained")
        records.append(collect_test(runtime, test, config.test_timeout))
        gate.verify_worktree()
    state.setdefault("reproductions", []).append({"at": now(), "records": records})
    store.save(state)
    refresh_report(directory, state)
    return records
