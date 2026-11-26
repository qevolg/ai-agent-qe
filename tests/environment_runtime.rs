use aaqe::{environment::ComposeEnvironment, process::ProcessResult};
use serde_json::json;
use std::{
    fs,
    sync::{Arc, Mutex},
};

#[test]
fn inline_compose_is_owned_outside_worktree_and_readiness_retries_reuse_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("worktree");
    fs::create_dir(&workspace).unwrap();
    let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let captured = calls.clone();
    let fail_readiness = Arc::new(Mutex::new(true));
    let fail = fail_readiness.clone();
    let runtime = ComposeEnvironment::new(
        &workspace,
        &temp.path().join("run"),
        "retry",
        json!({
            "compose":{"services":{"runner":{"image":"alpine","command":["sleep","infinity"]}}},
            "runner_service":"runner", "readiness":[["test","-f","/ready"]]
        }),
        5,
    )
    .with_executor(Arc::new(move |argv, _| {
        captured.lock().unwrap().push(argv.to_vec());
        let readiness = argv.iter().any(|s| s == "exec");
        ProcessResult {
            argv: argv.to_vec(),
            returncode: if readiness && *fail.lock().unwrap() {
                1
            } else {
                0
            },
            stdout: if argv.iter().any(|s| s == "config") {
                "{\"services\":{\"runner\":{\"image\":\"alpine\"}}}".into()
            } else {
                String::new()
            },
            stderr: String::new(),
            elapsed: 0.0,
            timed_out: false,
        }
    }));
    assert!(
        runtime
            .start()
            .unwrap_err()
            .to_string()
            .contains("readiness")
    );
    let snapshot = fs::read(runtime.env_dir.join("compose.resolved.json")).unwrap();
    assert_eq!(fs::read_dir(&workspace).unwrap().count(), 0);
    assert!(runtime.env_dir.join("compose.generated.json").is_file());
    *fail_readiness.lock().unwrap() = false;
    runtime.start().unwrap();
    assert_eq!(
        fs::read(runtime.env_dir.join("compose.resolved.json")).unwrap(),
        snapshot
    );
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|argv| argv.iter().any(|s| s == "config"))
            .count(),
        1
    );
    fs::write(runtime.env_dir.join("compose.resolved.json"), "{}").unwrap();
    let count = calls.lock().unwrap().len();
    assert!(
        runtime
            .start()
            .unwrap_err()
            .to_string()
            .contains("integrity")
    );
    assert_eq!(calls.lock().unwrap().len(), count);
}

#[test]
fn compose_rejects_ambiguous_sources_and_run_directories_inside_workspace() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("compose.yaml"), "services: {}").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let runtime = ComposeEnvironment::new(
        temp.path(),
        outside.path(),
        "ambiguous",
        json!({
            "compose_files":["compose.yaml"], "compose":{"services":{}}, "runner_service":"runner"
        }),
        5,
    );
    assert!(
        runtime
            .arguments(false)
            .unwrap_err()
            .to_string()
            .contains("both")
    );
    let inside = ComposeEnvironment::new(
        temp.path(),
        &temp.path().join("run"),
        "inside",
        json!({
            "compose":{"services":{}}, "runner_service":"runner"
        }),
        5,
    );
    assert!(inside.start().unwrap_err().to_string().contains("outside"));
    assert!(!temp.path().join("run").exists());
}

#[test]
fn file_compose_retry_uses_snapshot_after_source_disappears_and_rejects_changed_spec() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("compose.yaml"), "services: {}").unwrap();
    let mut runtime = ComposeEnvironment::new(
        &workspace,
        &temp.path().join("run"),
        "file",
        json!({
            "compose_files":["compose.yaml"], "runner_service":"runner"
        }),
        5,
    )
    .with_executor(Arc::new(|argv, _| ProcessResult {
        argv: argv.to_vec(),
        returncode: 0,
        stdout: if argv.contains(&"config".into()) {
            "{\"services\":{\"runner\":{\"image\":\"alpine\"}}}".into()
        } else {
            String::new()
        },
        stderr: String::new(),
        elapsed: 0.0,
        timed_out: false,
    }));
    runtime.start().unwrap();
    fs::remove_file(workspace.join("compose.yaml")).unwrap();
    runtime.spec =
        serde_json::from_str(r#"{"runner_service":"runner","compose_files":["compose.yaml"]}"#)
            .unwrap();
    runtime.start().unwrap();
    runtime.spec["runner_service"] = json!("changed");
    assert!(
        runtime
            .start()
            .unwrap_err()
            .to_string()
            .contains("immutable")
    );
}

#[test]
fn encoded_compose_objects_support_structured_agent_output() {
    let temp = tempfile::tempdir().unwrap();
    let run = tempfile::tempdir().unwrap();
    let runtime = ComposeEnvironment::new(temp.path(), run.path(), "encoded", json!({
        "compose":"{\"services\":{\"runner\":{\"image\":\"alpine\"}}}", "runner_service":"runner"
    }), 5).with_executor(Arc::new(|argv,_| ProcessResult {argv:argv.to_vec(),returncode:0,
        stdout:if argv.contains(&"config".into()) {"{\"services\":{\"runner\":{\"image\":\"alpine\"}}}".into()} else {String::new()},stderr:String::new(),elapsed:0.0,timed_out:false}));
    runtime.start().unwrap();
    let generated =
        aaqe::state::read_json(&runtime.env_dir.join("compose.generated.json")).unwrap();
    assert_eq!(generated["services"]["runner"]["image"], "alpine");
}

#[test]
fn startup_commands_share_one_operation_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let captured = calls.clone();
    let runtime=ComposeEnvironment::new(&workspace,&temp.path().join("run"),"deadline",json!({"compose":{"services":{"runner":{"image":"fixture"}}},"runner_service":"runner","readiness":[["ready"],["ready-again"]]}),2).with_executor(Arc::new(move|argv,timeout|{
        captured.lock().unwrap().push(timeout);
        if argv.iter().any(|arg|arg=="config"){std::thread::sleep(std::time::Duration::from_millis(1100));}
        ProcessResult{argv:argv.to_vec(),returncode:0,stdout:if argv.iter().any(|arg|arg=="config"){"{\"services\":{\"runner\":{\"image\":\"fixture\"}}}".into()}else{String::new()},stderr:String::new(),elapsed:0.0,timed_out:false}
    }));
    runtime.start().unwrap();
    assert_eq!(
        *calls.lock().unwrap(),
        vec![2, 1, 1, 1],
        "render, up and readiness share the initial time budget"
    );
}
