from aaqe.cli import main


def test_no_argument_entry_reads_a_prompt(monkeypatch, tmp_path, capsys):
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr("builtins.input", lambda _: "inspect this module")
    seen = []

    def execute(prompt, caller, config, **kwargs):
        seen.append((prompt, caller))
        return {"id": "sample-run", "status": "completed", "error": None}

    monkeypatch.setattr("aaqe.cli.run_audit", execute)
    assert main([]) == 0
    assert seen == [("inspect this module", tmp_path)]
    assert "sample-run" in capsys.readouterr().out


def test_run_accepts_prompt_without_named_target(monkeypatch, tmp_path):
    monkeypatch.chdir(tmp_path)
    seen = []
    monkeypatch.setattr(
        "aaqe.cli.run_audit",
        lambda prompt, *args, **kwargs: (
            seen.append(prompt) or {"id": "run", "status": "completed", "error": None}
        ),
    )
    assert main(["run", "check the entire product"]) == 0
    assert seen == ["check the entire product"]


def test_empty_prompt_is_rejected_before_running(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    assert main(["run", "   "]) == 2


def test_cleanup_refreshes_report_and_keeps_saved_evidence(repository, tmp_path, monkeypatch):
    import json

    from test_workflow import Agents, Runtime

    from aaqe.config import Config
    from aaqe.workflow import run_audit

    config = Config(state_dir=tmp_path / "runs")
    state = run_audit("inspect", repository, config, agent=Agents(), environment_type=Runtime)
    report_path = config.state_dir / state["id"] / "report.json"
    original = json.loads(report_path.read_text())
    cleaned = []
    monkeypatch.setattr(
        "aaqe.cli.ComposeEnvironment.cleanup", lambda self: cleaned.append(self.project)
    )
    monkeypatch.chdir(tmp_path)
    command = ["--state-dir", str(config.state_dir), "cleanup", state["id"]]
    assert main(command) == 0
    assert main(command) == 0
    refreshed = json.loads(report_path.read_text())
    assert len(cleaned) == 1
    assert refreshed["environment_retained"] is False
    assert refreshed["environment_cleaned_at"]
    assert refreshed["execution"] == original["execution"]
    assert "Docker resources are retained" not in report_path.with_suffix(".md").read_text()
