import copy

import jsonschema

from .config import ARGV, STRINGS
from .errors import AAQEError


def record(properties):
    return {
        "type": "object",
        "properties": properties,
        "required": list(properties),
        "additionalProperties": False,
    }


def sequence(items):
    return {"type": "array", "items": items}


TEXT = {"type": "string"}
IDENTIFIER = {"type": "string", "pattern": "^[A-Za-z0-9][A-Za-z0-9_.-]{0,99}$"}
REFERENCE = record(
    {"id": IDENTIFIER, "path": TEXT, "line": {"type": "integer", "minimum": 1}, "claim": TEXT}
)
ENVIRONMENT = record(
    {
        "compose_files": STRINGS,
        "services": STRINGS,
        "runner_service": TEXT,
        "runner_workdir": TEXT,
        "external_containers": STRINGS,
        "readiness_timeout": {"type": "integer", "minimum": 1},
    }
)
TEST = record(
    {
        "id": IDENTIFIER,
        "candidate_id": IDENTIFIER,
        "command": ARGV,
        "report": {"type": "string", "pattern": "^[A-Za-z0-9_.-]+\\.xml$"},
        "expected_names": {**STRINGS, "minItems": 1, "uniqueItems": True},
        "reproduction_commands": sequence(ARGV),
    }
)
SCHEMAS = {
    "code-explorer": record(
        {
            "summary": TEXT,
            "scope": STRINGS,
            "references": sequence(REFERENCE),
            "uncertainties": STRINGS,
            "environment": ENVIRONMENT,
            "baseline": sequence(ARGV),
        }
    ),
    "bug-hunter": record(
        {
            "summary": TEXT,
            "candidates": sequence(
                record(
                    {
                        "id": IDENTIFIER,
                        "title": TEXT,
                        "expected": TEXT,
                        "observed": TEXT,
                        "trigger": TEXT,
                        "evidence_ids": {**STRINGS, "minItems": 1},
                        "confidence": {"type": "string", "enum": ["high", "medium", "low"]},
                    }
                )
            ),
            "excluded_areas": STRINGS,
        }
    ),
    "test-designer": record(
        {
            "summary": TEXT,
            "plans": sequence(
                record(
                    {
                        "candidate_id": IDENTIFIER,
                        "test_id": IDENTIFIER,
                        "expectation": TEXT,
                        "oracle": TEXT,
                        "steps": STRINGS,
                        "reproduction": STRINGS,
                        "blocked_reason": {"type": ["string", "null"]},
                    }
                )
            ),
        }
    ),
    "test-developer": record(
        {
            "summary": TEXT,
            "patch": TEXT,
            "tests": sequence(TEST),
            "limitations": STRINGS,
        }
    ),
    "test-validator": record(
        {
            "summary": TEXT,
            "verdicts": sequence(
                record(
                    {
                        "candidate_id": IDENTIFIER,
                        "test_ids": STRINGS,
                        "status": {
                            "type": "string",
                            "enum": [
                                "confirmed",
                                "not_reproduced",
                                "blocked",
                                "invalid_test",
                                "flaky",
                            ],
                        },
                        "rationale": TEXT,
                        "evidence": STRINGS,
                    }
                )
            ),
        }
    ),
}


def schema(stage):
    document = copy.deepcopy(SCHEMAS[stage])

    def model_subset(value):
        if isinstance(value, dict):
            value.pop("uniqueItems", None)
            for child in value.values():
                model_subset(child)
        elif isinstance(value, list):
            for child in value:
                model_subset(child)

    model_subset(document)
    return document


def validate(stage, artifact):
    try:
        jsonschema.Draft202012Validator(SCHEMAS[stage]).validate(artifact)
    except jsonschema.ValidationError as error:
        raise AAQEError(f"Invalid {stage} output contract: {error.message}") from error
    return artifact


def unique(items, key, label):
    values = [item[key] for item in items]
    if len(values) != len(set(values)):
        raise AAQEError(f"Duplicate {label} identity")


def validate_links(stage, artifact, artifacts):
    if stage == "code-explorer":
        unique(artifact["references"], "id", "reference")
    if stage == "bug-hunter":
        unique(artifact["candidates"], "id", "candidate")
        references = {item["id"] for item in artifacts["code-explorer"]["references"]}
        for candidate in artifact["candidates"]:
            if not set(candidate["evidence_ids"]) <= references:
                raise AAQEError("Candidate references unknown source evidence")
    if stage == "test-designer":
        unique(artifact["plans"], "test_id", "test plan")
        candidates = {item["id"] for item in artifacts["bug-hunter"]["candidates"]}
        if {item["candidate_id"] for item in artifact["plans"]} != candidates:
            raise AAQEError("Test plans must cover every candidate")
    if stage == "test-developer":
        unique(artifact["tests"], "id", "test")
        plans = {
            item["test_id"]: item
            for item in artifacts["test-designer"]["plans"]
            if not item["blocked_reason"]
        }
        if set(plans) != {item["id"] for item in artifact["tests"]}:
            raise AAQEError("Generated tests must match all unblocked test plans")
        if any(
            plans[item["id"]]["candidate_id"] != item["candidate_id"] for item in artifact["tests"]
        ):
            raise AAQEError("Generated test refers to the wrong candidate")
    if stage == "test-validator":
        unique(artifact["verdicts"], "candidate_id", "verdict")
        candidates = {item["id"] for item in artifacts["bug-hunter"]["candidates"]}
        if {item["candidate_id"] for item in artifact["verdicts"]} != candidates:
            raise AAQEError("Validation must cover every candidate")
        tests = {item["id"]: item for item in artifacts.get("test-developer", {}).get("tests", [])}
        for verdict in artifact["verdicts"]:
            if any(
                test_id not in tests or tests[test_id]["candidate_id"] != verdict["candidate_id"]
                for test_id in verdict["test_ids"]
            ):
                raise AAQEError("Verdict references an unrelated or unknown test")
