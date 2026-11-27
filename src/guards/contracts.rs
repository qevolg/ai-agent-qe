use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

fn full_schema(stage: &str) -> Result<Value> {
    let (_, _, text) = crate::agents::stage::Stage::parse(stage)?.resources();
    Ok(serde_json::from_str(text)?)
}

pub fn schema(stage: &str) -> Result<Value> {
    fn model_subset(value: &mut Value) {
        match value {
            Value::Object(map) => {
                map.remove("uniqueItems");
                if let Some(properties) = map.get_mut("properties").and_then(Value::as_object_mut) {
                    if let Some(compose) = properties.get_mut("compose") {
                        *compose = serde_json::json!({"type":["string","null"]});
                    }
                    let required = properties.keys().cloned().collect::<Vec<_>>();
                    map.insert("required".into(), serde_json::json!(required));
                    map.insert("additionalProperties".into(), serde_json::json!(false));
                }
                for value in map.values_mut() {
                    model_subset(value);
                }
            }
            Value::Array(items) => {
                for item in items {
                    model_subset(item);
                }
            }
            _ => {}
        }
    }
    let mut schema = full_schema(stage)?;
    model_subset(&mut schema);
    Ok(schema)
}

pub fn validate(stage: &str, artifact: &Value) -> Result<()> {
    let schema = full_schema(stage)?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|e| anyhow::anyhow!("Invalid {stage} output schema: {e}"))?;
    if let Err(error) = validator.validate(artifact) {
        bail!("Invalid {stage} output contract: {error}");
    }
    Ok(())
}

fn items<'a>(artifact: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    artifact[key]
        .as_array()
        .with_context(|| format!("Invalid artifact array: {key}"))
}

fn unique(items: &[Value], key: &str, label: &str) -> Result<()> {
    let mut seen = HashSet::new();
    for item in items {
        let value = item[key]
            .as_str()
            .with_context(|| format!("Invalid {label} identity"))?;
        if !seen.insert(value) {
            bail!("Duplicate {label} identity");
        }
    }
    Ok(())
}

fn identities<'a>(items: &'a [Value], key: &str) -> HashSet<&'a str> {
    items.iter().filter_map(|item| item[key].as_str()).collect()
}

pub fn validate_links(stage: &str, artifact: &Value, artifacts: &Value) -> Result<()> {
    match stage {
        "code-explorer" => unique(items(artifact, "references")?, "id", "reference")?,
        "bug-hunter" => {
            let candidates = items(artifact, "candidates")?;
            unique(candidates, "id", "candidate")?;
            let references = identities(items(&artifacts["code-explorer"], "references")?, "id");
            for candidate in candidates {
                for evidence in items(candidate, "evidence_ids")? {
                    if !evidence.as_str().is_some_and(|id| references.contains(id)) {
                        bail!("Candidate references unknown source evidence");
                    }
                }
            }
        }
        "test-designer" => {
            let plans = items(artifact, "plans")?;
            unique(plans, "test_id", "test plan")?;
            let candidates = identities(items(&artifacts["bug-hunter"], "candidates")?, "id");
            if identities(plans, "candidate_id") != candidates {
                bail!("Test plans must cover every candidate");
            }
        }
        "test-developer" => {
            let tests = items(artifact, "tests")?;
            unique(tests, "id", "test")?;
            let plans: HashMap<_, _> = items(&artifacts["test-designer"], "plans")?
                .iter()
                .filter(|plan| plan["blocked_reason"].is_null() || plan["blocked_reason"] == "")
                .filter_map(|plan| plan["test_id"].as_str().map(|id| (id, plan)))
                .collect();
            if plans.keys().copied().collect::<HashSet<_>>() != identities(tests, "id") {
                bail!("Generated tests must match all unblocked test plans");
            }
            for test in tests {
                let plan = plans
                    .get(test["id"].as_str().context("Invalid test identity")?)
                    .context("Unknown test plan")?;
                if plan["candidate_id"] != test["candidate_id"] {
                    bail!("Generated test refers to the wrong candidate");
                }
            }
        }
        "test-validator" => {
            let verdicts = items(artifact, "verdicts")?;
            unique(verdicts, "candidate_id", "verdict")?;
            let candidates = identities(items(&artifacts["bug-hunter"], "candidates")?, "id");
            if identities(verdicts, "candidate_id") != candidates {
                bail!("Validation must cover every candidate");
            }
            let test_key = if artifacts.get("test-developer").is_some() {
                "id"
            } else {
                "test_id"
            };
            let sources = if test_key == "id" {
                &artifacts["test-developer"]["tests"]
            } else {
                &artifacts["test-designer"]["plans"]
            };
            let tests: HashMap<_, _> = sources
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|test| test[test_key].as_str().map(|id| (id, test)))
                .collect();
            for verdict in verdicts {
                for id in items(verdict, "test_ids")? {
                    let Some(test) = id.as_str().and_then(|id| tests.get(id)) else {
                        bail!("Verdict references an unrelated or unknown test");
                    };
                    if test["candidate_id"] != verdict["candidate_id"] {
                        bail!("Verdict references an unrelated or unknown test");
                    }
                }
            }
        }
        _ => bail!("Unknown stage: {stage}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::guards::contracts;
    use serde_json::{Value, json};

    #[test]
    fn model_schema_omits_unsupported_uniqueness_but_host_validates_it() {
        let schema = contracts::schema("test-developer").unwrap();
        assert!(
            schema["properties"]["tests"]["items"]["properties"]["expected_names"]
                .get("uniqueItems")
                .is_none()
        );
        let mut artifact = json!({"summary":"regression", "patch":"diff", "tests":[{"id":"test-1","candidate_id":"bug-1","command":["test"],"report":"junit.xml","expected_names":["same","same"],"reproduction_commands":[]}],"limitations":[]});
        assert!(contracts::validate("test-developer", &artifact).is_err());
        artifact["tests"][0]["expected_names"] = json!(["same"]);
        contracts::validate("test-developer", &artifact).unwrap();
        artifact["unexpected"] = json!(true);
        assert!(contracts::validate("test-developer", &artifact).is_err());
    }

    #[test]
    fn links_reject_unknown_sources_candidates_and_tests() {
        let artifacts = json!({"code-explorer":{"references":[{"id":"source-1"}]},"bug-hunter":{"candidates":[{"id":"bug-1"}]},"test-designer":{"plans":[{"test_id":"test-1","candidate_id":"bug-1","blocked_reason":null}]},"test-developer":{"tests":[{"id":"test-1","candidate_id":"bug-1"}]}});
        assert!(
            contracts::validate_links(
                "bug-hunter",
                &json!({"candidates":[{"id":"bug-2","evidence_ids":["missing"]}]}),
                &artifacts
            )
            .is_err()
        );
        assert!(
            contracts::validate_links("test-designer", &json!({"plans":[]}), &artifacts).is_err()
        );
        assert!(
            contracts::validate_links(
                "test-developer",
                &json!({"tests":[{"id":"test-1","candidate_id":"wrong"}]}),
                &artifacts
            )
            .is_err()
        );
        assert!(
            contracts::validate_links(
                "test-validator",
                &json!({"verdicts":[{"candidate_id":"bug-1","test_ids":["missing"]}]}),
                &artifacts
            )
            .is_err()
        );
        contracts::validate_links(
            "test-validator",
            &json!({"verdicts":[{"candidate_id":"bug-1","test_ids":["test-1"]}]}),
            &artifacts,
        )
        .unwrap();
    }

    #[test]
    fn artifact_links_reject_duplicate_identities_at_every_stage() {
        let duplicate_cases = [
            (
                "code-explorer",
                json!({"references":[{"id":"source"},{"id":"source"}]}),
            ),
            (
                "bug-hunter",
                json!({"candidates":[{"id":"bug"},{"id":"bug"}]}),
            ),
            (
                "test-designer",
                json!({"plans":[{"test_id":"test"},{"test_id":"test"}]}),
            ),
            (
                "test-developer",
                json!({"tests":[{"id":"test"},{"id":"test"}]}),
            ),
            (
                "test-validator",
                json!({"verdicts":[{"candidate_id":"bug"},{"candidate_id":"bug"}]}),
            ),
        ];
        for (stage, artifact) in duplicate_cases {
            let error = contracts::validate_links(stage, &artifact, &json!({})).unwrap_err();
            assert!(error.to_string().contains("Duplicate"), "{stage}: {error}");
        }
    }

    #[test]
    fn blocked_plans_do_not_require_tests_and_verdicts_cannot_cross_candidates() {
        let artifacts = json!({
            "bug-hunter":{"candidates":[{"id":"bug-1"},{"id":"bug-2"}]},
            "test-designer":{"plans":[
                {"test_id":"blocked-test","candidate_id":"bug-1","blocked_reason":"missing environment"},
                {"test_id":"active-test","candidate_id":"bug-2","blocked_reason":null}
            ]},
            "test-developer":{"tests":[{"id":"active-test","candidate_id":"bug-2"}]}
        });
        contracts::validate_links(
            "test-developer",
            &json!({"tests":[{"id":"active-test","candidate_id":"bug-2"}]}),
            &artifacts,
        )
        .unwrap();
        assert!(contracts::validate_links("test-developer", &json!({"tests":[{"id":"blocked-test","candidate_id":"bug-1"},{"id":"active-test","candidate_id":"bug-2"}]}), &artifacts).is_err());
        assert!(
            contracts::validate_links(
                "test-validator",
                &json!({"verdicts":[
                    {"candidate_id":"bug-1","test_ids":["active-test"]},
                    {"candidate_id":"bug-2","test_ids":["active-test"]}
                ]}),
                &artifacts
            )
            .is_err()
        );
        contracts::validate_links(
            "test-validator",
            &json!({"verdicts":[
                {"candidate_id":"bug-1","test_ids":[]},
                {"candidate_id":"bug-2","test_ids":["active-test"]}
            ]}),
            &artifacts,
        )
        .unwrap();
    }

    #[test]
    fn model_schema_projects_optional_fields_without_breaking_legacy_host_artifacts() {
        fn strict(value: &Value) {
            if let Some(properties) = value.get("properties").and_then(Value::as_object) {
                let required = value["required"].as_array().unwrap();
                assert_eq!(properties.len(), required.len());
                assert_eq!(value["additionalProperties"], false);
                for key in properties.keys() {
                    assert!(required.iter().any(|item| item == key));
                }
            }
            match value {
                Value::Object(map) => {
                    for value in map.values() {
                        strict(value)
                    }
                }
                Value::Array(items) => {
                    for value in items {
                        strict(value)
                    }
                }
                _ => {}
            }
        }
        for stage in crate::agents::stage::STAGES {
            strict(&contracts::schema(stage).unwrap());
        }
        assert_eq!(
            contracts::schema("code-explorer").unwrap()["properties"]["environment"]["properties"]
                ["compose"],
            json!({"type":["string","null"]})
        );
    }
}
