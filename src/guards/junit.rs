use std::{fs, path::Path};

use serde_json::{Value, json};

fn declared_encoding(bytes: &[u8]) -> anyhow::Result<Option<&str>> {
    let content = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    if !content.starts_with(b"<?xml") {
        return Ok(None);
    }
    let Some(end) = content.windows(2).position(|pair| pair == b"?>") else {
        return Ok(None);
    };
    let declaration = &content[..end];
    let expression = regex::bytes::Regex::new(
        r#"\bencoding\s*=\s*(?:"([A-Za-z][A-Za-z0-9._-]*)"|'([A-Za-z][A-Za-z0-9._-]*)')"#,
    )?;
    let Some(captures) = expression.captures(declaration) else {
        return Ok(None);
    };
    let name = captures.get(1).or_else(|| captures.get(2)).unwrap();
    Ok(Some(std::str::from_utf8(name.as_bytes())?))
}

fn read_xml(path: &Path) -> anyhow::Result<String> {
    let bytes = fs::read(path)?;
    let utf16 = if bytes.starts_with(&[0xff, 0xfe]) {
        Some((&bytes[2..], true))
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        Some((&bytes[2..], false))
    } else if bytes.starts_with(&[b'<', 0, b'?', 0]) {
        Some((bytes.as_slice(), true))
    } else if bytes.starts_with(&[0, b'<', 0, b'?']) {
        Some((bytes.as_slice(), false))
    } else {
        None
    };
    if let Some((content, little_endian)) = utf16 {
        anyhow::ensure!(content.len() % 2 == 0, "Invalid UTF-16 XML length");
        let characters = content.as_chunks::<2>().0.iter().map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        });
        return char::decode_utf16(characters)
            .collect::<Result<String, _>>()
            .map_err(Into::into);
    }
    let Some(label) = declared_encoding(&bytes)? else {
        return String::from_utf8(bytes).map_err(Into::into);
    };
    let normalized = label.to_ascii_lowercase().replace(['-', '_', '.'], "");
    if matches!(
        normalized.as_str(),
        "ascii"
            | "usascii"
            | "us"
            | "iso646us"
            | "isoir6"
            | "ansix341968"
            | "ansix341986"
            | "cp367"
            | "ibm367"
            | "csascii"
            | "646"
    ) {
        anyhow::ensure!(bytes.is_ascii(), "Non-ASCII byte in ASCII XML report");
        return String::from_utf8(bytes).map_err(Into::into);
    }
    // WHATWG maps Latin-1 labels to Windows-1252. XML's ISO-8859-1 must keep
    // the actual Latin-1 code points, including the U+0080..U+009F controls.
    if matches!(
        normalized.as_str(),
        "iso88591"
            | "iso8859"
            | "latin1"
            | "latin"
            | "l1"
            | "cp819"
            | "ibm819"
            | "csisolatin1"
            | "8859"
    ) {
        return Ok(bytes.into_iter().map(char::from).collect());
    }
    let encoding = encoding_rs::Encoding::for_label(label.as_bytes())
        .ok_or_else(|| anyhow::anyhow!("Unsupported XML encoding: {label}"))?;
    encoding
        .decode_without_bom_handling_and_without_replacement(&bytes)
        .map(|source| source.into_owned())
        .ok_or_else(|| anyhow::anyhow!("Invalid byte sequence in {label} XML report"))
}

pub fn parse_junit(path: &Path, expected: &[String]) -> Value {
    let mut errors = Vec::new();
    let mut missing = Vec::new();
    let mut cases = Vec::new();
    if expected.is_empty() {
        errors.push("expected_case_names_empty".to_string());
    }
    let source = match read_xml(path) {
        Ok(source) => source,
        Err(error) => {
            errors.push(format!("report_unavailable: {error}"));
            return json!({"complete":false,"cases":cases,"missing":expected,"errors":errors});
        }
    };
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let document = match roxmltree::Document::parse_with_options(&source, options) {
        Ok(document) => document,
        Err(error) => {
            errors.push(format!("report_unavailable: {error}"));
            return json!({"complete":false,"cases":cases,"missing":expected,"errors":errors});
        }
    };
    let all_cases: Vec<_> = document
        .descendants()
        .filter(|node| {
            node.is_element()
                && node.tag_name().namespace().is_none()
                && node.tag_name().name() == "testcase"
        })
        .collect();
    for name in expected {
        let matches: Vec<_> = all_cases
            .iter()
            .filter(|case| {
                case.attribute("name") == Some(name.as_str())
                    || format!(
                        "{}::{}",
                        case.attribute("classname").unwrap_or(""),
                        case.attribute("name").unwrap_or("")
                    ) == *name
            })
            .collect();
        if matches.len() != 1 {
            missing.push(name.clone());
            errors.push(format!("missing_or_ambiguous_case: {name}"));
            continue;
        }
        let case = matches[0];
        if case.attribute("name") == Some("ctest-incomplete")
            || case.attribute("classname") == Some("worker-admission")
        {
            missing.push(name.clone());
            errors.push(format!("placeholder_case: {name}"));
            continue;
        }
        let child = |tag: &str| {
            case.children().find(|node| {
                node.is_element()
                    && node.tag_name().namespace().is_none()
                    && node.tag_name().name() == tag
            })
        };
        let failure = child("failure");
        let error = child("error");
        let skipped = child("skipped").is_some()
            || matches!(
                case.attribute("status"),
                Some("notrun" | "disabled" | "skipped")
            );
        let detail = failure
            .or(error)
            .map(|detail| {
                format!(
                    "{}\n{}",
                    detail.attribute("message").unwrap_or(""),
                    detail.text().unwrap_or("")
                )
                .trim()
                .to_string()
            })
            .unwrap_or_default();
        let status = if skipped {
            "skipped"
        } else if error.is_some() {
            "error"
        } else if failure.is_some() {
            "failed"
        } else {
            "passed"
        };
        cases.push(json!({"name":name,"status":status,"detail":detail,"time":case.attribute("time").unwrap_or("")}));
    }
    let complete = !expected.is_empty()
        && missing.is_empty()
        && errors.is_empty()
        && cases
            .iter()
            .all(|case| matches!(case["status"].as_str(), Some("passed" | "failed")));
    json!({"complete":complete,"cases":cases,"missing":missing,"errors":errors})
}

#[cfg(test)]
mod tests {
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
}
