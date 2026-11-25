use aaqe::{config::Config, gitops::git, state::read_json, tmux::cleanup_tmux};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn repo(path: &Path) {
    git(path, &["init", "-b", "main"]).unwrap();
    git(path, &["config", "user.name", "Test"]).unwrap();
    git(path, &["config", "user.email", "test@example.invalid"]).unwrap();
    fs::write(
        path.join("app.py"),
        "def bounded_value(n):\n    return min(n,10)\n",
    )
    .unwrap();
    git(path, &["add", "."]).unwrap();
    git(path, &["commit", "-m", "base"]).unwrap();
    git(path, &["remote", "add", "origin", "."]).unwrap();
}

#[test]
fn native_coordinator_uses_mcp_and_starts_fresh_stage_windows() {
    let temp = tempfile::tempdir().unwrap();
    repo(temp.path());
    let config = Config {
        state_dir: temp.path().join("runs"),
        codex_command: vec![
            "python3".into(),
            format!(
                "{}/tests/fixtures/scripted_codex.py",
                env!("CARGO_MANIFEST_DIR")
            ),
            "--loop-test".into(),
        ],
        run_timeout: 30,
        ..Config::default()
    };
    let state = aaqe::workflow::run_audit("audit", temp.path(), &config, &|_| {}).unwrap();
    let directory = config.state_dir.join(state["id"].as_str().unwrap());
    let sessions = [
        "coordinator",
        "code-explorer",
        "code-explorer/attempts/0002",
        "bug-hunter",
    ]
    .map(|p| {
        let stage = directory.join("stages").join(p);
        assert!(stage.join("input.json").is_file());
        assert!(stage.join("prompt.md").is_file());
        assert!(stage.join("stdout.log").is_file());
        read_json(&stage.join("tmux.json")).unwrap()["session"]
            .as_str()
            .unwrap()
            .to_owned()
    });
    cleanup_tmux(&directory).unwrap();
    assert_eq!(state["status"], "completed", "{state}");
    assert_eq!(
        sessions
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        4
    );
    assert_eq!(state["stages"].as_array().unwrap().len(), 3);
    let events = fs::read_to_string(directory.join("stages/coordinator/tools.jsonl")).unwrap();
    assert!(events.contains("Missing prerequisite"));
    assert!(events.contains("finish"));
}

#[test]
fn mcp_protocol_errors_do_not_pollute_stdout() {
    let temp = tempfile::tempdir().unwrap();
    // A legitimate initialized harness comes from a deliberately unfinished model.
    repo(temp.path());
    let config = Config {
        state_dir: temp.path().join("runs"),
        codex_command: vec!["true".into()],
        run_timeout: 10,
        ..Config::default()
    };
    let state = aaqe::workflow::run_audit("audit", temp.path(), &config, &|_| {}).unwrap();
    assert_eq!(state["status"], "blocked");
    let directory = config.state_dir.join(state["id"].as_str().unwrap());
    let mut child = Command::new(env!("CARGO_BIN_EXE_aaqe"))
        .args(["__harness-mcp", directory.to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for (id, method, params) in [
        (1, "initialize", json!({"protocolVersion":"2024-11-05"})),
        (2, "tools/list", json!({})),
        (
            3,
            "tools/call",
            json!({"name":"run_stage","arguments":{"stage":"bad","instruction":"check"}}),
        ),
        (4, "unknown", json!({})),
    ] {
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    cleanup_tmux(&directory).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(replies.len(), 4);
    assert_eq!(replies[0]["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(replies[1]["result"]["tools"].as_array().unwrap().len(), 3);
    assert_eq!(replies[2]["result"]["isError"], true);
    assert_eq!(replies[3]["error"]["code"], -32601);
}

#[test]
fn cancelling_or_timing_out_the_coordinator_stops_separate_stage_processes() {
    use std::{
        thread,
        time::{Duration, Instant},
    };
    for timeout in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        repo(temp.path());
        let runs = temp.path().join("runs");
        let config = temp.path().join("test-config.yaml");
        fs::write(&config,serde_json::to_string(&json!({"state_dir":runs,"codex":{"command":["python3",format!("{}/tests/fixtures/scripted_codex.py",env!("CARGO_MANIFEST_DIR")),"--cancel-test"]},"loop":{"timeout":if timeout {3} else {30}}})).unwrap()).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_aaqe"))
            .current_dir(temp.path())
            .args(["--config", config.to_str().unwrap(), "run", "audit"])
            .stdout(fs::File::create(temp.path().join("parent.log")).unwrap())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        let run = loop {
            if let Ok(entries) = fs::read_dir(&runs)
                && let Some(entry) = entries.flatten().next()
                && entry
                    .path()
                    .join("stages/code-explorer/worker.pid")
                    .exists()
            {
                break entry.path();
            }
            assert!(Instant::now() < deadline, "stage did not start");
            thread::sleep(Duration::from_millis(20));
        };
        if !timeout {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGINT);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(9);
        let exit = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                panic!("coordinator did not stop")
            }
            thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(exit.code(), Some(1));
        let state = read_json(&run.join("run.json")).unwrap();
        assert_eq!(
            state["status"],
            if timeout { "blocked" } else { "interrupted" },
            "{state}"
        );
        let stage = run.join("stages/code-explorer");
        let pid = fs::read_to_string(stage.join("worker.pid")).unwrap();
        if let Ok(stat) = fs::read_to_string(format!("/proc/{}/stat", pid.trim())) {
            assert_eq!(
                stat.split_whitespace().nth(2),
                Some("Z"),
                "stage worker survived coordinator cancellation"
            );
        }
        assert!(stage.join("finished").exists());
        assert!(run.join("report.md").is_file());
        cleanup_tmux(&run).unwrap();
    }
}
