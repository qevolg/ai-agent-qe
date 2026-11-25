import json
import os
import shutil
import sys
from pathlib import Path

import pytest
from conftest import git

from aaqe.config import Config
from aaqe.environment import ComposeEnvironment
from aaqe.state import Store
from aaqe.workflow import reproduce, run_audit


@pytest.mark.skipif(os.environ.get("AAQE_DOCKER_TESTS") != "1", reason="opt-in real Docker test")
def test_real_docker_workflow_reproduction_and_owned_cleanup(repository, tmp_path):
    project = Path(__file__).resolve().parents[1]
    shutil.copytree(project / "examples/minimal", repository, dirs_exist_ok=True)
    git(repository, "add", ".")
    git(repository, "commit", "-m", "add runnable project")
    git(repository, "push", "origin", "main")
    config = Config.load(repository)
    config.state_dir = tmp_path / "runs"
    config.codex_command = [sys.executable, str(project / "tests/fixtures/scripted_codex.py")]
    state = run_audit("Verify the documented inclusive bounds", repository, config, progress=print)
    runtime = ComposeEnvironment(
        state["workspace"]["path"],
        config.state_dir / state["id"],
        state["id"],
        state["environment"],
    )
    try:
        assert state["status"] == "completed", state.get("error")
        report = json.loads((config.state_dir / state["id"] / "report.json").read_text())
        assert report["verdicts"][0]["status"] == "confirmed"
        assert runtime.status().returncode == 0
        records = reproduce(Store(config.state_dir), state, "negative-input")
        assert records[0]["junit"]["complete"]
        assert records[0]["junit"]["cases"][0]["status"] == "failed"
        assert records[0]["directory"] != report["execution"]["tests"][0]["directory"]
    finally:
        if (runtime.env_dir / "ownership.json").exists():
            runtime.cleanup()
