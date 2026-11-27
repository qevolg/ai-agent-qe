use super::grounded_verdicts;
use serde_json::json;

#[test]
fn execution_gate_requires_real_matching_failure_and_healthy_baseline() {
    let artifact = json!({"verdicts":[{"candidate_id":"bug", "test_ids":["test"], "status":"confirmed", "rationale":"failed assertion", "evidence":["/run/attempt/execution.json"]}]});
    let execution = json!({"baseline":[{"result":{"returncode":0}}],"tests":[{"test_id":"test", "directory":"/run/attempt", "junit":{"complete":true,"cases":[{"status":"failed"}]}}]});
    assert_eq!(
        grounded_verdicts(&artifact, &execution)[0]["status"],
        "confirmed"
    );
    let mut missing = execution.clone();
    missing["tests"][0]["junit"]["complete"] = json!(false);
    assert_eq!(
        grounded_verdicts(&artifact, &missing)[0]["status"],
        "blocked"
    );
    let mut fabricated = artifact.clone();
    fabricated["verdicts"][0]["evidence"] = json!(["made-up.json"]);
    assert_eq!(
        grounded_verdicts(&fabricated, &execution)[0]["status"],
        "blocked"
    );
    let mut baseline = execution.clone();
    baseline["baseline"] = json!([]);
    assert_eq!(
        grounded_verdicts(&artifact, &baseline)[0]["status"],
        "blocked"
    );
}
