use aaqe::{environment::ComposeEnvironment, process::ProcessResult};
use serde_json::json;
use std::{
    fs,
    sync::{Arc, Mutex},
};

#[test]
fn dependency_setup_runs_as_root_before_readiness_and_retries_without_replacing_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
    let captured = calls.clone();
    let installed = Arc::new(Mutex::new(false));
    let available = installed.clone();
    let runtime = ComposeEnvironment::new(
        &workspace,
        &temp.path().join("run"),
        "setup",
        json!({
            "compose":{"services":{"runner":{"image":"fixture"}}},
            "runner_service":"runner", "setup":[["install-dependency"]],
            "readiness":[["check-dependency"]]
        }),
        5,
    )
    .with_executor(Arc::new(move |argv, _| {
        captured.lock().unwrap().push(argv.to_vec());
        let mut code = 0;
        if argv.last().is_some_and(|a| a == "install-dependency") {
            assert!(argv.windows(2).any(|a| a == ["--user", "0"]));
            if !*available.lock().unwrap() {
                code = 1;
            }
        }
        if argv.last().is_some_and(|a| a == "check-dependency") && !*available.lock().unwrap() {
            code = 1;
        }
        ProcessResult {
            argv: argv.to_vec(),
            returncode: code,
            stdout: if argv.iter().any(|a| a == "config") {
                r#"{"services":{"runner":{"image":"fixture"}}}"#.into()
            } else {
                String::new()
            },
            stderr: if code != 0 {
                "package repository unavailable".into()
            } else {
                String::new()
            },
            elapsed: 0.0,
            timed_out: false,
        }
    }));
    let failure = runtime.start().unwrap_err().to_string();
    assert!(failure.contains("setup"));
    assert!(failure.contains("package repository unavailable"));
    assert!(
        !calls
            .lock()
            .unwrap()
            .iter()
            .any(|a| a.last().is_some_and(|s| s == "check-dependency"))
    );
    let snapshot = fs::read(runtime.env_dir.join("compose.resolved.json")).unwrap();
    *installed.lock().unwrap() = true;
    runtime.start().unwrap();
    assert_eq!(
        snapshot,
        fs::read(runtime.env_dir.join("compose.resolved.json")).unwrap()
    );
    let calls = calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|a| a.iter().any(|s| s == "config"))
            .count(),
        1
    );
    let exec: Vec<_> = calls
        .iter()
        .filter(|a| a.iter().any(|s| s == "exec"))
        .map(|a| a.last().unwrap().as_str())
        .collect();
    assert_eq!(
        exec,
        [
            "install-dependency",
            "install-dependency",
            "check-dependency"
        ]
    );
    assert_eq!(fs::read_dir(&workspace).unwrap().count(), 0);
}

#[test]
fn invalid_setup_is_rejected_before_docker_or_evidence_changes() {
    for setup in [
        json!("shell text"),
        json!([[]]),
        json!([["bad\u{0}argument"]]),
        json!([[""]]),
        json!([vec!["tool"; 65]]),
    ] {
        let workspace = tempfile::tempdir().unwrap();
        let run = tempfile::tempdir().unwrap();
        let mut spec = json!({"compose":{"services":{"runner":{"image":"fixture"}}},"runner_service":"runner"});
        spec["setup"] = setup;
        let runtime = ComposeEnvironment::new(workspace.path(), run.path(), "bad", spec, 5)
            .with_executor(Arc::new(|_, _| panic!("invalid setup reached Docker")));
        assert!(runtime.start().is_err());
        assert!(!runtime.env_dir.exists());
    }
}

#[test]
fn repair_recipes_are_deduplicated_validated_and_loaded_by_a_new_runtime() {
    let workspace = tempfile::tempdir().unwrap();
    let run = tempfile::tempdir().unwrap();
    let runtime = ComposeEnvironment::new(workspace.path(), run.path(), "repair", json!({}), 5);
    let install = vec![vec!["install".into(), "dependency".into()]];
    runtime.record_setup(&install).unwrap();
    runtime.record_setup(&install).unwrap();
    let before = fs::read(runtime.env_dir.join("dependency-setup.json")).unwrap();
    for invalid in [
        vec![],
        vec![vec![]],
        vec![vec!["bad\u{0}argument".into()]],
        (0..33).map(|i| vec![format!("package-{i}")]).collect(),
    ] {
        assert!(runtime.record_setup(&invalid).is_err());
        assert_eq!(
            before,
            fs::read(runtime.env_dir.join("dependency-setup.json")).unwrap()
        );
    }
    let restored = ComposeEnvironment::new(workspace.path(), run.path(), "repair", json!({}), 5);
    assert_eq!(restored.saved_setup().unwrap(), install);
}

#[test]
fn incorrect_installation_recipe_can_be_replaced_without_changing_compose() {
    let workspace = tempfile::tempdir().unwrap();
    let run = tempfile::tempdir().unwrap();
    let runtime = ComposeEnvironment::new(
        workspace.path(),
        run.path(),
        "correction",
        json!({"setup":[["install-wrong-version"]]}),
        5,
    );
    runtime
        .record_setup_with_replace(&[vec!["install-correct-version".into()]], true)
        .unwrap();
    assert_eq!(
        runtime.saved_setup().unwrap(),
        vec![vec!["install-correct-version".to_string()]]
    );
    runtime
        .record_setup(&[vec!["install-another-package".into()]])
        .unwrap();
    assert_eq!(runtime.saved_setup().unwrap().len(), 2);
}

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
