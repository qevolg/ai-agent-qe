from dataclasses import asdict, dataclass, field
from pathlib import Path

import jsonschema
import yaml

from .errors import AAQEError


def object_schema(properties):
    return {"type": "object", "properties": properties, "additionalProperties": False}


STRINGS = {"type": "array", "items": {"type": "string", "minLength": 1}}
ARGV = {**STRINGS, "minItems": 1}
ENVIRONMENT_SCHEMA = object_schema(
    {
        "compose_files": STRINGS,
        "services": STRINGS,
        "runner_service": {"type": "string"},
        "runner_workdir": {"type": "string"},
        "external_containers": STRINGS,
        "env_file": {"type": "string"},
        "readiness_timeout": {"type": "integer", "minimum": 1},
    }
)
CONFIG_SCHEMA = object_schema(
    {
        "state_dir": {"type": "string", "minLength": 1},
        "git": object_schema(
            {
                "remote": {"type": "string", "pattern": "^[A-Za-z0-9][A-Za-z0-9._-]*$"},
                "branch": {"type": "string", "pattern": "^[A-Za-z0-9][A-Za-z0-9/._-]*$"},
            }
        ),
        "codex": object_schema(
            {
                "command": ARGV,
                "model": {"type": "string"},
            }
        ),
        "environment": ENVIRONMENT_SCHEMA,
        "references": STRINGS,
        "test_paths": STRINGS,
        "baseline": {"type": "array", "items": ARGV},
        "timeouts": object_schema(
            {name: {"type": "integer", "minimum": 1} for name in ("agent", "test", "docker")}
        ),
    }
)


@dataclass
class Config:
    state_dir: Path = field(default_factory=lambda: Path.home() / ".aaqe" / "runs")
    remote: str = "origin"
    branch: str = "main"
    codex_command: list[str] = field(default_factory=lambda: ["codex"])
    model: str = ""
    environment: dict = field(default_factory=dict)
    references: list[str] = field(default_factory=list)
    test_paths: list[str] = field(
        default_factory=lambda: [
            "tests/**",
            "test/**",
            "**/tests/**",
            "**/test/**",
            ".aaqe/tests/**",
        ]
    )
    baseline: list[list[str]] = field(default_factory=list)
    agent_timeout: int = 1800
    test_timeout: int = 1200
    docker_timeout: int = 1200

    @classmethod
    def load(cls, directory, filename=None):
        directory = Path(directory).resolve()
        path = Path(filename).resolve() if filename else directory / "aaqe.yaml"
        if not path.exists():
            if filename:
                raise AAQEError(f"Configuration does not exist: {path}")
            return cls()
        try:
            data = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
            jsonschema.validate(data, CONFIG_SCHEMA)
        except (OSError, yaml.YAMLError, jsonschema.ValidationError) as error:
            raise AAQEError(f"Invalid configuration: {error}") from error
        git = data.get("git", {})
        codex = data.get("codex", {})
        timeouts = data.get("timeouts", {})
        environment = dict(data.get("environment", {}))
        if environment.get("env_file"):
            environment["env_file"] = str((path.parent / environment["env_file"]).resolve())
        state = Path(data.get("state_dir", Path.home() / ".aaqe" / "runs")).expanduser()
        return cls(
            state_dir=(path.parent / state).resolve(),
            remote=git.get("remote", "origin"),
            branch=git.get("branch", "main"),
            codex_command=codex.get("command", ["codex"]),
            model=codex.get("model", ""),
            environment=environment,
            references=data.get("references", []),
            test_paths=data.get("test_paths", cls().test_paths),
            baseline=data.get("baseline", []),
            agent_timeout=timeouts.get("agent", 1800),
            test_timeout=timeouts.get("test", 1200),
            docker_timeout=timeouts.get("docker", 1200),
        )

    def to_dict(self):
        return {**asdict(self), "state_dir": str(self.state_dir)}

    @classmethod
    def from_dict(cls, data):
        return cls(**{**data, "state_dir": Path(data["state_dir"])})
