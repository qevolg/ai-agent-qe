from dataclasses import asdict, dataclass
from pathlib import Path

from .errors import AAQEError
from .process import run


def git(directory, *arguments):
    result = run(["git", "-C", str(directory), *arguments], timeout=180)
    if result.returncode:
        raise AAQEError(f"Git {arguments[0]} failed: {result.stderr.strip()}")
    return result.stdout.strip()


@dataclass
class Workspace:
    path: Path
    repository: Path
    commit: str
    branch: str

    def to_dict(self):
        return {key: str(value) for key, value in asdict(self).items()}


def create_workspace(caller, run_dir, config, run_id):
    repository = Path(git(caller, "rev-parse", "--show-toplevel")).resolve()
    git(repository, "check-ref-format", "refs/heads/" + config.branch)
    git(repository, "fetch", "--no-tags", config.remote, "refs/heads/" + config.branch)
    commit = git(repository, "rev-parse", "FETCH_HEAD^{commit}")
    path = Path(run_dir).resolve() / "worktree"
    branch = "aaqe/" + run_id
    git(repository, "worktree", "add", "-b", branch, str(path), commit)
    return Workspace(path, repository, commit, branch)
