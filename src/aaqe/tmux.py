import os
import re
import selectors
import shlex
import shutil
import signal
import subprocess
import sys
import time
import uuid
from pathlib import Path

from .errors import AAQEError
from .process import Result, run
from .state import read_json, write_json


def run_tmux(argv, *, cwd, directory, input_path, timeout, progress=None):
    if os.name == "nt" or not shutil.which("tmux"):
        raise AAQEError("Codex requires tmux; install tmux and run AAQE on Linux or WSL2")
    directory = Path(directory).resolve()
    directory.mkdir(parents=True, exist_ok=True)
    session = "aaqe-" + directory.name + "-" + uuid.uuid4().hex[:12]
    attach = f"tmux attach-session -t {shlex.quote(session)}"
    write_json(directory / "tmux.json", {"session": session, "attach_command": attach})
    write_json(
        directory / "invocation.json",
        {
            "argv": [str(argument) for argument in argv],
            "cwd": str(Path(cwd).resolve()),
            "input_path": str(Path(input_path).resolve()),
            "timeout": timeout,
        },
    )
    command = ["tmux", "new-session", "-d", "-s", session, "-c", str(Path(cwd).resolve())]
    environment = [
        f"{name}={value}" for name, value in os.environ.items() if name not in ("TMUX", "TMUX_PANE")
    ]
    command.append(
        "exec "
        + shlex.join(["env", "-i", *environment])
        + ' TMUX="$TMUX" TMUX_PANE="$TMUX_PANE" '
        + shlex.join([sys.executable, "-m", "aaqe.tmux", str(directory)])
    )
    command += [";", "set-option", "-t", session, "@aaqe_directory", str(directory)]
    try:
        launched = run(command, timeout=20)
        if launched.returncode:
            raise AAQEError("Cannot start Codex tmux session: " + launched.stderr.strip())
        if progress:
            progress("Codex session: " + attach)
        deadline = time.monotonic() + timeout + 20
        while not (directory / "finished").exists():
            alive = run(["tmux", "has-session", "-t", session], timeout=5)
            if alive.returncode:
                raise AAQEError(f"Codex tmux session disappeared; see {directory}")
            if time.monotonic() >= deadline:
                raise AAQEError(f"Codex tmux worker did not finish before timeout; see {directory}")
            time.sleep(0.05)
    except BaseException:
        _stop_worker(session, directory)
        raise
    return Result(**read_json(directory / "process.json"))


def cleanup_tmux(directory):
    directory = Path(directory).resolve()
    sessions = []
    for metadata in sorted((directory / "stages").glob("*/tmux.json")):
        if not metadata.resolve().is_relative_to(directory):
            raise AAQEError(f"Invalid tmux metadata path: {metadata}")
        session = read_json(metadata).get("session")
        if not isinstance(session, str) or not re.fullmatch(
            r"aaqe-[A-Za-z0-9_-]+-[0-9a-f]{12}", session
        ):
            raise AAQEError(f"Invalid tmux session in {metadata}")
        sessions.append((session, metadata.parent.resolve()))
    if not sessions:
        return
    listed = run(["tmux", "list-sessions", "-F", "#{session_name}"], timeout=5)
    if listed.returncode:
        if listed.returncode == 1 and (
            "no server running" in listed.stderr or "No such file or directory" in listed.stderr
        ):
            return
        raise AAQEError("Cannot list tmux sessions: " + listed.stderr.strip())
    existing = set(listed.stdout.splitlines())
    owned = []
    for session, stage_directory in sessions:
        if session not in existing:
            continue
        target = "=" + session
        owner = run(["tmux", "show-options", "-qv", "-t", session, "@aaqe_directory"], timeout=5)
        if owner.returncode or owner.stdout.rstrip("\n") != str(stage_directory):
            raise AAQEError(f"Tmux session ownership does not match stage directory: {session}")
        owned.append(target)
    for target in owned:
        deleted = run(["tmux", "kill-session", "-t", target], timeout=5)
        if deleted.returncode:
            raise AAQEError(f"Cannot clean tmux session {target}: {deleted.stderr.strip()}")


def _stop_worker(session, directory):
    pane = run(["tmux", "display-message", "-p", "-t", session, "#{pane_pid}"], timeout=5)
    if pane.returncode:
        return
    try:
        os.kill(int(pane.stdout.strip()), signal.SIGTERM)
    except (ValueError, ProcessLookupError):
        pass
    deadline = time.monotonic() + 5
    while not (directory / "finished").exists() and time.monotonic() < deadline:
        time.sleep(0.05)
    if not (directory / "finished").exists():
        run(["tmux", "kill-session", "-t", session], timeout=5)


def _kill_process(process):
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def _display(stream, chunk):
    try:
        stream.write(chunk)
        stream.flush()
    except OSError:
        pass


def _worker(directory):
    invocation = read_json(directory / "invocation.json")
    started = time.monotonic()
    run(["tmux", "set-option", "-w", "remain-on-exit", "on"], timeout=5)
    process = None
    interrupted = False

    def stop(signum, frame):
        nonlocal interrupted
        interrupted = True
        if process is not None:
            _kill_process(process)

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    signal.signal(signal.SIGHUP, stop)
    stdout_path = directory / "stdout.log"
    stderr_path = directory / "stderr.log"
    timed_out = False
    returncode = 127
    with (
        stdout_path.open("wb") as stdout_log,
        stderr_path.open("wb") as stderr_log,
        Path(invocation["input_path"]).open("rb") as source,
        selectors.DefaultSelector() as selector,
    ):
        try:
            process = subprocess.Popen(
                invocation["argv"],
                cwd=invocation["cwd"],
                stdin=source,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                start_new_session=True,
            )
        except OSError as error:
            message = (str(error) + "\n").encode("utf-8", errors="replace")
            stderr_log.write(message)
            _display(sys.stderr.buffer, message)
        else:
            if interrupted:
                _kill_process(process)
            selector.register(process.stdout, selectors.EVENT_READ, (stdout_log, sys.stdout.buffer))
            selector.register(process.stderr, selectors.EVENT_READ, (stderr_log, sys.stderr.buffer))
            while selector.get_map() or process.poll() is None:
                if not timed_out and time.monotonic() - started >= invocation["timeout"]:
                    timed_out = True
                    _kill_process(process)
                for key, events in selector.select(timeout=0.05):
                    chunk = os.read(key.fd, 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        key.fileobj.close()
                        continue
                    logfile, terminal = key.data
                    logfile.write(chunk)
                    logfile.flush()
                    _display(terminal, chunk)
            returncode = process.wait()
    result = Result(
        invocation["argv"],
        124 if timed_out else 130 if interrupted else returncode,
        stdout_path.read_text(encoding="utf-8", errors="replace"),
        stderr_path.read_text(encoding="utf-8", errors="replace"),
        time.monotonic() - started,
        timed_out,
    )
    write_json(directory / "process.json", result.to_dict())
    _display(
        sys.stdout.buffer,
        f"\nAAQE Codex finished (exit {result.returncode}); logs: {directory}\n".encode("utf-8"),
    )
    (directory / "finished").touch()


if __name__ == "__main__":
    _worker(Path(sys.argv[1]))
