import hashlib
import json
import uuid
from pathlib import Path

from .errors import AAQEError
from .patchgate import safe_path
from .process import run, save_result
from .state import write_json


class ComposeEnvironment:
    def __init__(self, workspace, run_dir, run_id, spec, *, timeout=1200, execute=run):
        self.workspace = Path(workspace).resolve()
        self.run_dir = Path(run_dir).resolve()
        self.project = "aaqe-" + run_id.lower()
        self.spec = dict(spec)
        self.timeout = timeout
        self._run = execute
        self.env_dir = self.run_dir / "environment"
        self.results = self.run_dir / "executions"

    def arguments(self, *, resolved=True):
        arguments = [
            "docker",
            "compose",
            "--project-name",
            self.project,
            "--project-directory",
            str(self.workspace),
        ]
        snapshot = self.env_dir / "compose.resolved.json"
        if resolved and not snapshot.is_file():
            raise AAQEError("Compose snapshot is missing; cannot inspect or change owned resources")
        if resolved and snapshot.exists():
            ownership = self.env_dir / "ownership.json"
            if (
                not ownership.exists()
                or json.loads(ownership.read_text(encoding="utf-8")).get("compose_sha256")
                != hashlib.sha256(snapshot.read_bytes()).hexdigest()
            ):
                raise AAQEError("Compose snapshot integrity check failed")
            return [*arguments, "-f", str(snapshot)]
        for relative in self.spec.get("compose_files", []):
            path = safe_path(self.workspace, relative)
            if not path.is_file():
                raise AAQEError(f"Compose file is missing: {relative}")
            arguments += ["-f", str(path)]
        if not self.spec.get("compose_files"):
            raise AAQEError("No Docker Compose environment was found; supply project configuration")
        if self.spec.get("env_file"):
            arguments += ["--env-file", self.spec["env_file"]]
        return [*arguments, "-f", str(self.env_dir / "aaqe.override.json")]

    def checked(self, arguments, label):
        result = self._run(arguments, timeout=self.timeout)
        save_result(result, self.env_dir / label)
        if result.returncode:
            raise AAQEError(
                f"Docker {label} failed (exit {result.returncode}); environment retained"
            )
        return result

    def start(self):
        self.env_dir.mkdir(parents=True, exist_ok=True)
        self.results.mkdir(parents=True, exist_ok=True)
        runner = self.spec.get("runner_service")
        if not runner:
            raise AAQEError("No test runner service was identified")
        for name in self.spec.get("external_containers", []):
            inspected = self.checked(
                ["docker", "inspect", "--type", "container", name],
                "external-" + uuid.uuid4().hex[:8],
            )
            try:
                if not json.loads(inspected.stdout)[0]["State"]["Running"]:
                    raise AAQEError("External test container is not running: " + name)
            except (ValueError, KeyError, IndexError) as error:
                raise AAQEError("Cannot inspect external test container: " + name) from error
        workdir = self.spec.get("runner_workdir") or "/workspace"
        if not workdir.startswith("/workspace") or ".." in workdir.split("/"):
            raise AAQEError("Runner working directory must be inside /workspace")
        write_json(
            self.env_dir / "aaqe.override.json",
            {
                "services": {
                    runner: {
                        "working_dir": workdir,
                        "volumes": [
                            {
                                "type": "bind",
                                "source": str(self.workspace),
                                "target": "/workspace",
                                "read_only": True,
                            },
                            {
                                "type": "bind",
                                "source": str(self.results),
                                "target": "/aaqe-results",
                            },
                        ],
                        "labels": {"io.aaqe.run": self.project},
                    }
                }
            },
        )
        rendered = self.checked(
            [*self.arguments(resolved=False), "config", "--format", "json"], "config"
        )
        try:
            document = json.loads(rendered.stdout)
        except ValueError as error:
            raise AAQEError("Docker Compose returned invalid configuration") from error
        if runner not in document.get("services", {}):
            raise AAQEError("Runner service is absent from Compose configuration")
        for service in document.get("services", {}).values():
            if service.get("container_name"):
                raise AAQEError("Owned Compose services must not use fixed container_name")
        for section in ("volumes", "networks"):
            for resource in document.get(section, {}).values():
                if (
                    resource.get("name")
                    and not resource.get("external")
                    and not resource["name"].startswith(self.project + "_")
                ):
                    raise AAQEError("Owned Docker resources must be scoped to the run's project")
        write_json(self.env_dir / "compose.resolved.json", document)
        write_json(
            self.env_dir / "ownership.json",
            {
                "project": self.project,
                "external_containers": self.spec.get("external_containers", []),
                "compose_sha256": hashlib.sha256(
                    (self.env_dir / "compose.resolved.json").read_bytes()
                ).hexdigest(),
            },
        )
        services = list(dict.fromkeys([*self.spec.get("services", []), runner]))
        self.checked(
            [
                *self.arguments(),
                "up",
                "-d",
                "--wait",
                "--wait-timeout",
                str(self.spec.get("readiness_timeout", 180)),
                *services,
            ],
            "up",
        )

    def execute_test(self, command, test_id, *, timeout):
        attempt = uuid.uuid4().hex
        directory = self.results / attempt
        directory.mkdir(parents=True, exist_ok=False)
        result_dir = "/aaqe-results/" + attempt
        argv = [part.replace("{result_dir}", result_dir) for part in command]
        result = self._run(
            [
                *self.arguments(),
                "exec",
                "-T",
                "-e",
                "AAQE_RESULT_DIR=" + result_dir,
                self.spec["runner_service"],
                *argv,
            ],
            timeout=timeout,
        )
        save_result(result, directory)
        evidence = {
            "test_id": test_id,
            "attempt": attempt,
            "directory": str(directory),
            "result": result.to_dict(),
        }
        write_json(directory / "execution.json", evidence)
        return evidence

    def status(self):
        return self._run([*self.arguments(), "ps", "--all", "--format", "json"], timeout=30)

    def logs(self):
        self.checked([*self.arguments(), "logs", "--no-color", "--timestamps"], "logs")

    def cleanup(self):
        if not (self.env_dir / "ownership.json").is_file():
            raise AAQEError("No owned environment was created for this run")
        ownership = json.loads((self.env_dir / "ownership.json").read_text(encoding="utf-8"))
        if ownership.get("project") != self.project:
            raise AAQEError("Environment ownership does not match run ID")
        self.checked([*self.arguments(), "down", "--volumes"], "cleanup")
