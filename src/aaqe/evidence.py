import xml.etree.ElementTree as ET
from pathlib import Path


def parse_junit(path, expected):
    result = {"complete": False, "cases": [], "missing": [], "errors": []}
    if not expected:
        result["errors"].append("expected_case_names_empty")
    try:
        root = ET.parse(Path(path)).getroot()
    except (OSError, ET.ParseError) as error:
        result["errors"].append(f"report_unavailable: {error}")
        result["missing"] = list(expected)
        return result
    all_cases = list(root.iter("testcase"))
    for name in expected:
        matches = [
            case
            for case in all_cases
            if name in (case.get("name"), (case.get("classname", "") + "::" + case.get("name", "")))
        ]
        if len(matches) != 1:
            result["missing"].append(name)
            result["errors"].append("missing_or_ambiguous_case: " + name)
            continue
        case = matches[0]
        if case.get("name") in ("ctest-incomplete",) or case.get("classname") == "worker-admission":
            result["missing"].append(name)
            result["errors"].append("placeholder_case: " + name)
            continue
        failure = case.find("failure")
        error = case.find("error")
        skipped = case.find("skipped") is not None or case.get("status") in (
            "notrun",
            "disabled",
            "skipped",
        )
        detail = failure if failure is not None else error
        status = (
            "skipped"
            if skipped
            else "error"
            if error is not None
            else "failed"
            if failure is not None
            else "passed"
        )
        result["cases"].append(
            {
                "name": name,
                "status": status,
                "detail": (
                    (detail.get("message", "") + "\n" + (detail.text or "")).strip()
                    if detail is not None
                    else ""
                ),
                "time": case.get("time", ""),
            }
        )
    result["complete"] = (
        bool(expected)
        and not result["missing"]
        and not result["errors"]
        and all(case["status"] in ("passed", "failed") for case in result["cases"])
    )
    return result
