use aaqe::reporting::{generate_report, refresh_report};
use serde_json::json;
use std::fs;

#[test]
fn reports_rebuild_from_snapshot_when_derived_files_are_missing_or_corrupt() {
    use aaqe::{
        config::Config,
        state::{Store, read_json, write_json},
    };
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path());
    let mut state = store
        .create("audit", &Config::default(), temp.path())
        .unwrap();
    let id = state["id"].as_str().unwrap().to_owned();
    let directory = store.path(&id).unwrap();
    state["status"] = json!("completed");
    let execution =
        json!({"baseline":[],"tests":[{"test_id":"original-test","result":{"returncode":1}}]});
    let verdicts =
        json!([{"candidate_id":"bug","status":"confirmed","rationale":"original audit"}]);
    write_json(&directory.join("harness.json"), &json!({"schema_version":1,"state":state,"artifacts":{"bug-hunter":{"candidates":[{"id":"bug","title":"Original candidate"}]}},"execution":execution,"verdicts":verdicts})).unwrap();
    for corrupt in [false, true] {
        if corrupt {
            fs::write(directory.join("report.json"), "broken derived report").unwrap();
        }
        let report = refresh_report(&directory, &state).unwrap();
        assert_eq!(report["execution"], execution);
        assert_eq!(report["verdicts"], verdicts);
        assert_eq!(report["candidates"][0]["title"], "Original candidate");
        assert!(
            fs::read_to_string(directory.join("report.md"))
                .unwrap()
                .contains("Original candidate")
        );
        assert_eq!(
            read_json(&directory.join("report.json")).unwrap()["execution"],
            execution
        );
    }
}

#[test]
fn reports_keep_candidate_blockers_and_generated_execution_evidence_distinct() {
    let temp = tempfile::tempdir().unwrap();
    let state = json!({"id":"report-test","status":"blocked","error":"run-level interruption"});
    let artifacts = json!({
        "bug-hunter":{"candidates":[
            {"id":"oracle","title":"Missing oracle"},
            {"id":"runner","title":"Missing runner"},
            {"id":"executed","title":"Executed candidate"}
        ]},
        "test-designer":{"plans":[
            {"candidate_id":"oracle","test_id":"oracle-test","blocked_reason":"No documented expected value"},
            {"candidate_id":"runner","test_id":"runner-test","blocked_reason":null,"execution_blocked_reason":"Runner compiler unavailable"},
            {"candidate_id":"executed","test_id":"executed-test","blocked_reason":null}
        ]},
        "test-developer":{"tests":[{"id":"runner-test","candidate_id":"runner"},{"id":"executed-test","candidate_id":"executed"}]}
    });
    let execution = json!({"baseline":[{"test_id":"baseline-unique","for_test_id":"executed-test","result":{"returncode":0}}],"tests":[{"test_id":"executed-test","directory":"evidence/executed","result":{"returncode":1},"junit":{"complete":true}}]});
    let report = generate_report(temp.path(), &state, &artifacts, &execution, &json!([])).unwrap();
    assert_eq!(
        report["verdicts"][0]["rationale"],
        "No documented expected value"
    );
    assert_eq!(
        report["verdicts"][1]["rationale"],
        "Runner compiler unavailable"
    );
    let results = report["candidate_results"].as_array().unwrap();
    assert_eq!(
        results[0]["plans"][0]["blocked_reason"],
        "No documented expected value"
    );
    assert_eq!(results[0]["generated_tests"], json!([]));
    assert_eq!(results[1]["generated_tests"][0]["id"], "runner-test");
    assert_eq!(results[1]["execution"]["tests"], json!([]));
    assert_eq!(results[1]["execution"]["baseline"], json!([]));
    assert_eq!(
        results[2]["execution"]["tests"][0]["test_id"],
        "executed-test"
    );
    assert_eq!(
        results[2]["execution"]["baseline"][0]["test_id"],
        "baseline-unique"
    );
    let md = fs::read_to_string(temp.path().join("report.md")).unwrap();
    let runner = md
        .split("### runner:")
        .nth(1)
        .unwrap()
        .split("### executed:")
        .next()
        .unwrap();
    assert!(runner.contains("Runner compiler unavailable"));
    assert!(runner.contains("generated: yes; executions: 0"));
    assert!(!runner.contains("No documented expected value"));
    assert!(!runner.contains("evidence/executed"));
}

#[test]
fn reports_separate_dispatch_rejections_from_launched_workers_and_preserve_history() {
    let temp = tempfile::tempdir().unwrap();
    let state = json!({"id":"history","status":"blocked","loop_started_at":"now",
        "stages":[{"name":"code-explorer","attempt":1,"status":"accepted"}],
        "dispatch_rejections":[{"stage":"test-developer","gate":"dispatch","error":"Missing plans","recovery":{"permitted_stages":["test-designer"]}}],
        "environment_actions":[{"operation":"inspect_environment","status":"ok","result":{"success":false,"directory":"environment/probes/probe-one"}}],
        "execution_attempts":[{"directory":"executions/rounds/previous","test_ids":["old-test"],"status":"failed"}],
        "environment_ready":false});
    let report = generate_report(temp.path(), &state, &json!({}), &json!({}), &json!([])).unwrap();
    assert_eq!(report["stages"].as_array().unwrap().len(), 1);
    assert_eq!(report["dispatch_rejections"][0]["error"], "Missing plans");
    assert_eq!(report["environment_actions"][0]["result"]["success"], false);
    assert_eq!(
        report["execution_attempts"][0]["directory"],
        "executions/rounds/previous"
    );
    let md = fs::read_to_string(temp.path().join("report.md")).unwrap();
    assert!(md.contains("Launched workers: 1; rejected dispatches: 1"));
    assert!(md.contains("Missing plans"));
    assert!(md.contains("environment/probes/probe-one"));
    assert!(md.contains("executions/rounds/previous"));
    let refreshed = refresh_report(temp.path(), &state).unwrap();
    assert_eq!(
        refreshed["dispatch_rejections"],
        report["dispatch_rejections"]
    );
}
