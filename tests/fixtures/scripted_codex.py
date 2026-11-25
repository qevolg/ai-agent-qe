"""Deterministic model boundary for integration tests; never used by real audits."""

import difflib
import json
import sys
from pathlib import Path


def main():
    arguments = sys.argv
    stage_directory = Path(arguments[arguments.index("--output-schema") + 1]).parent
    stage = stage_directory.name
    inputs = json.loads((stage_directory / "input.json").read_text())
    sys.stdin.read()
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
    Path(arguments[arguments.index("--output-last-message") + 1]).write_text(
        json.dumps(output[stage])
    )


if __name__ == "__main__":
    main()
