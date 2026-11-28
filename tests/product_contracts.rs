use aaqe::agents::stage::Stage;
use aaqe::guards::contracts;
use serde_json::{Value, json};

fn explorer() -> Value {
    json!({
        "summary": "Verify query behavior through SQL and Explorer",
        "scope": ["query"],
        "references": [{"id":"ref-query", "path":"docs/query.md", "line":1, "claim":"Queries return stored values"}],
        "uncertainties": [],
        "entry_points": [
            {"id":"entry-sql", "kind":"sql", "description":"SQL client", "applicable":true, "reason":"Supported query entry point"},
            {"id":"entry-ui", "kind":"explorer", "description":"Query page", "applicable":true, "reason":"Requested browser behavior"}
        ],
        "services": [
            {"id":"database", "name":"Database", "source":"external", "endpoint":"http://db.example:6041", "compose_service":null, "version":null, "check":["curl", "http://db.example:6041/version"]},
            {"id":"explorer", "name":"Explorer", "source":"docker", "endpoint":null, "compose_service":"explorer", "version":"1.0", "check":["agent-browser", "open", "http://explorer:6060"]}
        ],
        "environment": {
            "compose_files": [], "services":["runner", "explorer"],
            "runner_service":"runner", "runner_workdir":"/workspace",
            "external_containers":[], "readiness_timeout":30,
            "compose":{"services":{"runner":{"image":"runner:1"}, "explorer":{"image":"explorer:1"}}},
            "readiness":[["agent-browser", "--version"]], "setup":[]
        }
    })
}

fn designer() -> Value {
    json!({
        "summary":"Document query scenarios before execution",
        "scenarios":[{
            "id":"query-normal", "title":"Read an inserted value", "feature":"query",
            "entry_point":"sql", "category":"normal", "preconditions":["database is reachable"],
            "services":["database"], "oracle":"The result contains the inserted value",
            "steps":[{"id":"select", "action":"Query the prepared table", "expected":"One row with value 42"}],
            "cleanup":[], "blocked_reason":null, "derived_from":null, "reason":null
        }],
        "coverage":[{"feature":"query", "entry_point":"sql", "category":"normal", "status":"planned", "rationale":"Supported documented behavior", "scenario_ids":["query-normal"]}],
        "limitations":[]
    })
}

fn executor() -> Value {
    json!({
        "summary":"Execute the documented SELECT step",
        "operations":[{"id":"query-op", "scenario_id":"query-normal", "step_id":"select", "command":["taos", "-s", "select value from test.t"], "purpose":"check"}],
        "observations":[], "limitations":[]
    })
}

fn reproducer() -> Value {
    let mut artifact = executor();
    artifact["issues"] = json!([{
        "id":"issue-value", "title":"Stored value differs", "scenario_ids":["query-normal"],
        "expected":"42", "observed":"41", "trigger":"SELECT after inserting 42",
        "reproduction":["Insert 42", "Read the stored value"],
        "evidence":["execution/query-op/stdout.txt"]
    }]);
    artifact["followups"] =
        json!([{"scenario_id":"query-normal", "reason":"Repeat after reconnecting"}]);
    artifact
}

fn reviewer() -> Value {
    json!({
        "summary":"Independent review of operation evidence",
        "scenario_results":[{
            "scenario_id":"query-normal", "status":"failed", "rationale":"Observed 41 rather than 42",
            "evidence":["execution/query-op/stdout.txt"],
            "assessment":{"executed":true, "expectation_met":false, "infrastructure_failure":false, "no_mocks":true, "real_services_verified":true, "entry_point_verified":true}
        }],
        "issue_results":[{"issue_id":"issue-value", "status":"confirmed", "rationale":"The operation demonstrated the mismatch", "evidence":["execution/query-op/stdout.txt"]}],
        "limitations":[]
    })
}

fn accepts(stage: &str, artifact: &Value) {
    contracts::validate(stage, artifact).unwrap_or_else(|error| panic!("{stage}: {error:#}"));
}

fn rejects(stage: &str, artifact: &Value) {
    let error = contracts::validate(stage, artifact).expect_err("invalid artifact was accepted");
    assert!(
        error.to_string().contains("output contract"),
        "expected schema rejection for {stage}, got {error:#}"
    );
}

#[test]
fn product_roles_load_distinct_contracts_without_removing_legacy_roles() {
    for name in [
        "product-explorer",
        "scenario-designer",
        "product-executor",
        "issue-reproducer",
        "result-reviewer",
        "code-explorer",
        "bug-hunter",
        "test-designer",
        "test-developer",
        "test-validator",
    ] {
        let stage = Stage::parse(name).unwrap_or_else(|error| panic!("{name}: {error}"));
        let (role, skill, schema) = stage.resources();
        assert!(!role.is_empty() && !skill.is_empty());
        jsonschema::validator_for(&serde_json::from_str::<Value>(schema).unwrap()).unwrap();
        assert_eq!(stage.name(), name);
    }
}

#[test]
fn product_explorer_accepts_external_and_docker_service_inventory() {
    let mut artifact = explorer();
    accepts("product-explorer", &artifact);
    artifact["environment"]["compose"] = Value::Null;
    artifact["environment"]["compose_files"] = json!(["compose.yaml"]);
    accepts("product-explorer", &artifact);
    artifact["real_services"] = json!(["database"]);
    rejects("product-explorer", &artifact);
}

#[test]
fn external_endpoint_must_match_the_whole_prompt_address() {
    use aaqe::guards::product::validate_services;
    for prompt in [
        "地址：http://db.example:6041",
        "服务地址 (http://db.example:6041)，请验证",
        "http://db.example:6041\n测试",
    ] {
        validate_services(&explorer(), prompt).unwrap();
    }
    for prompt in [
        "地址 http://db.example:60410",
        "地址 http://db.example:6041/other",
        "地址 http://db.example:6041.evil",
    ] {
        assert!(
            validate_services(&explorer(), prompt).is_err(),
            "错误接受了地址前缀：{prompt}"
        );
    }
}

#[test]
fn product_service_contract_rejects_empty_probes_and_unknown_entry_points() {
    for (pointer, invalid) in [
        ("/services/0/check", json!([])),
        ("/services/0/source", json!("mock")),
        ("/services/0/id", json!("../database")),
        ("/entry_points/0/kind", json!("tcp")),
        ("/entry_points/0/applicable", json!("yes")),
        ("/references/0/line", json!(0)),
    ] {
        let mut artifact = explorer();
        *artifact.pointer_mut(pointer).unwrap() = invalid;
        rejects("product-explorer", &artifact);
    }
}

#[test]
fn scenario_designer_accepts_functional_plans_without_defect_candidates() {
    let mut artifact = designer();
    accepts("scenario-designer", &artifact);
    for category in [
        "normal",
        "abnormal",
        "boundary",
        "state",
        "permissions",
        "data",
        "cross_entry",
    ] {
        artifact["scenarios"][0]["category"] = json!(category);
        artifact["coverage"][0]["category"] = json!(category);
        accepts("scenario-designer", &artifact);
    }
    for entry_point in ["sql", "api", "explorer", "cli", "cross_entry"] {
        artifact["scenarios"][0]["entry_point"] = json!(entry_point);
        artifact["coverage"][0]["entry_point"] = json!(entry_point);
        accepts("scenario-designer", &artifact);
    }
    artifact["scenarios"][0]["derived_from"] = json!("execution/query-op/stdout.txt");
    artifact["scenarios"][0]["reason"] = json!("New in-scope observation");
    artifact["scenarios"][0]["blocked_reason"] = json!("Endpoint unavailable");
    accepts("scenario-designer", &artifact);
}

#[test]
fn scenario_contract_requires_addressable_steps_and_behavioral_oracles() {
    for (pointer, invalid) in [
        ("/scenarios/0/id", json!("../../query")),
        ("/scenarios/0/steps", json!([])),
        ("/scenarios/0/steps/0/id", json!("")),
        ("/scenarios/0/steps/0/expected", json!("")),
        ("/scenarios/0/oracle", json!("")),
        ("/scenarios/0/services/0", json!("../database")),
        ("/scenarios/0/category", json!("coverage")),
        ("/coverage/0/status", json!("passed")),
        ("/coverage/0/scenario_ids/0", json!("../query")),
    ] {
        let mut artifact = designer();
        *artifact.pointer_mut(pointer).unwrap() = invalid;
        rejects("scenario-designer", &artifact);
    }
}

#[test]
fn product_operations_require_argv_and_documented_identity_fields() {
    accepts("product-executor", &executor());
    for (pointer, invalid) in [
        ("/operations/0/command", json!([])),
        ("/operations/0/command", json!("taos -s select")),
        ("/operations/0/command/0", json!("")),
        ("/operations/0/id", json!("../operation")),
        ("/operations/0/scenario_id", json!("")),
        ("/operations/0/step_id", json!("../step")),
        ("/operations/0/purpose", json!("coverage")),
    ] {
        let mut artifact = executor();
        *artifact.pointer_mut(pointer).unwrap() = invalid;
        rejects("product-executor", &artifact);
    }
    let mut artifact = executor();
    artifact["operations"] = json!([]);
    artifact["observations"] =
        json!(["Analyze saved evidence before selecting the next browser action"]);
    accepts("product-executor", &artifact);
}

#[test]
fn issue_reproducer_accepts_operations_issues_and_followup_requests() {
    let mut artifact = reproducer();
    accepts("issue-reproducer", &artifact);
    for (pointer, invalid) in [
        ("/issues/0/scenario_ids/0", json!("../scenario")),
        ("/issues/0/id", json!("")),
        ("/followups/0/scenario_id", json!("../scenario")),
        ("/issues/0/evidence", json!("execution/stdout.txt")),
        ("/issues/0/reproduction", json!("repeat it")),
    ] {
        let mut invalid_artifact = artifact.clone();
        *invalid_artifact.pointer_mut(pointer).unwrap() = invalid;
        rejects("issue-reproducer", &invalid_artifact);
    }
    artifact["operations"] = json!([]);
    artifact["issues"] = json!([]);
    artifact["followups"] = json!([]);
    accepts("issue-reproducer", &artifact);
}

#[test]
fn result_reviewer_uses_separate_scenario_and_issue_results() {
    let mut artifact = reviewer();
    for status in ["passed", "failed", "blocked", "not_executed", "flaky"] {
        artifact["scenario_results"][0]["status"] = json!(status);
        accepts("result-reviewer", &artifact);
    }
    for status in [
        "confirmed",
        "not_reproduced",
        "blocked",
        "invalid_test",
        "flaky",
    ] {
        artifact["issue_results"][0]["status"] = json!(status);
        accepts("result-reviewer", &artifact);
    }
    for (pointer, invalid) in [
        ("/scenario_results/0/status", json!("confirmed")),
        ("/issue_results/0/status", json!("passed")),
        ("/scenario_results/0/assessment/executed", json!(1)),
        (
            "/scenario_results/0/assessment/entry_point_verified",
            Value::Null,
        ),
        ("/scenario_results/0/scenario_id", json!("../scenario")),
        ("/issue_results/0/issue_id", json!("")),
    ] {
        let mut invalid_artifact = artifact.clone();
        *invalid_artifact.pointer_mut(pointer).unwrap() = invalid;
        rejects("result-reviewer", &invalid_artifact);
    }
}

#[test]
fn product_contracts_require_exact_root_fields_and_reject_nested_extras() {
    for (stage, artifact) in [
        ("product-explorer", explorer()),
        ("scenario-designer", designer()),
        ("product-executor", executor()),
        ("issue-reproducer", reproducer()),
        ("result-reviewer", reviewer()),
    ] {
        accepts(stage, &artifact);
        for key in artifact.as_object().unwrap().keys() {
            let mut missing = artifact.clone();
            missing.as_object_mut().unwrap().remove(key);
            rejects(stage, &missing);
        }
        let mut unknown = artifact.clone();
        unknown["patch"] = json!("not part of the product workflow");
        rejects(stage, &unknown);
    }
    for (stage, mut artifact, pointer) in [
        ("product-explorer", explorer(), "/services/0"),
        ("scenario-designer", designer(), "/scenarios/0/steps/0"),
        ("product-executor", executor(), "/operations/0"),
        ("issue-reproducer", reproducer(), "/issues/0"),
        (
            "result-reviewer",
            reviewer(),
            "/scenario_results/0/assessment",
        ),
    ] {
        artifact.pointer_mut(pointer).unwrap()["unexpected"] = json!(true);
        rejects(stage, &artifact);
    }
}
