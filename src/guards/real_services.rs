use anyhow::{Result, ensure};
use regex::Regex;
use serde_json::Value;
use std::{collections::HashSet, sync::OnceLock};

/// An early check for common test-double APIs. Independent validation also
/// inspects helpers, aliases and handwritten substitutes; text cannot prove
/// that an arbitrary program uses a real service.
pub fn reject_mocks(path: &str, source: &str) -> Result<()> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(concat!(
            r"\b(?:org\.mockito|org\.easymock|org\.powermock|unittest\.mock|",
            r"mockall|mockito|gomock|sqlmock|go-sqlmock|testify/mock|",
            r"wiremock|WireMock|MockWebServer|httptest\.NewServer|",
            r"MagicMock|AsyncMock|MockK|mockk|respx|requests_mock)\b|",
            r"\b(?:jest|vi|sinon)\s*\.\s*(?:mock|fn|spyOn|spy|stub|replace)\s*\(|",
            r"\b(?:mock|spy|stub|create_autospec|createMock|niceMock|strictMock)\s*[(<]|",
            r"\b(?:monkeypatch\s*\.\s*(?:setattr|setitem)|patch\s*\.\s*object)\s*\(|",
            r"\b(?:responses|respx)\s*\.\s*(?:activate|mock|add|route)\b|",
            r#"(?:from\s*|require\s*\(\s*)['\"](?:nock|sinon|msw|mock-require|proxyquire)['\"]"#
        ))
        .expect("valid test-double pattern")
    });
    for (line, text) in source.lines().enumerate() {
        let text = text.trim_start();
        if ["//", "# ", "/*", "*"]
            .iter()
            .any(|prefix| text.starts_with(prefix))
        {
            continue;
        }
        ensure!(
            !pattern.is_match(text),
            "Prohibited mock/stub/spy API in {path}:{}; use the real implementation and real services",
            line + 1
        );
    }
    Ok(())
}

pub fn check_verified(check: &Value) -> bool {
    check["result"]["returncode"] == 0
        && check["result"]["timed_out"] == false
        && check["directory"]
            .as_str()
            .is_some_and(|path| !path.is_empty())
        && check["docker"]["service"] == check["service"]
        && check["docker"]["project"]
            .as_str()
            .is_some_and(|project| !project.is_empty())
        && check["docker"]["containers"]
            .as_array()
            .is_some_and(|containers| {
                !containers.is_empty()
                    && containers.iter().all(|container| {
                        container["running"] == true
                            && container["project"] == check["docker"]["project"]
                            && container["service"] == check["service"]
                            && ["id", "image_id", "image"].iter().all(|field| {
                                container[field]
                                    .as_str()
                                    .is_some_and(|value| !value.is_empty())
                            })
                    })
            })
}

pub fn checks_verified(record: &Value) -> bool {
    let (Some(required), Some(checks)) = (
        record["required_services"].as_array(),
        record["service_checks"].as_array(),
    ) else {
        return false;
    };
    let required_count = required.len();
    let required: HashSet<_> = required.iter().filter_map(Value::as_str).collect();
    let names: HashSet<_> = checks
        .iter()
        .filter_map(|check| check["service"].as_str())
        .collect();
    required == names
        && required.len() == required_count
        && checks.len() == names.len()
        && checks.iter().all(check_verified)
}

pub fn approved(execution: &Value, artifacts: &Value, grounded_verdicts: &Value) -> bool {
    let Some(tests) = artifacts["test-developer"]["tests"].as_array() else {
        return true;
    };
    tests.iter().all(|test| {
        execution["tests"].as_array().is_some_and(|records| {
            records.iter().any(|record| {
                let expected = test["service_checks"].as_array().map(|checks| {
                    checks
                        .iter()
                        .map(|check| check["service"].clone())
                        .collect::<Vec<_>>()
                });
                record["test_id"] == test["id"]
                    && record["target_executed"] == true
                    && record["evidence"]["complete"] == true
                    && expected.is_some_and(|expected| {
                        record["required_services"] == serde_json::json!(expected)
                    })
                    && checks_verified(record)
            })
        }) && grounded_verdicts.as_array().is_some_and(|verdicts| {
            verdicts.iter().any(|verdict| {
                verdict["test_ids"]
                    .as_array()
                    .is_some_and(|ids| ids.contains(&test["id"]))
                    && verdict["assessment"]["no_mocks"] == true
                    && verdict["assessment"]["real_services_verified"] == true
                    && verdict["assessment"]["executed"] == true
                    && verdict["assessment"]["infrastructure_failure"] == false
                    && matches!(
                        verdict["status"].as_str(),
                        Some("confirmed" | "not_reproduced" | "flaky")
                    )
            })
        })
    })
}
