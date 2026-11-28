mod support;
use aaqe::{config::Config, runtime::git::git, runtime::tmux::cleanup_tmux, state::read_json};
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
        path.join("app.rs"),
        "pub fn bounded_value(n: i32) -> i32 { n.min(10) }\n",
    )
    .unwrap();
    git(path, &["add", "."]).unwrap();
    git(path, &["commit", "-m", "base"]).unwrap();
    git(path, &["remote", "add", "origin", "."]).unwrap();
}

struct Cleanup(std::path::PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Ok(runs) = fs::read_dir(&self.0) {
            for run in runs.flatten() {
                let _ = cleanup_tmux(&run.path());
            }
        }
    }
}

#[test]
fn policy_denial_stops_stage_and_coordinator_without_retrying() {
    for flag in [
        "--policy-stage",
        "--policy-provider",
        "--policy-message-stage",
        "--policy-coordinator",
        "--policy-fast-coordinator",
    ] {
        let temp = tempfile::tempdir().unwrap();
        repo(temp.path());
        let config = Config {
            state_dir: temp.path().join("runs"),
            codex_command: support::codex(&["--loop-test", flag]),
            run_timeout: 12,
            agent_timeout: 12,
            ..Config::default()
        };
        let _cleanup = Cleanup(config.state_dir.clone());
        let started = std::time::Instant::now();
        let state =
            aaqe::audit::workflow::run_audit("audit", temp.path(), &config, &|_| {}).unwrap();
        let directory = config.state_dir.join(state["id"].as_str().unwrap());
        assert!(
            started.elapsed().as_secs() < 10,
            "policy refusal waited for timeout"
        );
        assert_eq!(state["status"], "blocked", "{state}");
        assert_eq!(state["workflow"], "product");
        assert!(
            state["error"].as_str().unwrap().contains("安全策略拒绝"),
            "{state}"
        );
        assert!(directory.join("policy-denial.json").is_file());
        assert!(
            !directory
                .join("stages/coordinator/continued-after-policy")
                .exists()
        );
        if !flag.ends_with("coordinator") {
            assert!(!directory.join("artifacts/scenario-designer.json").exists());
            assert!(
                !directory
                    .join("stages/scenario-designer/continued-after-policy")
                    .exists()
            );
            assert_eq!(state["stages"].as_array().unwrap().len(), 3);
        }
        let report = fs::read_to_string(directory.join("report.md")).unwrap();
        assert!(report.contains("安全策略拒绝"), "{report}");
    }
}

#[test]
fn native_coordinator_retains_one_interactive_tui_and_fresh_background_stages() {
    let temp = tempfile::tempdir().unwrap();
    repo(temp.path());
    let config = Config {
        state_dir: temp.path().join("runs"),
        codex_command: support::codex(&["--loop-test"]),
        run_timeout: 3,
        ..Config::default()
    };
    let _cleanup = Cleanup(config.state_dir.clone());
    let state = aaqe::audit::workflow::run_audit("audit", temp.path(), &config, &|_| {}).unwrap();
    let directory = config.state_dir.join(state["id"].as_str().unwrap());
    assert_eq!(state["status"], "blocked", "{state}");
    assert_eq!(state["workflow"], "product");
    assert!(
        state["error"]
            .as_str()
            .unwrap()
            .contains("真实产品服务部署")
    );
    assert!(directory.join("steps.md").is_file());
    assert!(directory.join("report.md").is_file());
    let report = read_json(&directory.join("report.json")).unwrap();
    assert_eq!(report["schema_version"], "2.0");
    assert_eq!(report["scenario_results"][0]["status"], "blocked");
    assert_eq!(
        report["scenario_results"][0]["assessment"]["executed"],
        false
    );
    assert!(
        report["execution"]["operations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let coordinator = directory.join("stages/coordinator");
    let metadata = read_json(&coordinator.join("tmux.json")).unwrap();
    let session = metadata["session"].as_str().unwrap();
    assert_eq!(session, format!("aaqe-{}", state["id"].as_str().unwrap()));
    let terminal = read_json(&coordinator.join("terminal.json")).unwrap();
    assert_eq!(terminal, json!({"stdin":true,"stdout":true,"stderr":true}));
    std::thread::sleep(std::time::Duration::from_millis(3200));
    assert!(
        !coordinator.join("finished").exists(),
        "TUI exited with the audit"
    );
    assert!(
        Command::new("tmux")
            .args([
                "send-keys",
                "-t",
                session,
                "follow up after report",
                "Enter"
            ])
            .status()
            .unwrap()
            .success()
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !coordinator.join("follow-up.txt").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "retained TUI did not accept input"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        fs::read_to_string(coordinator.join("follow-up.txt")).unwrap(),
        "follow up after report\n"
    );
    for path in [
        "product-explorer",
        "product-explorer/attempts/0002",
        "scenario-designer",
        "product-executor",
        "issue-reproducer",
        "result-reviewer",
    ] {
        let stage = directory.join("stages").join(path);
        assert!(stage.join("input.json").is_file());
        assert!(stage.join("prompt.md").is_file());
        assert!(stage.join("stdout.log").is_file());
        assert!(stage.join("process.json").is_file());
        assert!(!stage.join("tmux.json").exists());
    }
    cleanup_tmux(&directory).unwrap();
    assert!(
        !Command::new("tmux")
            .args(["has-session", "-t", &format!("={session}")])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(state["stages"].as_array().unwrap().len(), 6);
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
        codex_command: support::success(),
        run_timeout: 10,
        ..Config::default()
    };
    let _cleanup = Cleanup(config.state_dir.clone());
    let state = aaqe::audit::workflow::run_audit("audit", temp.path(), &config, &|_| {}).unwrap();
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
    assert_eq!(replies[1]["result"]["tools"].as_array().unwrap().len(), 6);
    let dispatch = replies[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "run_stage")
        .unwrap();
    assert_eq!(
        dispatch["inputSchema"]["properties"]["stage"]["enum"],
        json!([
            "product-explorer",
            "scenario-designer",
            "product-executor",
            "issue-reproducer",
            "result-reviewer"
        ])
    );
    assert_eq!(replies[2]["result"]["isError"], true);
    assert_eq!(replies[3]["error"]["code"], -32601);
}

#[test]
fn cancelling_or_timing_out_the_coordinator_stops_separate_stage_processes() {
    use std::{
        thread,
        time::{Duration, Instant},
    };
    if let Ok(serialized) = std::env::var("AAQE_CANCEL_TEST_CONFIG") {
        let config: Config = serde_json::from_str(&serialized).unwrap();
        aaqe::runtime::process::install_signals().unwrap();
        let state = aaqe::audit::workflow::run_audit(
            "audit",
            &std::env::current_dir().unwrap(),
            &config,
            &|line| println!("{line}"),
        )
        .unwrap();
        std::process::exit(i32::from(state["status"] != "completed"));
    }
    for cancellation in [
        "interrupt",
        "timeout",
        "delete-session",
        "exit-tui",
        "kill-mcp",
    ] {
        let temp = tempfile::tempdir().unwrap();
        repo(temp.path());
        let runs = temp.path().join("runs");
        let _cleanup = Cleanup(runs.clone());
        let config = Config {
            state_dir: runs.clone(),
            codex_command: support::codex(&[
                "--cancel-test",
                if cancellation == "kill-mcp" {
                    "--kill-mcp-test"
                } else {
                    "--normal-mcp-test"
                },
            ]),
            run_timeout: if cancellation == "timeout" { 3 } else { 30 },
            ..Config::default()
        };
        let mut child = Command::new(std::env::current_exe().unwrap())
            .current_dir(temp.path())
            .env(
                "AAQE_CANCEL_TEST_CONFIG",
                serde_json::to_string(&config).unwrap(),
            )
            .args([
                "--exact",
                "cancelling_or_timing_out_the_coordinator_stops_separate_stage_processes",
                "--nocapture",
            ])
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
                    .join("stages/product-explorer/worker.pid")
                    .exists()
            {
                break entry.path();
            }
            assert!(Instant::now() < deadline, "stage did not start");
            thread::sleep(Duration::from_millis(20));
        };
        if cancellation == "interrupt" || cancellation == "kill-mcp" {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGINT);
            }
        } else if cancellation == "delete-session" {
            let metadata = read_json(&run.join("stages/coordinator/tmux.json")).unwrap();
            assert!(
                Command::new("tmux")
                    .args(["kill-session", "-t", metadata["session"].as_str().unwrap()])
                    .status()
                    .unwrap()
                    .success()
            );
        } else if cancellation == "exit-tui" {
            let pid: i32 = fs::read_to_string(run.join("stages/coordinator/tui.pid"))
                .unwrap()
                .parse()
                .unwrap();
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(9);
        let exit = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                panic!(
                    "coordinator did not stop after {cancellation}: {}",
                    fs::read_to_string(run.join("stages/coordinator/worker.log"))
                        .unwrap_or_default()
                )
            }
            thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(exit.code(), Some(1));
        let state = read_json(&run.join("run.json")).unwrap();
        assert_eq!(state["workflow"], "product");
        assert_eq!(
            state["status"],
            if cancellation == "interrupt" || cancellation == "kill-mcp" {
                "interrupted"
            } else {
                "blocked"
            },
            "{state}"
        );
        let stage = run.join("stages/product-explorer");
        let pid = fs::read_to_string(stage.join("worker.pid")).unwrap();
        if let Ok(stat) = fs::read_to_string(format!("/proc/{}/stat", pid.trim())) {
            assert_eq!(
                stat.split_whitespace().nth(2),
                Some("Z"),
                "stage worker survived coordinator cancellation"
            );
        }
        assert!(stage.join("finished").exists());
        assert_eq!(
            read_json(&stage.join("process.json")).unwrap()["returncode"],
            130
        );
        let child_pid = fs::read_to_string(stage.join("child.pid")).unwrap();
        if let Ok(stat) = fs::read_to_string(format!("/proc/{}/stat", child_pid.trim())) {
            assert_eq!(
                stat.split_whitespace().nth(2),
                Some("Z"),
                "stage descendant survived cancellation"
            );
        }
        assert!(run.join("report.md").is_file());
        cleanup_tmux(&run).unwrap();
    }
}

#[test]
fn environment_tools_advertise_validated_operation_inputs() {
    let tools = aaqe::agents::mcp::tools();
    for (name, args) in [
        (
            "inspect_environment",
            json!({"probe":{"kind":"tools","tools":["git"]}}),
        ),
        (
            "prepare_environment",
            json!({"environment":{"compose":{"services":{"runner":{"image":"local"}}},"runner_service":"runner"}}),
        ),
        ("execute_tests", json!({"test_ids":["one"]})),
    ] {
        let tool = tools
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .expect("missing environment tool");
        jsonschema::validator_for(&tool["inputSchema"])
            .unwrap()
            .validate(&args)
            .unwrap();
    }
}
