import os
import signal
import subprocess
import time
from dataclasses import asdict, dataclass
from pathlib import Path


@dataclass
class Result:
    argv: list[str]
    returncode: int
    stdout: str
    stderr: str
    elapsed: float
    timed_out: bool = False

    def to_dict(self):
        return asdict(self)


def run(argv, *, cwd=None, timeout=600, input_text=None, env=None):
    started = time.monotonic()
    options = {"start_new_session": True} if os.name != "nt" else {}
    try:
        process = subprocess.Popen(
            [str(argument) for argument in argv],
            cwd=cwd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
            env=env,
            **options,
        )
    except OSError as error:
        return Result(list(map(str, argv)), 127, "", str(error), time.monotonic() - started)
    timed_out = False
    try:
        stdout, stderr = process.communicate(input_text, timeout=timeout)
    except (subprocess.TimeoutExpired, KeyboardInterrupt) as error:
        if os.name == "nt":
            subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], capture_output=True)
        else:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        stdout, stderr = process.communicate()
        if isinstance(error, KeyboardInterrupt):
            raise
        timed_out = True
    return Result(
        list(map(str, argv)),
        124 if timed_out else process.returncode,
        stdout,
        stderr,
        time.monotonic() - started,
        timed_out,
    )


def save_result(result, directory):
    from .state import write_json

    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "stdout.log").write_text(result.stdout, encoding="utf-8")
    (directory / "stderr.log").write_text(result.stderr, encoding="utf-8")
    write_json(directory / "process.json", result.to_dict())
