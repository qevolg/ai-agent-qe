use super::*;
use std::fs;
fn evidence(xml: &str, names: &[&str]) -> serde_json::Value {
    let directory = tempfile::tempdir().unwrap();
    let report = directory.path().join("junit.xml");
    fs::write(&report, xml).unwrap();
    parse_junit(
        &report,
        &names
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>(),
    )
}

#[test]
fn junit_missing_case_and_empty_expectations_are_incomplete() {
    let result = evidence(
        "<testsuite><testcase name=\"unrelated\"/></testsuite>",
        &["regression"],
    );
    assert_eq!(result["complete"], false);
    assert_eq!(result["missing"], json!(["regression"]));
    assert_eq!(evidence("<testsuite/>", &[])["complete"], false);
}

#[test]
fn junit_failed_target_retains_detail_while_skipped_is_incomplete() {
    let result = evidence(
        "<testsuite><testcase classname=\"suite\" name=\"regression\"><failure message=\"wrong value\">assertion details</failure></testcase><testcase name=\"other\"><skipped/></testcase></testsuite>",
        &["suite::regression", "other"],
    );
    assert_eq!(result["cases"][0]["status"], "failed");
    assert_eq!(
        result["cases"][0]["detail"],
        "wrong value\nassertion details"
    );
    assert_eq!(result["cases"][1]["status"], "skipped");
    assert_eq!(result["complete"], false);
}

#[test]
fn junit_duplicate_identity_requires_classname() {
    let xml = "<testsuite><testcase classname=\"one\" name=\"regression\"/><testcase classname=\"two\" name=\"regression\"/></testsuite>";
    assert_eq!(evidence(xml, &["regression"])["complete"], false);
    assert_eq!(evidence(xml, &["two::regression"])["complete"], true);
}

#[test]
fn junit_utf16_reports_preserve_target_case_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("junit.xml");
    let source = "<?xml version=\"1.0\" encoding=\"utf-16\"?><testsuite><testcase name=\"回归\"/></testsuite>";
    let mut bytes = vec![0xff, 0xfe];
    for character in source.encode_utf16() {
        bytes.extend_from_slice(&character.to_le_bytes());
    }
    fs::write(&path, bytes).unwrap();
    assert_eq!(parse_junit(&path, &["回归".into()])["complete"], true);
}

#[test]
fn junit_latin1_reports_preserve_accented_identity_and_failure_details() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("junit.xml");
    fs::write(&path, b"<?xml version='1.0' encoding='ISO-8859-1'?><testsuite><testcase name='caf\xe9'><failure message='d\xe9faut'>r\xe9sultat\x80</failure></testcase></testsuite>").unwrap();
    let parsed = parse_junit(&path, &["café".into()]);
    assert_eq!(parsed["complete"], true);
    assert_eq!(parsed["cases"][0]["detail"], "défaut\nrésultat\u{80}");
}

#[test]
fn junit_declared_windows1252_decodes_without_latin1_aliasing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("junit.xml");
    fs::write(&path, b"<?xml version='1.0' encoding='windows-1252'?><testsuite><testcase name='cost-\x80'/></testsuite>").unwrap();
    let parsed = parse_junit(&path, &["cost-€".into()]);
    assert_eq!(parsed["complete"], true);
}

#[test]
fn junit_declared_encodings_reject_invalid_bytes_instead_of_replacing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("junit.xml");
    for (encoding, invalid) in [
        ("ASCII", b"\xe9".as_slice()),
        ("UTF-8", b"\xff".as_slice()),
        ("Shift_JIS", b"\x82".as_slice()),
    ] {
        let mut bytes = format!("<?xml version='1.0' encoding='{encoding}'?><testsuite><testcase name='regression'><failure>").into_bytes();
        bytes.extend_from_slice(invalid);
        bytes.extend_from_slice(b"</failure></testcase></testsuite>");
        fs::write(&path, bytes).unwrap();
        let parsed = parse_junit(&path, &["regression".into()]);
        assert_eq!(parsed["complete"], false, "{encoding}");
        assert_eq!(parsed["cases"], json!([]), "{encoding}");
    }
}

#[test]
fn junit_internal_entities_expand_in_target_identities_and_details() {
    let result = evidence(
        "<!DOCTYPE testsuite [<!ENTITY label 'café'><!ENTITY detail 'résultat attendu'>]><testsuite><testcase name='&label;'><failure>&detail;</failure></testcase></testsuite>",
        &["café"],
    );
    assert_eq!(result["complete"], true);
    assert_eq!(result["cases"][0]["detail"], "résultat attendu");
}

#[test]
fn junit_external_entities_remain_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    let external = directory.path().join("external.txt");
    fs::write(&external, "sensitive external content").unwrap();
    let source = format!(
        "<!DOCTYPE testsuite [<!ENTITY external SYSTEM 'file://{}'>]><testsuite><testcase name='regression'><failure>&external;</failure></testcase></testsuite>",
        external.display()
    );
    let result = evidence(&source, &["regression"]);
    assert_eq!(result["complete"], false);
    assert_eq!(result["cases"], json!([]));
}

#[test]
fn junit_placeholder_disabled_error_and_malformed_report_are_incomplete() {
    for xml in [
        "<testsuite><testcase name=\"ctest-incomplete\"><failure/></testcase></testsuite>",
        "<testsuite><testcase name=\"regression\" status=\"notrun\"/></testsuite>",
        "<testsuite><testcase name=\"regression\"><error/></testcase></testsuite>",
        "<testsuite><testcase classname=\"worker-admission\" name=\"regression\"/></testsuite>",
        "<not-xml",
    ] {
        assert_eq!(evidence(xml, &["regression"])["complete"], false, "{xml}");
    }
    assert_eq!(
        parse_junit(
            Path::new("/does-not-exist/aaqe-junit.xml"),
            &["regression".into()]
        )["complete"],
        false
    );
}
