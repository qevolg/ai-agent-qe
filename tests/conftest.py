import json
import subprocess

import pytest


@pytest.fixture
def tmux_sessions(tmp_path):
    yield
    for metadata in tmp_path.rglob("tmux.json"):
        session = json.loads(metadata.read_text())["session"]
        subprocess.run(["tmux", "kill-session", "-t", session], capture_output=True)


def git(directory, *args):
    return subprocess.run(
        ["git", "-C", str(directory), *args], check=True, capture_output=True, text=True
    ).stdout.strip()


@pytest.fixture
def repository(tmp_path):
    remote = tmp_path / "remote.git"
    subprocess.run(["git", "init", "--bare", str(remote)], check=True, capture_output=True)
    seed = tmp_path / "seed"
    seed.mkdir()
    git(seed, "init", "-b", "main")
    git(seed, "config", "user.name", "Test User")
    git(seed, "config", "user.email", "test@example.invalid")
    (seed / "app.py").write_text("def value():\n    return 1\n")
    (seed / "tests").mkdir()
    (seed / "tests" / "test_app.py").write_text("def test_value():\n    assert True\n")
    git(seed, "add", ".")
    git(seed, "commit", "-m", "initial")
    git(seed, "remote", "add", "origin", str(remote))
    git(seed, "push", "-u", "origin", "main")
    return seed
