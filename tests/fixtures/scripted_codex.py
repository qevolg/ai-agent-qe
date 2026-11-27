"""Deterministic model boundary for integration tests; never used by real audits."""

import difflib
import json
import os
import signal
import sys
import subprocess
from pathlib import Path


def coordinate(arguments, directory):
    assert "exec" not in arguments
    assert "--json" not in arguments
    assert "--output-schema" not in arguments
    assert "--output-last-message" not in arguments
    assert "Initial audit status:" in arguments[-1]
    (directory / "terminal.json").write_text(json.dumps({"stdin":os.isatty(0), "stdout":os.isatty(1), "stderr":os.isatty(2)}))
    (directory / "tui.pid").write_text(str(os.getpid()))
    overrides = [arguments[i+1] for i, value in enumerate(arguments[:-1]) if value == "-c"]
    settings = dict(value.split("=", 1) for value in overrides)
    assert json.loads(settings.get("mcp_servers.aaqe.default_tools_approval_mode", '"prompt"')) == "approve", "Private harness tools must run with approval_policy=never"
    command = [json.loads(settings["mcp_servers.aaqe.command"])] + json.loads(settings["mcp_servers.aaqe.args"])
    child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    if "--kill-mcp-test" in arguments:
        def terminate_mcp(signum, frame):
            child.kill()
            child.wait()
            sys.exit(0)
        signal.signal(signal.SIGTERM, terminate_mcp)
    sequence = 0
    def request(method, params):
        nonlocal sequence
        sequence += 1
        child.stdin.write(json.dumps({"jsonrpc":"2.0", "id":sequence, "method":method, "params":params}) + "\n")
        child.stdin.flush()
        reply = json.loads(child.stdout.readline())
        assert "error" not in reply, reply
        return reply["result"]
    def tool(name, **arguments):
        return request("tools/call", {"name":name, "arguments":arguments})
    request("initialize", {"protocolVersion":"2024-11-05", "capabilities":{}, "clientInfo":{"name":"scripted-codex", "version":"1"}})
    assert {tool["name"] for tool in request("tools/list", {})["tools"]} == {"status", "run_stage", "inspect_environment", "prepare_environment", "execute_tests", "finish"}
    assert tool("run_stage", stage="test-developer", instruction="too early")["isError"]
    assert tool("finish", status="completed", reason="too early")["isError"]
    if "--early-inline" in arguments:
        inspected = tool("inspect_environment", probe={"kind":"docker"})
        assert not inspected["isError"], inspected
        assert inspected["structuredContent"]["result"]["success"], inspected
        readiness = """from pathlib import Path
from aaqe_fixture_dependency import VALUE
assert VALUE == 17
import errno
if not Path('/aaqe-results/early-ready').exists():
    assert not Path('/workspace/tests/test_regression.py').exists()
try:
    Path('/workspace/readiness-must-not-write').write_text('forbidden')
except OSError as error:
    assert error.errno == errno.EROFS, error
else:
    raise AssertionError('worktree mount was writable')
Path('/aaqe-results/early-ready').write_text('ready before Explorer and Developer')
"""
        prepared = tool("prepare_environment", environment={
            "compose": {"services":{"runner":{"image":"python:3.12-slim", "command":["sleep","infinity"], "init":True, "environment":{"PYTHONDONTWRITEBYTECODE":"1"}}}},
            "services":["runner"], "runner_service":"runner", "runner_workdir":"/workspace",
            "external_containers":[], "readiness_timeout":90,
            "readiness":[["python", "-c", readiness]],
        })
        assert prepared["isError"], prepared
        assert prepared["structuredContent"]["status"] == "failed", prepared
        inspected = tool("inspect_environment", probe={"kind":"tools", "tools":["python"]})
        assert inspected["structuredContent"]["result"]["target"] == "runner", inspected
        assert inspected["structuredContent"]["result"]["success"], inspected
        # Build a small offline wheel inside the runner, then use actual pip to
        # install it. No host package, network mirror or build backend is needed.
        wheel = "/aaqe-results/aaqe_fixture_dependency-1.0-py3-none-any.whl"
        package = """from zipfile import ZipFile
files = {
 'aaqe_fixture_dependency/__init__.py': 'VALUE = 17\\n',
 'aaqe_fixture_dependency-1.0.dist-info/METADATA': 'Metadata-Version: 2.1\\nName: aaqe-fixture-dependency\\nVersion: 1.0\\n',
 'aaqe_fixture_dependency-1.0.dist-info/WHEEL': 'Wheel-Version: 1.0\\nGenerator: AAQE-fixture\\nRoot-Is-Purelib: true\\nTag: py3-none-any\\n',
}
files['aaqe_fixture_dependency-1.0.dist-info/RECORD'] = ''.join(name + ',,\\n' for name in files) + 'aaqe_fixture_dependency-1.0.dist-info/RECORD,,\\n'
with ZipFile('/aaqe-results/aaqe_fixture_dependency-1.0-py3-none-any.whl', 'w') as archive:
    for name, contents in files.items():
        archive.writestr(name, contents)
"""
        prepared = tool("prepare_environment", setup=[
            ["python", "-c", package],
            ["python", "-m", "pip", "install", "--no-index", "--disable-pip-version-check", wheel],
        ])
        assert not prepared["isError"], prepared
        assert prepared["structuredContent"]["result"]["environment_ready"], prepared
    stages = ["code-explorer", "bug-hunter", "test-designer", "test-developer", "test-validator"]
    if "--retry-developer" in arguments:
        stages.insert(4, "test-developer")
    if "--loop-test" in arguments or "--cancel-test" in arguments:
        stages = ["code-explorer", "code-explorer", "bug-hunter"]
    for stage in stages:
        if stage == "test-validator" and "--early-inline" in arguments:
            rerun = tool("execute_tests", test_ids=["negative-input"])
            assert not rerun["isError"], rerun
            assert rerun["structuredContent"]["execution"]["tests"][0]["junit"]["complete"], rerun
        result = tool("run_stage", stage=stage, instruction="Inspect current evidence and perform this role independently")
        if result["isError"]:
            tool("finish", status="blocked", reason=result["content"][0]["text"])
            break
    else:
        result=tool("finish", status="completed", reason="All current candidates were checked independently")
        assert not result["isError"], result
    print("Audit finished via harness tools; ready for follow-up", flush=True)
    for line in sys.stdin:
        status = tool("status")["structuredContent"]["state"]
        assert status["finished_at"], "Status did not reload the finalized report state"
        assert tool("run_stage", stage="code-explorer", instruction="after audit")["isError"]
        (directory / "follow-up.txt").write_text(line)
        print("Follow-up received", flush=True)
    child.stdin.close()
    assert child.wait(timeout=10) == 0


def main():
    arguments = sys.argv
    if "--output-schema" not in arguments:
        overrides = [arguments[i+1] for i, value in enumerate(arguments[:-1]) if value == "-c"]
        settings = dict(value.split("=", 1) for value in overrides)
        directory = Path(json.loads(settings["mcp_servers.aaqe.args"])[1]) / "stages/coordinator"
        return coordinate(arguments, directory)
    stage_directory = Path(arguments[arguments.index("--output-schema") + 1]).parent
    inputs = json.loads((stage_directory / "input.json").read_text())
    sys.stdin.read()
    stage = inputs.get("stage", stage_directory.name)
    if "--cancel-test" in arguments:
        import time
        child = subprocess.Popen(["sleep", "60"])
        (stage_directory / "child.pid").write_text(str(child.pid))
        (stage_directory / "worker.pid").write_text(str(__import__("os").getpid()))
        time.sleep(60)
    output = {
        "code-explorer": {
            "summary": "Example scope examined",
            "scope": ["app.py"],
            "references": [
                {
                    "id": "source-1",
                    "path": "app.py",
                    "line": 1,
                    "claim": "Implementation clamps only the upper bound; README specifies zero to ten",
                }
            ],
            "uncertainties": [],
            "environment": {
                "compose_files": ["compose.yaml"],
                "services": ["runner"],
                "runner_service": "runner",
                "runner_workdir": "/workspace",
                "external_containers": [],
                "readiness_timeout": 90,
            },
            "baseline": [
                ["python", "run_tests.py", "tests/test_baseline.py", "{result_dir}/baseline.xml"]
            ],
        },
        "bug-hunter": {
            "summary": "A negative value bypasses the lower bound",
            "candidates": [
                {
                    "id": "lower-bound",
                    "title": "Negative inputs remain negative",
                    "expected": "zero",
                    "observed": "negative value",
                    "trigger": "bounded_value(-1)",
                    "evidence_ids": ["source-1"],
                    "confidence": "high",
                }
            ],
            "excluded_areas": [],
        },
        "test-designer": {
            "summary": "Check the documented lower bound",
            "plans": [
                {
                    "candidate_id": "lower-bound",
                    "test_id": "negative-input",
                    "expectation": "zero",
                    "oracle": "README contract: inclusive zero to ten",
                    "steps": ["call bounded_value(-1)", "assert equal to zero"],
                    "reproduction": ["run the saved test"],
                    "blocked_reason": None,
                }
            ],
        },
    }
    if stage == "test-developer":
        source = "import unittest\nfrom app import bounded_value\n\nclass NegativeTest(unittest.TestCase):\n    def test_negative(self):\n        self.assertEqual(bounded_value(-1), 0)\n"
        if "--early-inline" in arguments:
            source = "from aaqe_fixture_dependency import VALUE\nassert VALUE == 17\n" + source
        if inputs.get("attempt", 1) > 1:
            assert inputs["execution"]["tests"], "Retry lost prior execution feedback"
            source = "# Fixture reviewed against actual execution feedback\n" + source
        change = "".join(
            difflib.unified_diff(
                [],
                source.splitlines(keepends=True),
                fromfile="/dev/null",
                tofile="b/tests/test_regression.py",
            )
        )
        output[stage] = {
            "summary": "Add lower-bound regression",
            "patch": change,
            "tests": [
                {
                    "id": "negative-input",
                    "candidate_id": "lower-bound",
                    "command": [
                        "python",
                        "run_tests.py",
                        "tests/test_regression.py",
                        "{result_dir}/junit.xml",
                    ],
                    "report": "junit.xml",
                    "expected_names": ["test_negative"],
                    "reproduction_commands": [],
                }
            ],
            "limitations": [],
        }
    if stage == "test-validator":
        evidence = str(Path(inputs["execution"]["tests"][0]["directory"]) / "execution.json")
        output[stage] = {
            "summary": "Target assertion demonstrates the documented lower-bound defect",
            "verdicts": [
                {
                    "candidate_id": "lower-bound",
                    "test_ids": ["negative-input"],
                    "status": "confirmed",
                    "rationale": "Baseline passes; regression reaches bounded_value(-1) and observes -1 instead of zero",
                    "evidence": [evidence],
                }
            ],
        }
    if "--early-inline" in arguments:
        assert inputs["environment_ready"], "Early preparation did not reach the worker"
        if stage == "code-explorer":
            output[stage]["environment"] = inputs["environment"]
        if stage == "test-developer":
            output[stage]["tests"][0]["baseline"] = [["python", "run_tests.py", "tests/test_baseline.py", "{result_dir}/baseline.xml"]]
    if "--loop-test" in arguments and stage == "bug-hunter":
        output[stage]["candidates"] = []
    Path(arguments[arguments.index("--output-last-message") + 1]).write_text(
        json.dumps(output[stage])
    )


if __name__ == "__main__":
    main()
