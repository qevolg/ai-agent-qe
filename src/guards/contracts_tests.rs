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
    assert!(contracts::validate_links("test-designer", &json!({"plans":[]}), &artifacts).is_err());
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
        contracts::schema("code-explorer").unwrap()["properties"]["environment"]["properties"]["compose"],
        json!({"type":["string","null"]})
    );
}
