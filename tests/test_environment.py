import pytest

from aaqe.environment import ComposeEnvironment
from aaqe.errors import AAQEError
from aaqe.process import Result


class DockerCommands:
    def __init__(self):
        self.commands = []

    def __call__(self, argv, **kwargs):
        self.commands.append(list(map(str, argv)))
        if "inspect" in argv:
            return Result(argv, 0, '[{"State":{"Running":true}}]', "", 0)
        if "config" in argv:
            return Result(argv, 0, '{"services":{"runner":{"image":"python:3.12-slim"}}}', "", 0)
        return Result(argv, 0, "", "", 0)


def environment(tmp_path, commands):
    root = tmp_path / "worktree"
    root.mkdir()
    (root / "compose.yaml").write_text("services: {runner: {image: 'python:3.12-slim'}}\n")
    return ComposeEnvironment(
        root,
        tmp_path / "run",
        "test-run",
        {
            "compose_files": ["compose.yaml"],
            "services": ["runner"],
            "runner_service": "runner",
            "external_containers": ["shared-source"],
        },
        execute=commands,
    )


def test_start_retains_environment_and_reused_container_is_not_removed(tmp_path):
    commands = DockerCommands()
    runtime = environment(tmp_path, commands)
    runtime.start()
    assert any("up" in command and "--wait" in command for command in commands.commands)
    assert not any("down" in command or "rm" in command for command in commands.commands)
    runtime.cleanup()
    deletion = commands.commands[-1]
    assert "down" in deletion and "--volumes" in deletion
    assert "aaqe-test-run" in deletion
    assert "shared-source" not in deletion


def test_each_execution_has_fresh_report_directory_and_argv(tmp_path):
    runtime = environment(tmp_path, DockerCommands())
    runtime.start()
    first = runtime.execute_test(["python", "test.py", "space argument"], "regression", timeout=30)
    second = runtime.execute_test(["python", "test.py"], "regression", timeout=30)
    assert first["directory"] != second["directory"]
    assert first["result"]["argv"][-1] == "space argument"
    assert "AAQE_RESULT_DIR=" in " ".join(first["result"]["argv"])


def test_compose_path_cannot_escape_worktree(tmp_path):
    runtime = environment(tmp_path, DockerCommands())
    runtime.spec["compose_files"] = ["../outside.yaml"]
    with pytest.raises(AAQEError):
        runtime.start()


def test_changed_compose_snapshot_cannot_change_cleanup_scope(tmp_path):
    commands = DockerCommands()
    runtime = environment(tmp_path, commands)
    runtime.start()
    (runtime.env_dir / "compose.resolved.json").write_text(
        '{"services":{"runner":{"image":"other"}}}'
    )
    with pytest.raises(AAQEError, match="snapshot"):
        runtime.cleanup()
    assert not any("down" in command for command in commands.commands)


def test_missing_snapshot_cannot_fall_back_to_modified_project_files(tmp_path):
    commands = DockerCommands()
    runtime = environment(tmp_path, commands)
    runtime.start()
    (runtime.env_dir / "compose.resolved.json").unlink()
    with pytest.raises(AAQEError, match="snapshot"):
        runtime.cleanup()
    assert not any("down" in command for command in commands.commands)
