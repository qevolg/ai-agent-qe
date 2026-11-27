use aaqe::{
    config::Config,
    environment::ComposeEnvironment,
    gitops::git,
    state::{Store, read_json},
    tmux::cleanup_tmux,
    workflow::{AuditAgent, TestRuntime, run_audit_with},
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn command(root: &Path, args: &[&str]) -> String {
    git(root, args).unwrap()
}
fn repository(temp: &Path) -> PathBuf {
    let remote = temp.join("remote.git");
    fs::create_dir(&remote).unwrap();
    command(&remote, &["init", "--bare"]);
    let root = temp.join("repo");
    fs::create_dir(&root).unwrap();
    command(&root, &["init", "-b", "main"]);
    command(&root, &["config", "user.email", "test@example.invalid"]);
    command(&root, &["config", "user.name", "AAQE Test"]);
    fs::write(root.join("app.py"), "def value():\n    return 1\n").unwrap();
    command(&root, &["add", "."]);
    command(&root, &["commit", "-m", "base"]);
    command(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    command(&root, &["push", "origin", "main"]);
    root
}
struct Agent {
    mode: &'static str,
}
impl AuditAgent for Agent {
    fn coordinate(&self, harness: &mut aaqe::harness::AuditHarness<'_>) -> Result<()> {
        for stage in aaqe::workflow::STAGES {
            let result = harness.run_stage(stage, "scripted regression scenario")?;
            if result["status"] == "failed" {
                return harness.finish("blocked", result["error"].as_str().unwrap());
            }
            if stage == "bug-hunter" && self.mode == "empty" {
                break;
            }
            if stage == "test-designer" && self.mode == "blocked" {
                return harness.finish("blocked", "All test plans are blocked");
            }
        }
        harness.finish("completed", "scripted scenario complete")
    }

    fn invoke(
        &self,
        stage: &str,
        _: &Path,
        _: &Path,
        inputs: &Value,
        _: &dyn Fn(&str),
    ) -> Result<Value> {
        let mut output = match stage {
            "code-explorer" => {
                json!({"summary":"explored","scope":["app.py"],"references":[{"id":"source","path":if self.mode=="invented" {"missing.py"} else {"app.py"},"line":1,"claim":"one"}],"uncertainties":[],"environment":{"compose_files":["compose.yaml"],"services":["runner"],"runner_service":"runner","runner_workdir":"/workspace","external_containers":[],"readiness_timeout":30},"baseline":[["baseline"]]})
            }
            "bug-hunter" => {
                if self.mode == "fail" {
                    bail!("model transport unavailable");
                }
                json!({"summary":"candidate","candidates":if self.mode=="empty" {json!([])} else {json!([{"id":"bug","title":"wrong value","expected":"two","observed":"one","trigger":"value()","evidence_ids":["source"],"confidence":"high"}])},"excluded_areas":[]})
            }
            "test-designer" => {
                json!({"summary":"plan","plans":[{"candidate_id":"bug","test_id":"test","expectation":"two","oracle":"requirement","steps":["call"],"reproduction":["rerun"],"blocked_reason":if self.mode=="blocked" {json!("no requirement")} else {Value::Null}}]})
            }
            "test-developer" => {
                json!({"summary":"test","patch":"--- /dev/null\n+++ b/tests/test_generated.py\n@@ -0,0 +1 @@\n+assert False\n","tests":[{"id":"test","candidate_id":"bug","command":["test"],"report":"junit.xml","expected_names":["test"],"reproduction_commands":[]}],"limitations":[]})
            }
            "test-validator" => {
                json!({"summary":"checked","verdicts":[{"candidate_id":"bug","test_ids":["test"],"status":"confirmed","rationale":"assertion failed","evidence":[if self.mode=="fabricated" {"made-up.json".to_owned()} else {format!("{}/execution.json",inputs["execution"]["tests"][0]["directory"].as_str().unwrap())}]}]})
            }
            _ => bail!("unexpected stage"),
        };
        if self.mode == "float-line" && stage == "code-explorer" {
            output["references"][0]["line"] = json!(1.0);
        }
        Ok(output)
    }
}
struct Runtime {
    root: PathBuf,
    mode: &'static str,
    workspace: PathBuf,
}
impl TestRuntime for Runtime {
    fn start(&self) -> Result<()> {
        Ok(())
    }
    fn logs(&self) -> Result<()> {
        if self.mode == "interrupt-baseline" {
            let result = aaqe::process::run(
                &[
                    "touch".into(),
                    self.root
                        .join("logs-collected")
                        .to_string_lossy()
                        .into_owned(),
                ],
                None,
                5,
                None,
            );
            assert_eq!(result.returncode, 0);
        }
        Ok(())
    }
    fn execute_test(&self, command: &[String], id: &str, _: u64) -> Result<Value> {
        let baseline = id.starts_with("baseline");
        // This observes the actual git patch reversal, rather than mocking that operation.
        assert_eq!(
            self.workspace.join("tests/test_generated.py").exists(),
            !baseline
        );
        let directory = self.root.join(id);
        fs::create_dir_all(&directory)?;
        if baseline && self.mode == "interrupt-baseline" {
            unsafe {
                libc::raise(libc::SIGINT);
            }
            return Ok(
                json!({"test_id":id,"directory":directory,"attempt":id,"result":{"argv":command,"returncode":130,"stdout":"","stderr":"","elapsed":0.0,"timed_out":false}}),
            );
        }
        if !baseline && self.mode != "missing" {
            fs::write(
                directory.join("junit.xml"),
                "<testsuite><testcase name=\"test\"><failure message=\"expected two\"/></testcase></testsuite>",
            )?;
        }
        Ok(
            json!({"test_id":id,"directory":directory,"attempt":id,"result":{"argv":command,"returncode":if baseline {if self.mode=="bad-baseline" {1} else {0}} else if self.mode=="mismatch" {0} else {1},"stdout":"","stderr":"","elapsed":0.0,"timed_out":false}}),
        )
    }
}

#[test]
fn interrupted_baseline_restores_generated_patch_and_collects_logs() {
    if std::env::var_os("AAQE_INTERRUPT_BASELINE_CHILD").is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "interrupted_baseline_restores_generated_patch_and_collects_logs",
                "--nocapture",
            ])
            .env("AAQE_INTERRUPT_BASELINE_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    aaqe::process::install_signals().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = repository(temp.path());
    let config = Config {
        state_dir: temp.path().join("runs"),
        ..Config::default()
    };
    let state = run_audit_with(
        "inspect",
        &root,
        &config,
        &Agent { mode: "normal" },
        &|workspace, directory, _, _, _| {
            Box::new(Runtime {
                root: directory.join("executions"),
                workspace: workspace.into(),
                mode: "interrupt-baseline",
            })
        },
        &|_| {},
    )
    .unwrap();
    assert_eq!(state["status"], "interrupted");
    assert!(
        Path::new(state["workspace"]["path"].as_str().unwrap())
            .join("tests/test_generated.py")
            .is_file()
    );
    let dir = config.state_dir.join(state["id"].as_str().unwrap());
    assert!(dir.join("executions/logs-collected").is_file());
    assert_eq!(
        read_json(&dir.join("report.json")).unwrap()["status"],
        "interrupted"
    );
    assert!(aaqe::process::interrupted());
}
#[test]
fn workflow_preserves_stages_baseline_patch_evidence_and_partial_reports() {
    for mode in [
        "normal",
        "float-line",
        "empty",
        "fail",
        "invented",
        "blocked",
        "missing",
        "fabricated",
        "mismatch",
        "bad-baseline",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = repository(temp.path());
        let config = Config {
            state_dir: temp.path().join("runs"),
            ..Config::default()
        };
        let state = run_audit_with(
            "检查模块",
            &root,
            &config,
            &Agent { mode },
            &|workspace, directory, _, _, _| {
                Box::new(Runtime {
                    root: directory.join("executions"),
                    workspace: workspace.into(),
                    mode,
                })
            },
            &|_| {},
        )
        .unwrap();
        let dir = config.state_dir.join(state["id"].as_str().unwrap());
        let report = read_json(&dir.join("report.json")).unwrap();
        assert!(dir.join("report.md").is_file());
        assert_eq!(state["stage"], 6);
        assert!(!root.join("tests/test_generated.py").exists());
        match mode {
            "empty" => {
                assert_eq!(state["status"], "completed");
                assert_eq!(state["stages"].as_array().unwrap().len(), 2);
                assert!(state["environment"].is_null());
            }
            "fail" | "invented" | "blocked" | "bad-baseline" => {
                assert_eq!(state["status"], "blocked");
                assert!(state["error"].is_string());
            }
            _ => {
                assert_eq!(state["status"], "completed");
                assert_eq!(state["stages"].as_array().unwrap().len(), 5);
                assert_eq!(
                    report["verdicts"][0]["status"],
                    if mode == "normal" || mode == "float-line" {
                        "confirmed"
                    } else {
                        "blocked"
                    }
                );
            }
        }
    }
}
fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_aaqe"))
}
#[test]
fn cli_saved_python_run_compatibility_and_cleanup_idempotence() {
    let temp = tempfile::tempdir().unwrap();
    let config = Config {
        state_dir: temp.path().join("runs"),
        ..Config::default()
    };
    let store = Store::new(&config.state_dir);
    // Captured with the original Python Store and generate_report before removal.
    let mut state: Value = serde_json::from_str(include_str!("fixtures/python-run.json")).unwrap();
    let id = state["id"].as_str().unwrap().to_owned();
    let dir = store.path(&id).unwrap();
    store.save(&state).unwrap();
    fs::write(
        dir.join("report.json"),
        include_str!("fixtures/python-report.json"),
    )
    .unwrap();
    fs::write(dir.join("report.md"), "Legacy Python run compatibility\n").unwrap();
    state["status"] = json!("running");
    store.save(&state).unwrap();
    let active = binary()
        .args([
            "--state-dir",
            config.state_dir.to_str().unwrap(),
            "cleanup",
            &id,
        ])
        .output()
        .unwrap();
    assert_eq!(active.status.code(), Some(2));
    assert!(store.load(&id).unwrap()["tmux_cleaned_at"].is_null());
    state["status"] = json!("completed");
    store.save(&state).unwrap();
    for _ in 0..2 {
        let result = binary()
            .args([
                "--state-dir",
                config.state_dir.to_str().unwrap(),
                "cleanup",
                &id,
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
    }
    let report = binary()
        .args([
            "--state-dir",
            config.state_dir.to_str().unwrap(),
            "report",
            &id,
        ])
        .output()
        .unwrap();
    assert!(report.status.success());
    assert!(String::from_utf8_lossy(&report.stdout).contains("Legacy Python run compatibility"));
    assert!(store.load(&id).unwrap()["tmux_cleaned_at"].is_string());
    assert_eq!(
        read_json(&dir.join("report.json")).unwrap()["tmux_cleaned_at"],
        store.load(&id).unwrap()["tmux_cleaned_at"]
    );
}
#[test]
fn cli_prompt_options_version_and_invalid_inputs() {
    assert!(binary().arg("--version").output().unwrap().status.success());
    assert!(binary().arg("--help").output().unwrap().status.success());
    assert_eq!(
        binary()
            .args(["run", "   "])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        binary()
            .args(["--state-dir", "/tmp", "report", "../secret"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    let temp = tempfile::tempdir().unwrap();
    let mut child = binary()
        .current_dir(temp.path())
        .args(["--state-dir", temp.path().join("runs").to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"inspect module\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Describe the product/module"));
    assert_eq!(fs::read_dir(temp.path().join("runs")).unwrap().count(), 1);
}

struct Cleanup {
    run: PathBuf,
    state: Value,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = cleanup_tmux(&self.run);
        if self.state["environment"].is_object()
            && self.run.join("environment/ownership.json").exists()
        {
            let _ = ComposeEnvironment::new(
                Path::new(self.state["workspace"]["path"].as_str().unwrap()),
                &self.run,
                self.state["id"].as_str().unwrap(),
                self.state["environment"].clone(),
                60,
            )
            .cleanup();
        }
    }
}
#[test]
#[ignore = "requires a running Docker daemon, tmux and python3 for the deterministic model fixture"]
fn real_docker_workflow_reproduction_and_owned_cleanup() {
    real_docker_scenario(false);
    real_docker_scenario(true);
}

fn real_docker_scenario(early_inline: bool) {
    eprintln!("real Docker scenario: early_inline={early_inline}");
    let temp = tempfile::tempdir().unwrap();
    let root = repository(temp.path());
    let project = Path::new(env!("CARGO_MANIFEST_DIR"));
    for name in [
        "README.md",
        "compose.yaml",
        "run_tests.py",
        "app.py",
        "aaqe.yaml",
    ] {
        fs::copy(project.join("examples/minimal").join(name), root.join(name)).unwrap();
    }
    fs::create_dir(root.join("tests")).unwrap();
    fs::copy(
        project.join("examples/minimal/tests/test_baseline.py"),
        root.join("tests/test_baseline.py"),
    )
    .unwrap();
    if early_inline {
        fs::remove_file(root.join("compose.yaml")).unwrap();
    }
    command(&root, &["add", "."]);
    command(&root, &["commit", "-m", "runnable example"]);
    command(&root, &["push", "origin", "main"]);
    // The fixture is a model boundary; actual Git, tmux, Docker and JUnit execution are used.
    let config_file = root.join("aaqe.yaml");
    let mut config: Value =
        serde_yaml::from_str(&fs::read_to_string(&config_file).unwrap()).unwrap();
    let mut fixture_command = vec![
        json!("python3"),
        json!(project.join("tests/fixtures/scripted_codex.py")),
        json!("--retry-developer"),
    ];
    if early_inline {
        fixture_command.push(json!("--early-inline"));
        config["environment"] = json!({});
        config["baseline"] = json!([]);
    }
    config["codex"] = json!({"command":fixture_command});
    fs::write(&config_file, serde_yaml::to_string(&config).unwrap()).unwrap();
    let runs = temp.path().join("runs");
    let output = binary()
        .current_dir(&root)
        .args([
            "--state-dir",
            runs.to_str().unwrap(),
            "run",
            "Verify the documented inclusive bounds",
        ])
        .output()
        .unwrap();
    let dir = fs::read_dir(&runs).unwrap().next().unwrap().unwrap().path();
    let mut state = read_json(&dir.join("run.json")).unwrap();
    let _cleanup = Cleanup {
        run: dir.clone(),
        state: state.clone(),
    };
    assert!(
        output.status.success(),
        "{}\n{}\nCoordinator: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(dir.join("stages/coordinator/terminal.log")).unwrap_or_default()
    );
    let report = read_json(&dir.join("report.json")).unwrap();
    if early_inline {
        let workspace = Path::new(state["workspace"]["path"].as_str().unwrap());
        assert!(!workspace.join("compose.yaml").exists());
        assert!(!workspace.join("readiness-must-not-write").exists());
        assert!(dir.join("environment/compose.generated.json").is_file());
        assert_eq!(
            fs::read_to_string(dir.join("executions/early-ready")).unwrap(),
            "ready before Explorer and Developer"
        );
        let readiness: Vec<_> = fs::read_dir(dir.join("environment/readiness-0"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(
            readiness.len(),
            2,
            "Ready reuse reran readiness unexpectedly"
        );
        let codes: Vec<_> = readiness
            .iter()
            .map(|path| {
                read_json(&path.join("process.json")).unwrap()["returncode"]
                    .as_i64()
                    .unwrap()
            })
            .collect();
        assert!(codes.contains(&0) && codes.contains(&1), "{codes:?}");
        assert!(dir.join("environment/dependency-setup.json").is_file());
        let explorer = read_json(&dir.join("stages/code-explorer/input.json")).unwrap();
        assert_eq!(explorer["environment_ready"], true);
        assert_eq!(
            report["environment_actions"][0]["operation"],
            "inspect_environment"
        );
        assert_eq!(
            report["environment_actions"][1]["operation"],
            "prepare_environment"
        );
        assert_eq!(report["execution_attempts"].as_array().unwrap().len(), 3);
        assert_eq!(
            report["execution"]["baseline"][0]["for_test_id"],
            "negative-input"
        );
        assert!(
            report["execution_attempts"][2]["directory"]
                .as_str()
                .unwrap()
                .contains("executions/rounds/")
        );
    }
    assert_eq!(report["verdicts"][0]["status"], "confirmed");
    assert_eq!(
        report["execution"]["baseline"][0]["result"]["returncode"],
        0
    );
    assert_eq!(report["execution"]["tests"][0]["result"]["returncode"], 1);
    let id = state["id"].as_str().unwrap().to_owned();
    assert!(
        binary()
            .args(["--state-dir", runs.to_str().unwrap(), "env", "status", &id])
            .output()
            .unwrap()
            .status
            .success()
    );
    if early_inline {
        let runtime = ComposeEnvironment::new(
            Path::new(state["workspace"]["path"].as_str().unwrap()),
            &dir,
            &id,
            state["environment"].clone(),
            2,
        );
        let saved = runtime.saved_setup().unwrap();
        runtime.record_setup_with_replace(&[vec!["sh".into(), "-c".into(), "echo started > /aaqe-results/installer-started; sleep 8; echo leaked > /aaqe-results/installer-leaked".into()]], true).unwrap();
        assert!(runtime.start().is_err());
        assert!(
            dir.join("executions/installer-started").exists(),
            "setup never started"
        );
        let mut stopped = runtime.arguments(true).unwrap();
        stopped.extend([
            "ps".into(),
            "--status".into(),
            "running".into(),
            "--quiet".into(),
            "runner".into(),
        ]);
        let status = Command::new(&stopped[0])
            .args(&stopped[1..])
            .output()
            .unwrap();
        assert!(status.status.success());
        assert!(
            status.stdout.is_empty(),
            "timed-out installer still has a running container"
        );
        runtime.record_setup_with_replace(&saved, true).unwrap();
        let runtime = ComposeEnvironment::new(
            Path::new(state["workspace"]["path"].as_str().unwrap()),
            &dir,
            &id,
            state["environment"].clone(),
            120,
        );
        let argv = runtime.arguments(true).unwrap();
        let removed = Command::new(&argv[0])
            .args(&argv[1..])
            .args(["rm", "--stop", "--force", "runner"])
            .output()
            .unwrap();
        assert!(
            removed.status.success(),
            "{}",
            String::from_utf8_lossy(&removed.stderr)
        );
    }
    let replay = binary()
        .args([
            "--state-dir",
            runs.to_str().unwrap(),
            "reproduce",
            &id,
            "--case",
            "negative-input",
        ])
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stdout)
    );
    let records: Value = serde_json::from_slice(&replay.stdout).unwrap();
    assert_eq!(records[0]["junit"]["complete"], true);
    assert_eq!(records[0]["junit"]["cases"][0]["status"], "failed");
    if early_inline {
        assert!(!dir.join("executions/installer-leaked").exists());
    }
    assert_ne!(
        records[0]["directory"],
        report["execution"]["tests"][0]["directory"]
    );
    let updated = read_json(&dir.join("report.json")).unwrap();
    assert_eq!(updated["execution"], report["execution"]);
    assert_eq!(updated["verdicts"], report["verdicts"]);
    for _ in 0..2 {
        let cleanup = binary()
            .args(["--state-dir", runs.to_str().unwrap(), "cleanup", &id])
            .output()
            .unwrap();
        assert!(
            cleanup.status.success(),
            "{}",
            String::from_utf8_lossy(&cleanup.stdout)
        );
    }
    state = read_json(&dir.join("run.json")).unwrap();
    assert!(state["environment_cleaned_at"].is_string());
    assert!(state["tmux_cleaned_at"].is_string());
    assert_eq!(state["environment_retained"], false);
    assert_eq!(state["stages"].as_array().unwrap().len(), 6);
    let first = read_json(&dir.join("stages/test-developer/execution.json")).unwrap();
    let second =
        read_json(&dir.join("stages/test-developer/attempts/0002/execution.json")).unwrap();
    assert_ne!(
        first["tests"][0]["directory"],
        second["tests"][0]["directory"]
    );
    let session = read_json(&dir.join("stages/coordinator/tmux.json")).unwrap();
    assert!(
        !Command::new("tmux")
            .args([
                "has-session",
                "-t",
                &format!("={}", session["session"].as_str().unwrap())
            ])
            .output()
            .unwrap()
            .status
            .success()
    );
    for stage in aaqe::workflow::STAGES
        .into_iter()
        .chain(["test-developer/attempts/0002"])
    {
        assert!(!dir.join("stages").join(stage).join("tmux.json").exists());
    }
    let rejected = binary()
        .args(["--state-dir", runs.to_str().unwrap(), "reproduce", &id])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(2));
}
