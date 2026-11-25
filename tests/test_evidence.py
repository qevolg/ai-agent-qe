from aaqe.evidence import parse_junit


def test_missing_expected_case_is_unresolved_even_when_another_case_passes(tmp_path):
    report = tmp_path / "junit.xml"
    report.write_text('<testsuite tests="1"><testcase name="unrelated"/></testsuite>')
    evidence = parse_junit(report, ["regression"])
    assert evidence["complete"] is False
    assert evidence["missing"] == ["regression"]


def test_failed_target_and_skipped_target_have_different_results(tmp_path):
    report = tmp_path / "junit.xml"
    report.write_text(
        '<testsuite><testcase classname="suite" name="regression"><failure message="wrong value"/></testcase><testcase name="other"><skipped/></testcase></testsuite>'
    )
    evidence = parse_junit(report, ["suite::regression", "other"])
    assert evidence["cases"][0]["status"] == "failed"
    assert evidence["cases"][0]["detail"] == "wrong value"
    assert evidence["complete"] is False


def test_duplicate_case_names_require_unambiguous_identity(tmp_path):
    report = tmp_path / "junit.xml"
    report.write_text(
        '<testsuite><testcase classname="one" name="regression"/><testcase classname="two" name="regression"/></testsuite>'
    )
    assert parse_junit(report, ["regression"])["complete"] is False
    assert parse_junit(report, ["two::regression"])["complete"] is True


def test_missing_report_and_gitlab_placeholder_are_incomplete(tmp_path):
    report = tmp_path / "missing.xml"
    assert parse_junit(report, ["regression"])["complete"] is False
    report.write_text(
        '<testsuite><testcase name="ctest-incomplete"><failure/></testcase></testsuite>'
    )
    assert parse_junit(report, ["regression"])["complete"] is False


def test_disabled_and_expected_failure_cannot_confirm_a_bug(tmp_path):
    report = tmp_path / "junit.xml"
    report.write_text('<testsuite><testcase name="regression" status="notrun"/></testsuite>')
    assert parse_junit(report, ["regression"])["complete"] is False


def test_empty_expected_case_set_is_not_success(tmp_path):
    report = tmp_path / "junit.xml"
    report.write_text("<testsuite/>")
    assert parse_junit(report, [])["complete"] is False
