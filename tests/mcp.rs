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
            assert!(!directory.join("artifacts/bug-hunter.json").exists());
            assert!(
                !directory
                    .join("stages/bug-hunter/continued-after-policy")
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
        run_timeout: 20,
        ..Config::default()
    };
    let _cleanup = Cleanup(config.state_dir.clone());
    let state = aaqe::audit::workflow::run_audit("audit", temp.path(), &config, &|_| {}).unwrap();
    let directory = config.state_dir.join(state["id"].as_str().unwrap());
    assert_eq!(state["status"], "completed", "{state}");
    assert!(directory.join("report.md").is_file());
    let coordinator = directory.join("stages/coordinator");
    let metadata = read_json(&coordinator.join("tmux.json")).unwrap();
    let session = metadata["session"].as_str().unwrap();
    assert_eq!(session, format!("aaqe-{}", state["id"].as_str().unwrap()));
    let terminal = read_json(&coordinator.join("terminal.json")).unwrap();
    assert_eq!(terminal, json!({"stdin":true,"stdout":true,"stderr":true}));
    std::thread::sleep(std::time::Duration::from_millis(20200));
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
    for path in ["code-explorer", "code-explorer/attempts/0002", "bug-hunter"] {
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
    assert_eq!(replies[1]["result"]["tools"].as_array().unwrap().len(), 7);
    assert_eq!(replies[2]["result"]["isError"], true);
    assert_eq!(replies[3]["error"]["code"], -32601);
}

#[test]
fn cancelling_or_timing_out_the_coordinator_stops_separate_stage_processes() {
    use std::{
        thread,
        time::{Duration, Instant},
    };
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
        let config = temp.path().join("test-config.yaml");
        fs::write(&config,serde_json::to_string(&json!({"state_dir":runs,"codex":{"command":support::codex(&["--cancel-test",if cancellation == "kill-mcp" {"--kill-mcp-test"}else{"--normal-mcp-test"}])},"loop":{"timeout":if cancellation == "timeout" {8} else {30}}})).unwrap()).unwrap();
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
        assert_eq!(
            state["status"],
            if cancellation == "interrupt" || cancellation == "kill-mcp" {
                "interrupted"
            } else {
                "blocked"
            },
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

#[test]
fn resume_and_takeover_reuse_the_original_tui_and_relay() {
    let temp = tempfile::tempdir().unwrap();
    repo(temp.path());
    let config = Config {
        state_dir: temp.path().join("runs"),
        codex_command: support::codex(&["--loop-test", "--resume-test"]),
        run_timeout: 30,
        ..Config::default()
    };
    let _cleanup = Cleanup(config.state_dir.clone());
    let state = aaqe::audit::workflow::run_audit("audit", temp.path(), &config, &|_| {}).unwrap();
    assert_eq!(state["status"], "blocked", "{state}");
    let directory = config.state_dir.join(state["id"].as_str().unwrap());
    let coordinator = directory.join("stages/coordinator");
    let pid = fs::read_to_string(coordinator.join("tui.pid")).unwrap();
    let session = read_json(&coordinator.join("tmux.json")).unwrap()["session"]
        .as_str()
        .unwrap()
        .to_owned();
    // The human can resume directly in the retained conversation.
    Command::new("tmux")
        .args([
            "send-keys",
            "-t",
            &session,
            "继续，从 bug-hunter 开始",
            "Enter",
        ])
        .status()
        .unwrap();
    wait_for(&coordinator.join("resumed.txt"));
    assert_eq!(
        read_json(&directory.join("report.json")).unwrap()["status"],
        "completed"
    );
    fs::remove_file(coordinator.join("resumed.txt")).unwrap();
    // Pause a request from the old relay selection before it can acquire the
    // Run lock, then let a new executor take over in that scheduling gap.
    let mut stale = Command::new(state["executor"]["path"].as_str().unwrap())
        .arg("__harness-request")
        .arg(&directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stale_input = stale.stdin.take().unwrap();
    // A newer installed executor explicitly takes over without replacing TUI/MCP.
    let output = Command::new(env!("CARGO_BIN_EXE_aaqe"))
        .args(["resume", "--run-dir"])
        .arg(&directory)
        .arg("--takeover")
        .current_dir("/")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    wait_for(&coordinator.join("resumed.txt"));
    let saved = read_json(&directory.join("harness.json")).unwrap();
    assert_eq!(saved["state"]["status"], "completed");
    assert_eq!(saved["state"]["resume_round"], 2);
    assert_ne!(
        saved["state"]["executor"]["path"],
        state["executor"]["path"]
    );
    assert_eq!(
        fs::read_to_string(coordinator.join("tui.pid")).unwrap(),
        pid
    );
    assert!(Path::new(state["executor"]["path"].as_str().unwrap()).is_file());
    let log = fs::read_to_string(coordinator.join("tools.jsonl")).unwrap();
    assert!(log.contains(saved["state"]["executor"]["path"].as_str().unwrap()));
    assert!(directory.join("resumes/0001/harness.json").is_file());
    assert!(directory.join("resumes/0002/harness.json").is_file());
    let checkpoint = fs::read(directory.join("harness.json")).unwrap();
    writeln!(
        stale_input,
        "{}",
        json!({"name":"resume","arguments":{"from_stage":"bug-hunter"}})
    )
    .unwrap();
    drop(stale_input);
    let rejected = stale.wait_with_output().unwrap();
    assert_eq!(rejected.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("Executor changed"));
    assert_eq!(
        fs::read(directory.join("harness.json")).unwrap(),
        checkpoint
    );
}

fn wait_for(path: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !path.is_file() {
        assert!(
            std::time::Instant::now() < deadline,
            "missing {}",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn retained_resume_finalizes_after_timeout_crash_or_tmux_close() {
    for failure in ["timeout", "crash", "tmux-close"] {
        let temp = tempfile::tempdir().unwrap();
        repo(temp.path());
        let config = Config {
            state_dir: temp.path().join("runs"),
            codex_command: support::codex(&[
                "--loop-test",
                "--resume-test",
                if failure == "crash" {
                    "--resume-crash"
                } else {
                    "--resume-hang"
                },
            ]),
            run_timeout: 20,
            ..Config::default()
        };
        let _cleanup = Cleanup(config.state_dir.clone());
        let state =
            aaqe::audit::workflow::run_audit("audit", temp.path(), &config, &|_| {}).unwrap();
        assert_eq!(state["status"], "blocked");
        let directory = config.state_dir.join(state["id"].as_str().unwrap());
        let coordinator = directory.join("stages/coordinator");
        let mut saved = read_json(&directory.join("harness.json")).unwrap();
        saved["state"]["config"]["run_timeout"] = json!(3);
        aaqe::state::write_json(&directory.join("harness.json"), &saved).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_aaqe"))
            .args(["resume", "--run-dir"])
            .arg(&directory)
            .arg("--takeover")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        wait_for(&coordinator.join("resuming.txt"));
        // Delayed exit notifications from a prior round or coordinator cannot
        // terminate this still-active round.
        let owner = read_json(&coordinator.join("invocation.json")).unwrap()["owner"]
            .as_str()
            .unwrap()
            .to_owned();
        let active = read_json(&directory.join("harness.json")).unwrap();
        if failure == "tmux-close" {
            let checkpoint = fs::read(directory.join("harness.json")).unwrap();
            for (old_owner, old_round) in [(&owner[..], "0"), ("old-coordinator", "1")] {
                assert!(
                    Command::new(active["state"]["executor"]["path"].as_str().unwrap())
                        .arg("__coordinator-exit")
                        .arg(&directory)
                        .args([old_owner, old_round])
                        .status()
                        .unwrap()
                        .success()
                );
                assert_eq!(
                    fs::read(directory.join("harness.json")).unwrap(),
                    checkpoint
                );
            }
        }
        if failure == "tmux-close" {
            let session = read_json(&coordinator.join("tmux.json")).unwrap()["session"]
                .as_str()
                .unwrap()
                .to_owned();
            assert!(
                Command::new("tmux")
                    .args(["kill-session", "-t", &session])
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            let report = read_json(&directory.join("report.json")).unwrap();
            let saved = read_json(&directory.join("harness.json")).unwrap();
            if saved["state"]["status"] != "running" && report["resume_round"] == 1 {
                assert_eq!(saved["state"]["audit_finalized"], true);
                assert_eq!(report["status"], saved["state"]["status"]);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{failure}: abandoned running state {saved}"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

#[test]
fn resume_launches_a_replacement_tui_only_after_the_original_is_gone() {
    let temp = tempfile::tempdir().unwrap();
    repo(temp.path());
    let config = Config {
        state_dir: temp.path().join("runs"),
        codex_command: support::codex(&["--loop-test", "--resume-test"]),
        run_timeout: 30,
        ..Config::default()
    };
    let _cleanup = Cleanup(config.state_dir.clone());
    let state = aaqe::audit::workflow::run_audit("audit", temp.path(), &config, &|_| {}).unwrap();
    let directory = config.state_dir.join(state["id"].as_str().unwrap());
    let old_pid = fs::read_to_string(directory.join("stages/coordinator/tui.pid")).unwrap();
    // A legacy coordinator has no writable terminal bridge in its read-only
    // sandbox. Recovery must not leave it running with unusable instructions.
    let mut legacy = read_json(&directory.join("harness.json")).unwrap();
    legacy["state"]["executor"] = Value::Null;
    legacy["state"]["mcp_relay_protocol"] = Value::Null;
    aaqe::state::write_json(&directory.join("harness.json"), &legacy).unwrap();
    let checkpoint = fs::read(directory.join("harness.json")).unwrap();
    let rejected = Command::new(env!("CARGO_BIN_EXE_aaqe"))
        .args(["resume", "--run-dir"])
        .arg(&directory)
        .arg("--takeover")
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&rejected.stdout).contains("Legacy coordinator"));
    assert_eq!(
        fs::read(directory.join("harness.json")).unwrap(),
        checkpoint
    );
    assert_eq!(
        fs::read_to_string(directory.join("stages/coordinator/tui.pid")).unwrap(),
        old_pid
    );
    cleanup_tmux(&directory).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aaqe"))
        .args(["resume", "--run-dir"])
        .arg(&directory)
        .args(["--from-stage", "code-explorer"])
        .current_dir("/")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let saved = read_json(&directory.join("harness.json")).unwrap();
    assert_eq!(saved["state"]["workspace"], state["workspace"]);
    assert_eq!(saved["state"]["status"], "blocked");
    assert_eq!(saved["state"]["mcp_relay_protocol"], 1);
    assert!(saved["state"]["executor"]["path"].is_string());
    assert_eq!(saved["state"]["stages"].as_array().unwrap().len(), 6);
    assert_ne!(
        fs::read_to_string(directory.join("stages/coordinator/tui.pid")).unwrap(),
        old_pid
    );
    assert!(
        directory
            .join("resumes/0001/coordinator/terminal.log")
            .is_file()
    );
    assert!(
        directory
            .join("stages/code-explorer/attempts/0003/output.json")
            .is_file()
    );
}
