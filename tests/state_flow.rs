mod support;
use aaqe::{
    config::Config,
    state::{Store, read_json, write_json},
};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn snapshot(state: &Value) -> Value {
    json!({"schema_version":1,"state":state,"artifacts":{},"execution":{"baseline":[],"tests":[]},"verdicts":[]})
}

#[test]
fn reproduction_rejects_active_audits_before_accessing_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path());
    let mut state = store
        .create("audit", &Config::default(), temp.path())
        .unwrap();
    state["environment"] = json!({});
    for status in ["initializing", "running"] {
        state["status"] = json!(status);
        store.save(&state).unwrap();
        let error = aaqe::audit::workflow::reproduce(&store, &mut state, None).unwrap_err();
        assert!(
            format!("{error:#}").contains("audit is running"),
            "{error:#}"
        );
    }
}

#[test]
fn terminal_status_during_audit_finalization_rejects_reproduction_and_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path().join(".aaqe"));
    let mut state = store
        .create("audit", &Config::default(), temp.path())
        .unwrap();
    let id = state["id"].as_str().unwrap().to_owned();
    state["status"] = json!("completed");
    state["audit_finalized"] = json!(false);
    state["environment"] = json!({});
    store.save(&state).unwrap();
    let error = aaqe::audit::workflow::reproduce(&store, &mut state, None).unwrap_err();
    assert!(format!("{error:#}").contains("finaliz"), "{error:#}");
    let cleanup = std::process::Command::new(env!("CARGO_BIN_EXE_aaqe"))
        .env("HOME", temp.path())
        .args(["cleanup", &id])
        .output()
        .unwrap();
    assert_eq!(cleanup.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&cleanup.stdout).contains("finaliz"));
    assert!(store.load(&id).unwrap()["tmux_cleaned_at"].is_null());
}

fn git(root: &Path, args: &[&str]) -> String {
    aaqe::runtime::git::git(root, args).unwrap()
}

#[test]
fn failed_reproduction_start_is_registered_without_changing_original_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-b", "main"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    fs::write(root.join("app.txt"), "original").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial"]);
    let store = Store::new(temp.path().join("runs"));
    let mut state = store.create("audit", &Config::default(), &root).unwrap();
    let id = state["id"].as_str().unwrap().to_owned();
    let directory = store.path(&id).unwrap();
    state["status"] = json!("completed");
    state["environment"] = json!({}); // deterministic startup error before invoking Docker
    state["workspace"] = json!({"path":root,"commit":git(&root, &["rev-parse","HEAD"])});
    let mut saved = snapshot(&state);
    saved["verdicts"] = json!([{"candidate_id":"bug","status":"confirmed"}]);
    saved["artifacts"]["test-developer"] = json!({"tests":[{"id":"test","candidate_id":"bug","command":["test"],"report":"junit.xml","expected_names":["test"],"reproduction_commands":[]}]});
    write_json(&directory.join("harness.json"), &saved).unwrap();
    fs::create_dir_all(directory.join("artifacts")).unwrap();
    fs::write(
        directory.join("artifacts/test-developer.json"),
        "broken compatibility alias",
    )
    .unwrap();
    let error = aaqe::audit::workflow::reproduce(&store, &mut state, Some("test")).unwrap_err();
    assert!(format!("{error:#}").contains("runner"), "{error:#}");
    let loaded = store.load(&id).unwrap();
    assert_eq!(loaded["reproductions"][0]["status"], "failed");
    assert!(
        loaded["reproductions"][0]["error"]
            .as_str()
            .unwrap()
            .contains("runner")
    );
    assert_eq!(
        read_json(&directory.join("harness.json")).unwrap()["verdicts"][0]["status"],
        "confirmed"
    );
}

#[test]
fn busy_run_rejects_mutation_and_mcp_remains_available_after_contention() {
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Command, Stdio},
    };
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path());
    let mut state = store
        .create("audit", &Config::default(), temp.path())
        .unwrap();
    let directory = store.path(state["id"].as_str().unwrap()).unwrap();
    write_json(&directory.join("harness.json"), &snapshot(&state)).unwrap();
    let lock = aaqe::state::RunLock::acquire(&directory).unwrap();
    let error = aaqe::audit::workflow::reproduce(&store, &mut state, None).unwrap_err();
    assert!(format!("{error:#}").contains("busy"));
    let mut server = Command::new(env!("CARGO_BIN_EXE_aaqe"))
        .arg("__harness-mcp")
        .arg(&directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = server.stdin.take().unwrap();
    let mut output = BufReader::new(server.stdout.take().unwrap());
    writeln!(input, "{}", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"status","arguments":{}}})).unwrap();
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    assert!(
        !line.is_empty(),
        "MCP exited on recoverable lock contention"
    );
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply["result"]["isError"], true);
    assert!(reply.to_string().contains("busy"));
    drop(lock);
    writeln!(input, "{}", json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"status","arguments":{}}})).unwrap();
    line.clear();
    output.read_line(&mut line).unwrap();
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply["result"]["isError"], false);
    assert_eq!(
        reply["result"]["structuredContent"]["state"]["status"],
        "initializing"
    );
    drop(input);
    assert!(server.wait().unwrap().success());
    assert!(aaqe::state::RunLock::acquire(&directory).is_ok());
}
