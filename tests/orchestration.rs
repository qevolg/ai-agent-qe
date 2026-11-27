use aaqe::{
    environment::ComposeEnvironment, process::ProcessResult, state::read_json,
    workflow::grounded_verdicts,
};
use serde_json::{Value, json};
use std::{
    fs,
    sync::{Arc, Mutex},
};

#[test]
fn integral_float_config_timeouts_preserve_json_schema_integer_semantics() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("aaqe.yaml"),
        "timeouts: {agent: 3.0, test: 4.0, docker: 5.0}\n",
    )
    .unwrap();
    let config = aaqe::config::Config::load(temp.path(), None).unwrap();
    assert_eq!(
        (
            config.agent_timeout,
            config.test_timeout,
            config.docker_timeout
        ),
        (3, 4, 5)
    );
    assert_eq!(aaqe::config::unsigned_integer(&json!(7.0)), Some(7));
    assert_eq!(aaqe::config::unsigned_integer(&json!(7.1)), None);
    let saved: aaqe::config::Config = serde_json::from_value(
        json!({"agent_timeout":3.0,"test_timeout":4.0,"docker_timeout":5.0}),
    )
    .unwrap();
    assert_eq!(
        (
            saved.agent_timeout,
            saved.test_timeout,
            saved.docker_timeout
        ),
        (3, 4, 5)
    );
}

#[test]
fn yaml_merge_keys_keep_inherited_settings_and_explicit_overrides() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("aaqe.yaml"), "environment:\n  <<: &runner_defaults\n    runner_workdir: /workspace\n    services: [runner]\n    runner_service: inherited\n  runner_service: runner\n").unwrap();
    let config = aaqe::config::Config::load(temp.path(), None).unwrap();
    assert_eq!(
        config.environment,
        json!({"runner_workdir":"/workspace", "services":["runner"], "runner_service":"runner"})
    );
}

#[test]
fn compose_retains_and_only_cleans_owned_resources_with_intact_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("worktree");
    fs::create_dir(&workspace).unwrap();
    fs::write(
        workspace.join("compose.yaml"),
        "services: {runner: {image: 'python:3.12-slim'}}\n",
    )
    .unwrap();
    let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let capture = calls.clone();
    let runtime = ComposeEnvironment::new(&workspace, &temp.path().join("run"), "test-run", json!({
        "compose_files":["compose.yaml"], "services":["runner"], "runner_service":"runner", "external_containers":["shared-source"]
    }), 30).with_executor(Arc::new(move |argv, _| {
        capture.lock().unwrap().push(argv.to_vec());
        ProcessResult {argv: argv.to_vec(), returncode:0, stdout: if argv.iter().any(|s|s=="inspect") {
            "[{\"State\":{\"Running\":true}}]".into()
        } else if argv.iter().any(|s|s=="config") {
            "{\"services\":{\"runner\":{\"image\":\"python:3.12-slim\"}}}".into()
        } else {String::new()}, stderr:String::new(),elapsed:0.0,timed_out:false}
    }));
    runtime.start().unwrap();
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.contains(&"--wait".into()))
    );
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.contains(&"down".into()))
    );
    let first = runtime
        .execute_test(
            &[
                "echo".into(),
                "{result_dir}".into(),
                "space argument".into(),
            ],
            "test",
            30,
        )
        .unwrap();
    let second = runtime.execute_test(&["echo".into()], "test", 30).unwrap();
    assert_ne!(first["directory"], second["directory"]);
    assert_eq!(
        first["result"]["argv"].as_array().unwrap().last().unwrap(),
        "space argument"
    );
    runtime.cleanup().unwrap();
    let deletion = calls.lock().unwrap().last().unwrap().clone();
    assert!(deletion.contains(&"down".into()) && deletion.contains(&"--volumes".into()));
    assert!(!deletion.contains(&"shared-source".into()));
    fs::write(runtime.env_dir.join("compose.resolved.json"), "{}").unwrap();
    assert!(
        runtime
            .cleanup()
            .unwrap_err()
            .to_string()
            .contains("snapshot")
    );
    fs::remove_file(runtime.env_dir.join("compose.resolved.json")).unwrap();
    assert!(
        runtime
            .cleanup()
            .unwrap_err()
            .to_string()
            .contains("snapshot")
    );
}

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

#[test]
fn reports_refresh_without_overwriting_original_evidence_or_verdicts() {
    let temp = tempfile::tempdir().unwrap();
    let mut state = json!({"id":"legacy-run", "prompt":"检查模块", "status":"completed", "stages":[], "reproductions":[]});
    let execution = json!({"baseline":[],"tests":[]});
    let verdicts = json!([]);
    aaqe::reporting::generate_report(temp.path(), &state, &json!({}), &execution, &verdicts)
        .unwrap();
    state["tmux_cleaned_at"] = json!("2026-11-25T00:00:00+00:00");
    state["reproductions"] = json!([{"at":"now", "records":[]}]);
    aaqe::reporting::refresh_report(temp.path(), &state).unwrap();
    let report: Value = read_json(&temp.path().join("report.json")).unwrap();
    assert_eq!(report["execution"], execution);
    assert_eq!(report["verdicts"], verdicts);
    assert_eq!(report["schema_version"], "1.0");
    let markdown = fs::read_to_string(temp.path().join("report.md")).unwrap();
    assert!(
        markdown.contains("检查模块")
            && markdown.contains("本轮 tmux 会话已清理")
            && markdown.contains("复现记录")
    );
}
