import argparse
import json
import subprocess
from pathlib import Path

from . import __version__
from .config import Config
from .environment import ComposeEnvironment
from .errors import AAQEError
from .process import run
from .reporting import refresh_report
from .state import Store, now
from .tmux import cleanup_tmux
from .workflow import reproduce, run_audit


def parser():
    root = argparse.ArgumentParser(
        prog="aaqe",
        description="Describe a product or module to inspect; AAQE audits it in an isolated worktree.",
    )
    root.add_argument("--version", action="version", version=__version__)
    root.add_argument("--config", type=Path)
    root.add_argument("--state-dir", type=Path)
    commands = root.add_subparsers(dest="action")
    launch = commands.add_parser("run", help="Run an audit from a natural-language prompt")
    launch.add_argument("prompt", nargs="?")
    for command in (launch,):
        command.add_argument("--config", type=Path, default=argparse.SUPPRESS)
        command.add_argument("--state-dir", type=Path, default=argparse.SUPPRESS)
    commands.add_parser("doctor", help="Check local executables without launching an audit")
    report = commands.add_parser("report", help="Print a saved report")
    report.add_argument("run_id")
    replay = commands.add_parser("reproduce", help="Rerun saved tests in the retained environment")
    replay.add_argument("run_id")
    replay.add_argument("--case", dest="case_id")
    cleanup = commands.add_parser(
        "cleanup", help="Explicitly remove this run's owned Docker environment and tmux sessions"
    )
    cleanup.add_argument("run_id")
    environment = commands.add_parser("env", help="Inspect or enter a retained test environment")
    operations = environment.add_subparsers(dest="operation", required=True)
    for operation in ("status", "shell"):
        selected = operations.add_parser(operation)
        selected.add_argument("run_id")
        if operation == "shell":
            selected.add_argument("--service")
    return root


def main(argv=None):
    arguments = parser().parse_args(argv)
    try:
        caller = Path.cwd()
        config = Config.load(caller, arguments.config)
        if arguments.state_dir:
            config.state_dir = arguments.state_dir.expanduser().resolve()
        if arguments.action in (None, "run"):
            prompt = getattr(arguments, "prompt", None)
            if prompt is None:
                prompt = input("Describe the product/module and what to check:\n> ")
            if not prompt.strip():
                raise AAQEError("An audit prompt is required")
            state = run_audit(prompt, caller, config, progress=print)
            print(f"Run {state['id']}: {state['status']}")
            if state.get("error"):
                print(state["error"])
            return 0 if state["status"] == "completed" else 1
        if arguments.action == "doctor":
            commands = [
                ["git", "--version"],
                [*config.codex_command, "--version"],
                ["tmux", "-V"],
                ["docker", "version", "--format", "{{.Server.Version}}"],
                ["docker", "compose", "version"],
            ]
            results = [run(command, timeout=20) for command in commands]
            for result in results:
                print(
                    f"{'OK' if result.returncode == 0 else 'FAIL'} {result.argv[0]}: {(result.stdout or result.stderr).strip()}"
                )
            return int(any(result.returncode for result in results))
        store = Store(config.state_dir)
        state = store.load(arguments.run_id)
        directory = store.path(state["id"])
        if arguments.action == "report":
            print((directory / "report.md").read_text(encoding="utf-8"))
            return 0
        if arguments.action == "reproduce":
            print(
                json.dumps(reproduce(store, state, arguments.case_id), ensure_ascii=False, indent=2)
            )
            return 0
        if arguments.action == "cleanup":
            if state.get("status") in ("initializing", "running"):
                raise AAQEError("Cannot clean resources while the audit is running")
            cleanup_tmux(directory)
            state.setdefault("tmux_cleaned_at", now())
            store.save(state)
            refresh_report(directory, state)
            if not state.get("environment_retained") or state.get("environment_cleaned_at"):
                print("Owned tmux sessions cleaned; reports, branch and worktree retained.")
                return 0
        if not state.get("workspace") or not state.get("environment"):
            raise AAQEError("This run has no Docker test environment")
        runtime = ComposeEnvironment(
            state["workspace"]["path"],
            directory,
            state["id"],
            state["environment"],
            timeout=Config.from_dict(state["config"]).docker_timeout,
        )
        if arguments.action == "cleanup":
            if not state.get("environment_cleaned_at"):
                runtime.cleanup()
                state["environment_cleaned_at"] = now()
                state["environment_retained"] = False
                store.save(state)
            refresh_report(directory, state)
            print(
                "Owned Docker resources and tmux sessions cleaned; reports, branch and worktree retained."
            )
            return 0
        if arguments.operation == "status":
            result = runtime.status()
            print(result.stdout or result.stderr)
            return int(result.returncode != 0)
        service = arguments.service or state["environment"]["runner_service"]
        return subprocess.call([*runtime.arguments(), "exec", service, "sh"])
    except (AAQEError, OSError, EOFError) as error:
        print(f"aaqe: {error}")
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
