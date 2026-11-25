import pytest
from conftest import git

from aaqe.config import Config
from aaqe.errors import AAQEError
from aaqe.gitops import create_workspace


def test_workspace_uses_fetched_main_and_preserves_dirty_caller(repository, tmp_path):
    git(repository, "checkout", "-b", "local-work")
    (repository / "app.py").write_text("local uncommitted work\n")
    workspace = create_workspace(repository / "tests", tmp_path / "run", Config(), "run-001")
    assert workspace.path != repository
    assert (workspace.path / "app.py").read_text() == "def value():\n    return 1\n"
    assert (repository / "app.py").read_text() == "local uncommitted work\n"
    assert git(repository, "branch", "--show-current") == "local-work"
    assert workspace.commit == git(repository, "rev-parse", "origin/main")
    assert git(workspace.path, "branch", "--show-current") == "aaqe/run-001"


def test_missing_remote_branch_does_not_fall_back_to_head(repository, tmp_path):
    with pytest.raises(AAQEError, match="Git"):
        create_workspace(repository, tmp_path / "run", Config(branch="missing"), "run-002")


def test_configuration_is_optional_and_resolves_relative_state(tmp_path):
    config = Config.load(tmp_path)
    assert config.remote == "origin"
    assert config.branch == "main"
    assert config.state_dir.is_absolute()
    (tmp_path / "aaqe.yaml").write_text("state_dir: runs\ngit:\n  branch: develop\n")
    config = Config.load(tmp_path)
    assert config.branch == "develop"
    assert config.state_dir == tmp_path / "runs"


@pytest.mark.parametrize(
    "content", ["unknown: true", "git: []", "environment: {services: nope}", "timeouts: {test: 0}"]
)
def test_invalid_configuration_fails_before_creating_resources(tmp_path, content):
    (tmp_path / "aaqe.yaml").write_text(content)
    with pytest.raises(AAQEError):
        Config.load(tmp_path)


def test_config_snapshot_omits_environment_secret_values(tmp_path):
    (tmp_path / "aaqe.yaml").write_text("environment:\n  env_file: private.env\n")
    (tmp_path / "private.env").write_text("PASSWORD=do-not-publish\n")
    config = Config.load(tmp_path)
    assert config.environment["env_file"] == str(tmp_path / "private.env")
    assert "do-not-publish" not in str(config.to_dict())
