import json
from pathlib import Path

from test_patchgate import patch

from aaqe.config import Config
from aaqe.process import Result
from aaqe.state import Store
from aaqe.workflow import run_audit


class Agents:
    def __init__(self, *, candidate=True, fail_stage=None):
        self.calls = []
        self.candidate = candidate
        self.fail_stage = fail_stage

    def invoke(self, stage, workspace, directory, inputs):
        self.calls.append(stage)
        if stage == self.fail_stage:
            raise RuntimeError("model transport unavailable")
        artifacts = {
            "code-explorer": {
                "summary": "explored",
                "scope": ["app.py"],
                "references": [
                    {"id": "ref-1", "path": "app.py", "line": 1, "claim": "value is one"}
                ],
                "uncertainties": [],
                "environment": {
                    "compose_files": ["compose.yaml"],
                    "services": ["runner"],
                    "runner_service": "runner",
                    "runner_workdir": "/workspace",
                    "external_containers": [],
                    "readiness_timeout": 30,
                },
                "baseline": [["python", "-c", "pass"]],
            },
            "bug-hunter": {
                "summary": "candidate",
                "candidates": [
                    {
                        "id": "bug-1",
                        "title": "wrong value",
                        "expected": "two",
                        "observed": "one",
                        "trigger": "call value",
                        "evidence_ids": ["ref-1"],
                        "confidence": "high",
                    }
                ]
                if self.candidate
                else [],
                "excluded_areas": [],
            },
            "test-designer": {
                "summary": "plan",
                "plans": [
                    {
                        "candidate_id": "bug-1",
                        "test_id": "regression",
                        "expectation": "value is two",
                        "oracle": "external requirement",
                        "steps": ["call value"],
                        "reproduction": ["rerun test"],
                        "blocked_reason": None,
                    }
                ],
            },
            "test-developer": {
                "summary": "test",
                "patch": patch("tests/test_regression.py", "", "assert False\n"),
                "tests": [
                    {
                        "id": "regression",
                        "candidate_id": "bug-1",
                        "command": ["python", "test.py"],
                        "report": "junit.xml",
                        "expected_names": ["regression"],
                        "reproduction_commands": [],
                    }
                ],
                "limitations": [],
            },
            "test-validator": {
                "summary": "validated",
                "verdicts": [
                    {
                        "candidate_id": "bug-1",
                        "test_ids": ["regression"],
                        "status": "confirmed",
                        "rationale": "target assertion failed",
                        "evidence": ["execution.json"],
                    }
                ],
            },
        }
        if stage == "test-validator":
            evidence = inputs["execution"]["tests"][0]["directory"] + "/execution.json"
            artifacts[stage]["verdicts"][0]["evidence"] = [evidence]
        return artifacts[stage]


class Runtime:
    instances = []
    missing_report = False

    def __init__(self, workspace, run_dir, run_id, spec, **kwargs):
        self.workspace = Path(workspace)
        self.run_dir = Path(run_dir)
        self.calls = []
        self.spec = spec
        self.instances.append(self)

    def start(self):
        self.calls.append("start")

    def execute_test(self, command, test_id, **kwargs):
        self.calls.append(test_id)
        directory = self.run_dir / "executions" / test_id
        directory.mkdir(parents=True, exist_ok=True)
        if test_id == "regression" and not self.missing_report:
            (directory / "junit.xml").write_text(
                '<testsuite><testcase name="regression"><failure message="expected 2 got 1"/></testcase></testsuite>'
            )
        return {
            "test_id": test_id,
            "attempt": test_id,
            "directory": str(directory),
            "result": Result(command, 1 if test_id == "regression" else 0, "", "", 0).to_dict(),
        }

    def logs(self):
        self.calls.append("logs")

    def cleanup(self):
        raise AssertionError("workflow must retain environments")


def test_full_workflow_uses_five_independent_stages_and_retains_environment(repository, tmp_path):
    agents = Agents()
    config = Config(state_dir=tmp_path / "runs")
    state = run_audit(
        "inspect the product", repository, config, agent=agents, environment_type=Runtime
    )
    assert state["status"] == "completed"
    assert agents.calls == [
        "code-explorer",
        "bug-hunter",
        "test-designer",
        "test-developer",
        "test-validator",
    ]
    assert (Path(state["workspace"]["path"]) / "tests/test_regression.py").exists()
    assert not (repository / "tests/test_regression.py").exists()
    report = json.loads((config.state_dir / state["id"] / "report.json").read_text())
    assert report["verdicts"][0]["status"] == "confirmed"
    assert state["environment_retained"] is True
    assert Runtime.instances[-1].calls == ["start", "baseline-1", "regression", "logs"]


def test_empty_candidates_finish_without_docker_or_test_generation(repository, tmp_path):
    agents = Agents(candidate=False)
    state = run_audit(
        "inspect the module",
        repository,
        Config(state_dir=tmp_path / "runs"),
        agent=agents,
        environment_type=Runtime,
    )
    assert state["status"] == "completed"
    assert agents.calls == ["code-explorer", "bug-hunter"]
    assert state["environment"] is None


def test_missing_actual_test_evidence_overrules_model_confirmation(repository, tmp_path):
    class MissingRuntime(Runtime):
        missing_report = True

    config = Config(state_dir=tmp_path / "runs")
    state = run_audit(
        "inspect", repository, config, agent=Agents(), environment_type=MissingRuntime
    )
    report = json.loads((config.state_dir / state["id"] / "report.json").read_text())
    assert report["verdicts"][0]["status"] == "blocked"


def test_failed_stage_still_writes_report_and_run_record(repository, tmp_path):
    config = Config(state_dir=tmp_path / "runs")
    state = run_audit("inspect", repository, config, agent=Agents(fail_stage="bug-hunter"))
    assert state["status"] == "failed"
    assert "model transport unavailable" in state["error"]
    assert (config.state_dir / state["id"] / "report.md").exists()
    assert Store(config.state_dir).load(state["id"])["stage"] == 6


def test_fabricated_evidence_reference_cannot_confirm_candidate(repository, tmp_path):
    class FabricatingAgents(Agents):
        def invoke(self, stage, *args):
            artifact = super().invoke(stage, *args)
            if stage == "test-validator":
                artifact["verdicts"][0]["evidence"] = ["made-up-evidence.json"]
            return artifact

    config = Config(state_dir=tmp_path / "runs")
    state = run_audit(
        "inspect", repository, config, agent=FabricatingAgents(), environment_type=Runtime
    )
    report = json.loads((config.state_dir / state["id"] / "report.json").read_text())
    assert report["verdicts"][0]["status"] == "blocked"


def test_unknown_source_reference_blocks_following_stages(repository, tmp_path):
    class InventedSource(Agents):
        def invoke(self, stage, *args):
            artifact = super().invoke(stage, *args)
            if stage == "code-explorer":
                artifact["references"][0]["path"] = "does-not-exist.py"
            return artifact

    agents = InventedSource()
    state = run_audit("inspect", repository, Config(state_dir=tmp_path / "runs"), agent=agents)
    assert state["status"] == "blocked"
    assert agents.calls == ["code-explorer"]


def test_reproduction_refreshes_report_and_preserves_initial_verdict(
    repository, tmp_path, monkeypatch
):
    from aaqe.workflow import reproduce

    config = Config(state_dir=tmp_path / "runs")
    state = run_audit("inspect", repository, config, agent=Agents(), environment_type=Runtime)
    report_path = config.state_dir / state["id"] / "report.json"
    original = json.loads(report_path.read_text())
    monkeypatch.setattr("aaqe.workflow.ComposeEnvironment", Runtime)
    records = reproduce(Store(config.state_dir), state, "regression")
    refreshed = json.loads(report_path.read_text())
    assert refreshed["reproductions"][0]["records"] == records
    assert refreshed["verdicts"] == original["verdicts"]
    assert refreshed["execution"] == original["execution"]
    assert "Reproduction attempts" in report_path.with_suffix(".md").read_text()


def test_reproduction_rejects_source_commit_changed_after_audit(repository, tmp_path, monkeypatch):
    import pytest
    from conftest import git

    from aaqe.errors import AAQEError
    from aaqe.workflow import reproduce

    config = Config(state_dir=tmp_path / "runs")
    state = run_audit("inspect", repository, config, agent=Agents(), environment_type=Runtime)
    workspace = Path(state["workspace"]["path"])
    (workspace / "app.py").write_text("def value():\n    return 999\n")
    git(workspace, "add", "app.py")
    git(workspace, "commit", "-m", "change source after audit")
    monkeypatch.setattr("aaqe.workflow.ComposeEnvironment", Runtime)
    with pytest.raises(AAQEError, match="base commit"):
        reproduce(Store(config.state_dir), state, "regression")
