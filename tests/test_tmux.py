import json
import os
import signal
import subprocess
import sys
import time
import uuid
from concurrent.futures import ThreadPoolExecutor

import pytest

from aaqe.errors import AAQEError

pytestmark = pytest.mark.usefixtures("tmux_sessions")


def test_tmux_streams_logs_before_exit_and_preserves_arguments_and_environment(
    tmp_path, monkeypatch
):
    from aaqe.tmux import run_tmux

    workspace = tmp_path / "workspace with 'quotes $dollar"
    workspace.mkdir()
    directory = tmp_path / "stage with spaces"
    directory.mkdir()
    prompt = directory / "prompt.md"
    prompt.write_text("stage input\n")
    monkeypatch.setenv("AAQE_TEST_TMUX_ENV", "current caller environment")
    executable = workspace / "script.py"
    executable.write_text(
        "import os,pathlib,sys,time\n"
        "print(sys.stdin.read(), end='', flush=True)\n"
        "print(sys.argv[1], flush=True)\n"
        "print(os.environ['AAQE_TEST_TMUX_ENV'], file=sys.stderr, flush=True)\n"
        "pathlib.Path('ready').touch()\n"
        "while not pathlib.Path('release').exists(): time.sleep(0.01)\n"
    )
    argument = "; touch UNEXPECTED $(echo unsafe)"
    with ThreadPoolExecutor() as executor:
        future = executor.submit(
            run_tmux,
            [sys.executable, str(executable), argument],
            cwd=workspace,
            directory=directory,
            input_path=prompt,
            timeout=10,
        )
        try:
            deadline = time.monotonic() + 5
            while not (workspace / "ready").exists() and time.monotonic() < deadline:
                time.sleep(0.02)
            assert (workspace / "ready").exists()
            assert not future.done()
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                if argument in (directory / "stdout.log").read_text():
                    break
                time.sleep(0.02)
            assert argument in (directory / "stdout.log").read_text()
            assert "current caller environment" in (directory / "stderr.log").read_text()
            session = json.loads((directory / "tmux.json").read_text())["session"]
            pane = subprocess.run(
                ["tmux", "capture-pane", "-p", "-S", "-", "-t", session],
                check=True,
                capture_output=True,
                text=True,
            ).stdout
            assert "stage input" in pane
            assert "current caller environment" in pane
        finally:
            (workspace / "release").touch()
        result = future.result(timeout=5)
    assert result.returncode == 0
    assert result.stdout == "stage input\n" + argument + "\n"
    assert not (workspace / "UNEXPECTED").exists()
    assert "current caller environment" not in (directory / "tmux.json").read_text()


@pytest.mark.parametrize("exit_code", [0, 23])
def test_tmux_preserves_process_exit_code(tmp_path, exit_code):
    from aaqe.tmux import run_tmux

    prompt = tmp_path / "prompt.md"
    prompt.write_text("")
    result = run_tmux(
        [
            sys.executable,
            "-c",
            f"import sys; print('diagnostic', file=sys.stderr); sys.exit({exit_code})",
        ],
        cwd=tmp_path,
        directory=tmp_path,
        input_path=prompt,
        timeout=5,
    )
    assert result.returncode == exit_code
    assert not result.timed_out
    assert result.stderr == "diagnostic\n"


def test_tmux_timeout_terminates_descendants_and_retains_evidence(tmp_path):
    from aaqe.tmux import run_tmux

    prompt = tmp_path / "prompt.md"
    prompt.write_text("")
    command = (
        "import pathlib,subprocess,sys,time; "
        "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)']); "
        "pathlib.Path('child.pid').write_text(str(child.pid)); "
        "print('before timeout',flush=True); time.sleep(30)"
    )
    result = run_tmux(
        [sys.executable, "-c", command],
        cwd=tmp_path,
        directory=tmp_path,
        input_path=prompt,
        timeout=1,
    )
    assert result.timed_out
    assert result.returncode == 124
    assert result.stdout == "before timeout\n"
    child_pid = int((tmp_path / "child.pid").read_text())
    try:
        os.kill(child_pid, 0)
    except ProcessLookupError:
        pass
    else:
        with open(f"/proc/{child_pid}/stat") as status:
            assert status.read().split()[2] == "Z"
    assert json.loads((tmp_path / "process.json").read_text())["timed_out"]


def test_tmux_reports_a_missing_executable(tmp_path):
    from aaqe.tmux import run_tmux

    prompt = tmp_path / "prompt.md"
    prompt.write_text("")
    result = run_tmux(
        ["aaqe-nonexistent-executable"],
        cwd=tmp_path,
        directory=tmp_path,
        input_path=prompt,
        timeout=5,
    )
    assert result.returncode == 127
    assert "aaqe-nonexistent-executable" in result.stderr


def test_missing_tmux_has_an_actionable_error(tmp_path, monkeypatch):
    from aaqe.tmux import run_tmux

    monkeypatch.setattr("aaqe.tmux.shutil.which", lambda command: None)
    with pytest.raises(AAQEError, match="tmux.*Linux.*WSL"):
        run_tmux([], cwd=tmp_path, directory=tmp_path, input_path=tmp_path / "prompt.md", timeout=5)


def test_deleting_a_live_tmux_session_terminates_codex(tmp_path):
    from aaqe.tmux import run_tmux

    prompt = tmp_path / "prompt.md"
    prompt.write_text("")
    command = (
        "import os,pathlib,time; pathlib.Path('codex.pid').write_text(str(os.getpid())); "
        "time.sleep(30)"
    )
    with ThreadPoolExecutor() as executor:
        future = executor.submit(
            run_tmux,
            [sys.executable, "-c", command],
            cwd=tmp_path,
            directory=tmp_path,
            input_path=prompt,
            timeout=10,
        )
        deadline = time.monotonic() + 5
        while not (tmp_path / "codex.pid").exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        assert (tmp_path / "codex.pid").exists()
        session = json.loads((tmp_path / "tmux.json").read_text())["session"]
        subprocess.run(["tmux", "kill-session", "-t", session], check=True)
        try:
            future.result(timeout=7)
        except AAQEError:
            pass
    codex_pid = int((tmp_path / "codex.pid").read_text())
    deadline = time.monotonic() + 2
    while time.monotonic() < deadline:
        try:
            os.kill(codex_pid, 0)
        except ProcessLookupError:
            break
        time.sleep(0.02)
    else:
        os.kill(codex_pid, 9)
        pytest.fail("deleting the tmux session left Codex running")


def test_interrupting_aaqe_stops_codex_and_preserves_the_session(tmp_path):
    prompt = tmp_path / "prompt.md"
    prompt.write_text("")
    launcher = tmp_path / "launcher.py"
    launcher.write_text(
        "import pathlib,sys\n"
        "from aaqe.tmux import run_tmux\n"
        "directory=pathlib.Path(sys.argv[1])\n"
        "try:\n"
        "    run_tmux([sys.executable,'-c',"
        "\"import os,pathlib,time; pathlib.Path('codex.pid').write_text(str(os.getpid())); "
        'time.sleep(30)"],cwd=directory,directory=directory,'
        "input_path=directory/'prompt.md',timeout=10)\n"
        "except KeyboardInterrupt:\n"
        "    (directory/'interrupted').touch()\n"
    )
    parent = subprocess.Popen([sys.executable, str(launcher), str(tmp_path)])
    try:
        deadline = time.monotonic() + 5
        while not (tmp_path / "codex.pid").exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        assert (tmp_path / "codex.pid").exists()
        parent.send_signal(signal.SIGINT)
        assert parent.wait(timeout=7) == 0
    finally:
        if parent.poll() is None:
            parent.kill()
            parent.wait()
    assert (tmp_path / "interrupted").exists()
    process = json.loads((tmp_path / "process.json").read_text())
    assert process["returncode"] == 130
    assert not process["timed_out"]
    with pytest.raises(ProcessLookupError):
        os.kill(int((tmp_path / "codex.pid").read_text()), 0)
    session = json.loads((tmp_path / "tmux.json").read_text())["session"]
    assert subprocess.run(["tmux", "has-session", "-t", session]).returncode == 0


def test_progress_interruption_stops_the_just_started_session(tmp_path):
    from aaqe.tmux import run_tmux

    prompt = tmp_path / "prompt.md"
    prompt.write_text("")

    def interrupt(message):
        raise KeyboardInterrupt

    with pytest.raises(KeyboardInterrupt):
        run_tmux(
            [
                sys.executable,
                "-c",
                "import pathlib,time; time.sleep(0.5); pathlib.Path('leaked').touch()",
            ],
            cwd=tmp_path,
            directory=tmp_path,
            input_path=prompt,
            timeout=5,
            progress=interrupt,
        )
    time.sleep(0.7)
    assert not (tmp_path / "leaked").exists()


def test_codex_does_not_inherit_stale_tmux_server_environment(tmp_path, monkeypatch):
    from aaqe.tmux import run_tmux

    socket = "aaqe-test-" + uuid.uuid4().hex
    subprocess.run(
        ["tmux", "-L", socket, "new-session", "-d", "-s", "seed", "sleep 30"],
        env={**os.environ, "AAQE_TEST_STALE_ENV": "stale server value"},
        check=True,
    )

    def isolated_tmux(argv, **kwargs):
        from aaqe.process import run

        return run([argv[0], "-L", socket, *argv[1:]], **kwargs)

    monkeypatch.setattr("aaqe.tmux.run", isolated_tmux)
    prompt = tmp_path / "prompt.md"
    prompt.write_text("")
    try:
        result = run_tmux(
            [sys.executable, "-c", "import os; print(os.getenv('AAQE_TEST_STALE_ENV', 'unset'))"],
            cwd=tmp_path,
            directory=tmp_path,
            input_path=prompt,
            timeout=5,
        )
        assert result.returncode == 0
        assert result.stdout == "unset\n"
    finally:
        subprocess.run(["tmux", "-L", socket, "kill-server"], capture_output=True)


def test_cleanup_refuses_a_session_owned_by_another_stage(tmp_path):
    from aaqe.state import write_json
    from aaqe.tmux import cleanup_tmux, run_tmux

    stage = tmp_path / "other-run" / "stages" / "code-explorer"
    stage.mkdir(parents=True)
    prompt = stage / "prompt.md"
    prompt.write_text("")
    run_tmux(
        [sys.executable, "-c", "pass"],
        cwd=tmp_path,
        directory=stage,
        input_path=prompt,
        timeout=5,
    )
    metadata = json.loads((stage / "tmux.json").read_text())
    write_json(tmp_path / "run" / "stages" / "code-explorer" / "tmux.json", metadata)
    with pytest.raises(AAQEError, match="ownership"):
        cleanup_tmux(tmp_path / "run")
    assert subprocess.run(["tmux", "has-session", "-t", "=" + metadata["session"]]).returncode == 0


def test_cleanup_does_not_accept_a_tmux_target_pattern(tmp_path):
    from aaqe.state import write_json
    from aaqe.tmux import cleanup_tmux

    write_json(tmp_path / "stages" / "code-explorer" / "tmux.json", {"session": "aaqe-"})
    with pytest.raises(AAQEError, match="Invalid tmux session"):
        cleanup_tmux(tmp_path)
