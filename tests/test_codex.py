import json
import subprocess
import sys

import pytest

from aaqe.codex import Codex
from aaqe.config import Config
from aaqe.contracts import schema, validate
from aaqe.errors import AAQEError
from aaqe.process import run

pytestmark = pytest.mark.usefixtures("tmux_sessions")


def test_codex_receives_skill_and_validates_final_output(tmp_path):
    executable = tmp_path / "codex.py"
    executable.write_text("""import json, pathlib, sys
arguments = sys.argv[1:]
prompt = sys.stdin.read()
pathlib.Path("received.txt").write_text(prompt)
assert "--sandbox" in arguments and arguments[arguments.index("--sandbox")+1] == "read-only"
assert "--ephemeral" in arguments
assert "--output-schema" in arguments
output = arguments[arguments.index("--output-last-message")+1]
pathlib.Path(output).write_text(json.dumps({"summary":"nothing found", "candidates":[], "excluded_areas":[]}))
""")
    config = Config(codex_command=[sys.executable, str(executable)])
    artifact = Codex(config).invoke(
        "bug-hunter", tmp_path, tmp_path / "stage", {"prompt": "check the module"}
    )
    assert artifact["candidates"] == []
    prompt = (tmp_path / "received.txt").read_text()
    assert "check the module" in prompt
    assert "Inputs" in prompt and "Outputs" in prompt


def test_codex_runs_in_a_retained_tmux_session_and_announces_attachment(tmp_path):
    executable = tmp_path / "codex.py"
    executable.write_text(
        "import json,pathlib,sys\n"
        'print("visible stage output", flush=True)\n'
        'print("visible diagnostic", file=sys.stderr, flush=True)\n'
        'pathlib.Path(sys.argv[sys.argv.index("--output-last-message")+1]).write_text('
        'json.dumps({"summary":"done", "candidates":[], "excluded_areas":[]}))\n'
    )
    messages = []
    directory = tmp_path / "stage"
    artifact = Codex(
        Config(codex_command=[sys.executable, str(executable)]), progress=messages.append
    ).invoke("bug-hunter", tmp_path, directory, {})
    assert artifact["summary"] == "done"
    metadata = json.loads((directory / "tmux.json").read_text())
    assert any(metadata["attach_command"] in message for message in messages)
    pane = subprocess.run(
        ["tmux", "capture-pane", "-p", "-S", "-", "-t", metadata["session"]],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    assert "visible stage output" in pane
    assert "visible diagnostic" in pane
    assert (directory / "stdout.log").read_text() == "visible stage output\n"
    assert (directory / "stderr.log").read_text() == "visible diagnostic\n"
    process = json.loads((directory / "process.json").read_text())
    assert process["returncode"] == 0
    assert process["argv"][0] == sys.executable


def test_invalid_model_output_is_not_accepted(tmp_path):
    executable = tmp_path / "codex.py"
    executable.write_text(
        'import pathlib,sys\npathlib.Path(sys.argv[sys.argv.index("--output-last-message")+1]).write_text("{}")\n'
    )
    with pytest.raises(AAQEError, match="contract"):
        Codex(Config(codex_command=[sys.executable, str(executable)])).invoke(
            "bug-hunter", tmp_path, tmp_path / "stage", {}
        )


def test_process_timeout_records_failure(tmp_path):
    result = run([sys.executable, "-c", "import time; time.sleep(30)"], timeout=0.05)
    assert result.timed_out
    assert result.returncode == 124


def test_model_schema_omits_unsupported_uniqueness_but_host_keeps_validation():
    model_names = schema("test-developer")["properties"]["tests"]["items"]["properties"][
        "expected_names"
    ]
    assert "uniqueItems" not in model_names
    artifact = {
        "summary": "regression",
        "patch": "diff",
        "tests": [
            {
                "id": "test-1",
                "candidate_id": "bug-1",
                "command": ["test"],
                "report": "junit.xml",
                "expected_names": ["same", "same"],
                "reproduction_commands": [],
            }
        ],
        "limitations": [],
    }
    with pytest.raises(AAQEError, match="contract"):
        validate("test-developer", artifact)
