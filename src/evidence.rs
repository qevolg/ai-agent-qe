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
        let characters = content.chunks_exact(2).map(|pair| {
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
