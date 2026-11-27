use aaqe::{config::Config, contracts, gitops, process, state::Store};
use serde_json::{Value, json};
use std::{fs, path::Path};

#[test]
fn new_run_identity_uses_shanghai_wall_time_and_keeps_legacy_ids_readable() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path());
    let state = store
        .create("audit", &Config::default(), temp.path())
        .unwrap();
    let id = state["id"].as_str().unwrap();
    assert!(
        regex::Regex::new(r"^[0-9]{14}-[0-9a-f]{8}$")
            .unwrap()
            .is_match(id),
        "{id}"
    );
    let created =
        chrono::DateTime::parse_from_rfc3339(state["created_at"].as_str().unwrap()).unwrap();
    assert_eq!(created.offset().local_minus_utc(), 28_800);
    let named = chrono::NaiveDateTime::parse_from_str(&id[..14], "%Y%m%d%H%M%S").unwrap();
    assert!((created.naive_local() - named).num_seconds().abs() <= 1);
    assert_eq!(store.load(id).unwrap()["id"], id);
    let mut legacy = state.clone();
    legacy["id"] = json!("20261125T081514Z-ab12cd34");
    store.save(&legacy).unwrap();
    assert_eq!(
        store.load("20261125T081514Z-ab12cd34").unwrap()["id"],
        legacy["id"]
    );
}

#[test]
fn stage_logs_keep_full_output_on_disk_with_bounded_head_and_tail_in_process_records() {
    let temp = tempfile::tempdir().unwrap();
    let command = vec!["python3".into(), "-c".into(), "import sys; sys.stdout.buffer.write(b'BEGIN\\n'+b'x'*2000000+b'\\nEND'); sys.stderr.buffer.write(b'ERROR-BEGIN\\n'+b'y'*2000000+b'\\nERROR-END')".into()];
    let result = process::run_logged(&command, temp.path(), 10, "", temp.path()).unwrap();
    assert_eq!(result.returncode, 0);
    assert!(result.stdout.len() < 70000 && result.stderr.len() < 70000);
    assert!(result.stdout.starts_with("BEGIN\n") && result.stdout.ends_with("\nEND"));
    assert!(result.stderr.starts_with("ERROR-BEGIN\n") && result.stderr.ends_with("\nERROR-END"));
    assert!(result.stdout.contains(process::OUTPUT_TRUNCATED.trim()));
    assert_eq!(
        fs::metadata(temp.path().join("stdout.log")).unwrap().len(),
        2_000_010
    );
    assert_eq!(
        fs::metadata(temp.path().join("stderr.log")).unwrap().len(),
        2_000_022
    );
    let saved = aaqe::state::read_json(&temp.path().join("process.json")).unwrap();
    assert_eq!(saved["stdout"], result.stdout);
}

#[test]
fn streamed_binary_output_stays_bounded_after_lossy_utf8_conversion() {
    let temp = tempfile::tempdir().unwrap();
    let command = vec![
        "python3".into(),
        "-c".into(),
        "import sys; sys.stdout.buffer.write(bytes([255])*60000)".into(),
    ];
    let result = process::run_logged(&command, temp.path(), 10, "", temp.path()).unwrap();
    assert_eq!(result.returncode, 0);
    assert!(result.stdout.len() < 70000);
    assert_eq!(
        fs::read(temp.path().join("stdout.log")).unwrap(),
        vec![255; 60000]
    );
}

#[test]
fn failure_to_persist_a_stream_is_reported_as_failed_execution() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    symlink("/dev/full", temp.path().join("stdout.log")).unwrap();
    let result = process::run_logged(
        &["sh".into(), "-c".into(), "printf evidence".into()],
        temp.path(),
        5,
        "",
        temp.path(),
    )
    .unwrap();
    assert_ne!(
        result.returncode, 0,
        "A process without persisted evidence must not appear successful"
    );
    assert!(result.stderr.contains("log"), "{}", result.stderr);
}

fn git(directory: &Path, arguments: &[&str]) -> String {
    gitops::git(directory, arguments).unwrap()
}

#[test]
fn config_is_optional_and_resolves_paths_without_reading_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path();
    let default = Config::load(directory, None).unwrap();
    assert_eq!(default.remote, "origin");
    assert_eq!(default.branch, "main");
    assert_eq!(default.max_stage_calls, 20);
    assert_eq!(default.max_stage_attempts, 4);
    assert_eq!(default.run_timeout, 14400);
    assert!(default.state_dir.is_absolute());
    fs::write(
        directory.join("aaqe.yaml"),
        "loop: {max_stage_calls: 8, max_stage_attempts: 2, timeout: 3600}\nstate_dir: runs\ngit:\n  branch: develop\nenvironment:\n  env_file: private.env\n",
    )
    .unwrap();
    fs::write(directory.join("private.env"), "PASSWORD=do-not-publish\n").unwrap();
    let loaded = Config::load(directory, None).unwrap();
    assert_eq!(loaded.branch, "develop");
    assert_eq!(loaded.max_stage_calls, 8);
    assert_eq!(loaded.max_stage_attempts, 2);
    assert_eq!(loaded.run_timeout, 3600);
    assert_eq!(loaded.state_dir, directory.join("runs"));
    assert_eq!(
        loaded.environment["env_file"],
        directory.join("private.env").to_str().unwrap()
    );
    let snapshot = serde_json::to_value(&loaded).unwrap();
    assert!(!snapshot.to_string().contains("do-not-publish"));
    assert_eq!(
        serde_json::from_value::<Config>(snapshot).unwrap().branch,
        "develop"
    );
}

#[test]
fn invalid_config_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    for content in [
        "unknown: true",
        "git: []",
        "environment: {services: nope}",
        "timeouts: {test: 0}",
        "git: {remote: --upload-pack=bad}",
        "codex: {command: []}",
        "loop: {max_stage_calls: 0}",
        "loop: {max_stage_attempts: -1}",
        "loop: {timeout: 0}",
    ] {
        fs::write(temp.path().join("aaqe.yaml"), content).unwrap();
        assert!(Config::load(temp.path(), None).is_err(), "{content}");
    }
    assert!(Config::load(temp.path(), Some(&temp.path().join("missing.yaml"))).is_err());
}

#[test]
fn workspace_uses_fetched_main_and_preserves_dirty_caller() {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    fs::create_dir(&seed).unwrap();
    git(temp.path(), &["init", "--bare", remote.to_str().unwrap()]);
    git(&seed, &["init", "-b", "main"]);
    git(&seed, &["config", "user.name", "Test User"]);
    git(&seed, &["config", "user.email", "test@example.invalid"]);
    fs::write(seed.join("app.py"), "original\n").unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-m", "initial"]);
    git(
        &seed,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&seed, &["push", "-u", "origin", "main"]);
    git(&seed, &["checkout", "-b", "local-work"]);
    fs::write(seed.join("app.py"), "local uncommitted work\n").unwrap();
    let run = temp.path().join("run");
    let workspace = gitops::create_workspace(&seed, &run, &Config::default(), "run-001").unwrap();
    let path = Path::new(workspace["path"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(path.join("app.py")).unwrap(),
        "original\n"
    );
    assert_eq!(
        fs::read_to_string(seed.join("app.py")).unwrap(),
        "local uncommitted work\n"
    );
    assert_eq!(git(&seed, &["branch", "--show-current"]), "local-work");
    assert_eq!(
        workspace["commit"],
        git(&seed, &["rev-parse", "origin/main"])
    );
    assert_eq!(git(path, &["branch", "--show-current"]), "aaqe/run-001");
    let missing = Config {
        branch: "missing".into(),
        ..Config::default()
    };
    assert!(gitops::create_workspace(&seed, &run.join("missing"), &missing, "run-002").is_err());
}

#[test]
fn state_rejects_traversal_and_preserves_json_format() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path().join("runs"));
    for id in ["../other", "", "-bad", "a/b", "."] {
        assert!(store.path(id).is_err());
    }
    let state = store
        .create("inspect 产品", &Config::default(), temp.path())
        .unwrap();
    assert_eq!(store.load(state["id"].as_str().unwrap()).unwrap(), state);
    assert_eq!(state["prompt"], "inspect 产品");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        assert_eq!(
            fs::metadata(store.path(state["id"].as_str().unwrap()).unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        symlink(temp.path(), temp.path().join("runs/linked")).unwrap();
        assert!(store.path("linked").is_err());
    }
}

#[test]
fn process_captures_stdin_and_missing_executables() {
    let result = process::run(
        &[
            "sh".into(),
            "-c".into(),
            "cat; printf diagnostic >&2; exit 7".into(),
        ],
        None,
        10,
        Some("产品"),
    );
    assert_eq!(result.returncode, 7);
    assert_eq!(result.stdout, "产品");
    assert_eq!(result.stderr, "diagnostic");
    let result = process::run(&["aaqe-this-command-does-not-exist".into()], None, 10, None);
    assert_eq!(result.returncode, 127);
    assert!(!result.stderr.is_empty());
}

#[test]
fn process_timeout_kills_descendants() {
    let result = process::run(
        &["sh".into(), "-c".into(), "sleep 30 & wait".into()],
        None,
        1,
        None,
    );
    assert_eq!(result.returncode, 124);
    assert!(result.timed_out);
    assert!(result.elapsed < 5.0);
}

#[cfg(unix)]
#[test]
fn interrupted_processes_can_finish_cleanup_without_clearing_status() {
    if std::env::var_os("AAQE_CORE_INTERRUPT_CHILD").is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "interrupted_processes_can_finish_cleanup_without_clearing_status",
                "--nocapture",
            ])
            .env("AAQE_CORE_INTERRUPT_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    process::install_signals().unwrap();
    unsafe {
        libc::raise(libc::SIGINT);
    }
    assert!(process::interrupted());
    let cancelled = process::run(
        &["sh".into(), "-c".into(), "sleep 30".into()],
        None,
        10,
        None,
    );
    assert_eq!(cancelled.returncode, 130);
    let cleanup = process::ignoring_interrupts(|| {
        process::run(&["sh".into(), "-c".into(), "exit 0".into()], None, 10, None)
    });
    assert_eq!(cleanup.returncode, 0);
    assert!(process::interrupted());
    process::reset_interrupt();
}

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
    for stage in aaqe::workflow::STAGES {
        strict(&contracts::schema(stage).unwrap());
    }
    assert_eq!(
        contracts::schema("code-explorer").unwrap()["properties"]["environment"]["properties"]["compose"],
        json!({"type":["string","null"]})
    );
}
