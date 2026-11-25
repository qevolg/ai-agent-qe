import json
import subprocess
import sys

import pytest

from aaqe.cli import main
from aaqe.config import Config
from aaqe.state import Store
from aaqe.tmux import run_tmux


def test_doctor_checks_tmux(monkeypatch, tmp_path):
    from aaqe.process import Result

    monkeypatch.chdir(tmp_path)
    commands = []

    def execute(command, **kwargs):
        commands.append(command)
        return Result(command, 0, "available", "", 0)

    monkeypatch.setattr("aaqe.cli.run", execute)
    assert main(["doctor"]) == 0
    assert ["tmux", "-V"] in commands


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


@pytest.mark.usefixtures("tmux_sessions")
@pytest.mark.parametrize("docker_created", [False, True])
def test_cleanup_removes_only_this_runs_tmux_sessions_and_preserves_logs(
    repository, tmp_path, monkeypatch, docker_created
):
    from test_workflow import Agents, Runtime

    from aaqe.workflow import run_audit

    config = Config(state_dir=tmp_path / "runs")
    state = run_audit(
        "inspect",
        repository,
        config,
        agent=Agents(candidate=docker_created),
        environment_type=Runtime,
    )
    directory = config.state_dir / state["id"]
    stage = directory / "stages" / "code-explorer"
    stage.mkdir(parents=True)
    prompt = stage / "prompt.md"
    prompt.write_text("saved prompt")
    run_tmux(
        [sys.executable, "-c", "print('saved stage evidence')"],
        cwd=repository,
        directory=stage,
        input_path=prompt,
        timeout=5,
    )
    session = json.loads((stage / "tmux.json").read_text())["session"]
    other = tmp_path / "other-run" / "stages" / "code-explorer"
    other.mkdir(parents=True)
    (other / "prompt.md").write_text("")
    run_tmux(
        [sys.executable, "-c", "print('other run')"],
        cwd=repository,
        directory=other,
        input_path=other / "prompt.md",
        timeout=5,
    )
    other_session = json.loads((other / "tmux.json").read_text())["session"]
    docker_cleaned = []
    monkeypatch.setattr(
        "aaqe.cli.ComposeEnvironment.cleanup", lambda self: docker_cleaned.append(self.project)
    )
    monkeypatch.chdir(tmp_path)
    command = ["--state-dir", str(config.state_dir), "cleanup", state["id"]]
    assert main(command) == 0
    assert main(command) == 0
    assert len(docker_cleaned) == int(docker_created)
    assert subprocess.run(["tmux", "has-session", "-t", "=" + session]).returncode == 1
    assert subprocess.run(["tmux", "has-session", "-t", "=" + other_session]).returncode == 0
    assert prompt.read_text() == "saved prompt"
    assert (stage / "stdout.log").read_text() == "saved stage evidence\n"
    assert (stage / "process.json").is_file()
    state = Store(config.state_dir).load(state["id"])
    assert state["tmux_cleaned_at"]
    report = json.loads((directory / "report.json").read_text())
    assert report["tmux_cleaned_at"] == state["tmux_cleaned_at"]
    assert "Owned tmux sessions cleaned at" in (directory / "report.md").read_text()


def test_cleanup_without_docker_or_tmux_is_idempotent(repository, tmp_path, monkeypatch):
    from test_workflow import Agents

    from aaqe.workflow import run_audit

    config = Config(state_dir=tmp_path / "runs")
    state = run_audit("inspect", repository, config, agent=Agents(candidate=False))
    monkeypatch.chdir(tmp_path)
    command = ["--state-dir", str(config.state_dir), "cleanup", state["id"]]
    assert main(command) == 0
    assert main(command) == 0


def test_cleanup_refuses_an_active_audit(tmp_path, monkeypatch):
    config = Config(state_dir=tmp_path / "runs")
    store = Store(config.state_dir)
    state = store.create("inspect", config, tmp_path)
    state["status"] = "running"
    store.save(state)
    monkeypatch.chdir(tmp_path)
    assert main(["--state-dir", str(config.state_dir), "cleanup", state["id"]]) == 2
    assert "tmux_cleaned_at" not in store.load(state["id"])


def test_cleanup_records_tmux_completion_when_docker_cleanup_fails(
    repository, tmp_path, monkeypatch
):
    from test_workflow import Agents, Runtime

    from aaqe.errors import AAQEError
    from aaqe.workflow import run_audit

    config = Config(state_dir=tmp_path / "runs")
    state = run_audit("inspect", repository, config, agent=Agents(), environment_type=Runtime)

    def fail(self):
        raise AAQEError("Docker unavailable")

    monkeypatch.setattr("aaqe.cli.ComposeEnvironment.cleanup", fail)
    monkeypatch.chdir(tmp_path)
    assert main(["--state-dir", str(config.state_dir), "cleanup", state["id"]]) == 2
    saved = Store(config.state_dir).load(state["id"])
    assert saved["tmux_cleaned_at"]
    assert not saved.get("environment_cleaned_at")
    report = json.loads((config.state_dir / state["id"] / "report.json").read_text())
    assert report["tmux_cleaned_at"] == saved["tmux_cleaned_at"]
    assert report["environment_retained"]
