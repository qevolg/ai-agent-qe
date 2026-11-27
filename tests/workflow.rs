mod support;
use aaqe::{
    audit::{
        interfaces::{AuditAgent, TestRuntime},
        workflow::run_audit_with,
    },
    config::Config,
    runtime::compose::ComposeEnvironment,
    runtime::git::git,
    runtime::tmux::cleanup_tmux,
    state::read_json,
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
    support::repository(temp, false)
}

struct Agent {
    mode: &'static str,
}
impl AuditAgent for Agent {
    fn coordinate(&self, harness: &mut aaqe::audit::harness::AuditHarness<'_>) -> Result<()> {
        for stage in aaqe::agents::stage::STAGES {
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
                json!({"summary":"explored","scope":["app.rs"],"references":[{"id":"source","path":if self.mode=="invented" {"missing.rs"} else {"app.rs"},"line":1,"claim":"one"}],"uncertainties":[],"environment":{"compose_files":["compose.yaml"],"services":["runner"],"runner_service":"runner","runner_workdir":"/workspace","external_containers":[],"readiness_timeout":30},"baseline":[["baseline"]]})
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
                json!({"summary":"test","coverage":{"command":["coverage"],"report":"coverage.json","files":["app.rs"]},"patch":"--- /dev/null\n+++ b/tests/generated.rs\n@@ -0,0 +1 @@\n+#[test] fn regression() { assert!(false); }\n","tests":[{"id":"test","candidate_id":"bug","command":["test"],"report":"junit.xml","expected_names":["test"],"reproduction_commands":[]}],"limitations":[]})
            }
            "test-validator" => {
                json!({"summary":"checked","coverage_assessment":{"scope_valid":true,"measurement_valid":true,"meaningful_assertions":true,"rationale":"Reviewed native evidence and complete production scope"},"verdicts":[{"candidate_id":"bug","test_ids":["test"],"status":"confirmed","assessment":{"executed":true,"assertion_failed":true,"infrastructure_failure":false},"rationale":"assertion failed","evidence":[if self.mode=="fabricated" {"made-up.json".to_owned()} else {format!("{}/execution.json",inputs["execution"]["tests"][0]["directory"].as_str().unwrap())}]}]})
            }
            _ => bail!("unexpected stage"),
        };
        if stage == "test-validator" && self.mode == "missing" {
            output["verdicts"][0]["assessment"]["executed"] = json!(false);
        }
        if stage == "test-validator" && self.mode == "coverage-rejected" {
            output["coverage_assessment"]["measurement_valid"] = json!(false);
        }
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
            let result = aaqe::runtime::process::run(
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
        if id.starts_with("coverage-") {
            let directory = self.root.join(id);
            fs::create_dir_all(&directory)?;
            fs::write(directory.join("raw.txt"), "measurement")?;
            fs::write(directory.join("coverage.json"), json!({"files":[{"path":"app.rs","covered_lines":1,"executable_lines":1}],"raw_reports":["raw.txt"]}).to_string())?;
            return Ok(
                json!({"test_id":id,"directory":directory,"result":{"returncode":0,"timed_out":false}}),
            );
        }
        let baseline = id.starts_with("baseline");
        // This observes the actual git patch reversal, rather than mocking that operation.
        assert_eq!(
            self.workspace.join("tests/generated.rs").exists(),
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
    aaqe::runtime::process::install_signals().unwrap();
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
            .join("tests/generated.rs")
            .is_file()
    );
    let dir = config.state_dir.join(state["id"].as_str().unwrap());
    assert!(dir.join("executions/logs-collected").is_file());
    assert_eq!(
        read_json(&dir.join("report.json")).unwrap()["status"],
        "interrupted"
    );
    assert!(aaqe::runtime::process::interrupted());
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
        "coverage-rejected",
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
        assert!(!root.join("tests/generated.rs").exists());
        match mode {
            "empty" => {
                assert_eq!(state["status"], "completed");
                assert_eq!(state["stages"].as_array().unwrap().len(), 2);
                assert!(state["environment"].is_null());
            }
            "fail" | "invented" | "blocked" | "bad-baseline" | "coverage-rejected" => {
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
            .args(["report", "../secret"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    let temp = tempfile::tempdir().unwrap();
    let mut child = binary()
        .current_dir(temp.path())
        .env("HOME", temp.path())
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
    assert_eq!(fs::read_dir(temp.path().join(".aaqe")).unwrap().count(), 1);
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
#[ignore = "requires a running Docker daemon and tmux"]
fn real_docker_workflow_reproduction_and_owned_cleanup() {
    real_docker_scenario(false);
    real_docker_scenario(true);
}

fn real_docker_scenario(early_inline: bool) {
    eprintln!("real Docker scenario: early_inline={early_inline}");
    let temp = tempfile::tempdir().unwrap();
    let root = repository(temp.path());
    support::project::create(&root);
    if early_inline {
        fs::remove_file(root.join("compose.yaml")).unwrap();
    }
    command(&root, &["add", "."]);
    command(&root, &["commit", "-m", "runnable example"]);
    command(&root, &["push", "origin", "main"]);
    // The fixture is a model boundary; actual Git, tmux, Docker and LLVM coverage are used.
    let config_file = root.join("aaqe.yaml");
    let mut config: Value =
        serde_yaml::from_str(&fs::read_to_string(&config_file).unwrap()).unwrap();
    let mut fixture_flags = vec!["--retry-developer"];
    if early_inline {
        fixture_flags.push("--early-inline");
        config["environment"] = json!({});
        config["baseline"] = json!([]);
    }
    fs::write(&config_file, serde_yaml::to_string(&config).unwrap()).unwrap();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    support::executable(
        &bin.join("codex"),
        json!({"mode":"codex","flags":fixture_flags}),
    );
    let runs = temp.path().join(".aaqe");
    let output = binary()
        .current_dir(&root)
        .env("HOME", temp.path())
        .env("PATH", support::path_with(&bin))
        .args(["run", "Verify the documented inclusive bounds"])
        .output()
        .unwrap();
    let dir = fs::read_dir(&runs).unwrap().next().unwrap().unwrap().path();
    let mut state = read_json(&dir.join("run.json")).unwrap();
    let _cleanup = Cleanup {
        run: dir.clone(),
        state: state.clone(),
    };
    let report = read_json(&dir.join("report.json")).unwrap();
    assert!(
        output.status.success(),
        "{}\n{}\nCoordinator: {}\nCoverage: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(dir.join("stages/coordinator/terminal.log")).unwrap_or_default(),
        serde_json::to_string_pretty(&report["execution"]["coverage"]).unwrap()
    );
    if early_inline {
        let workspace = Path::new(state["workspace"]["path"].as_str().unwrap());
        assert!(!workspace.join("compose.yaml").exists());
        assert!(!workspace.join("readiness-must-not-write").exists());
        assert!(dir.join("environment/compose.generated.json").is_file());
        assert_eq!(
            fs::read_to_string(dir.join("executions/early-ready")).unwrap(),
            "ready before Explorer and Developer"
        );
        let readiness: Vec<_> = fs::read_dir(dir.join("environment/readiness-1"))
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
        assert!(
            codes.contains(&0) && codes.iter().any(|code| *code != 0),
            "{codes:?}"
        );
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
    fs::remove_file(dir.join("report.md")).unwrap();
    fs::write(dir.join("report.json"), "broken derived report").unwrap();
    let rebuilt = binary()
        .env("HOME", temp.path())
        .args(["report", &id])
        .output()
        .unwrap();
    assert!(
        rebuilt.status.success(),
        "{}",
        String::from_utf8_lossy(&rebuilt.stdout)
    );
    assert_eq!(
        read_json(&dir.join("report.json")).unwrap()["verdicts"],
        report["verdicts"]
    );
    assert!(
        binary()
            .env("HOME", temp.path())
            .args(["env", "status", &id])
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
        runtime
            .record_setup_with_replace(
                &[vec![
                    "/aaqe-results/project-runner".into(),
                    "installer".into(),
                ]],
                true,
            )
            .unwrap();
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
    if !early_inline {
        // A later preparation failure must retain an earlier target's evidence.
        let mut saved = read_json(&dir.join("harness.json")).unwrap();
        let developer = saved["artifacts"]["test-developer"].clone();
        let mut second = developer["tests"][0].clone();
        second["id"] = json!("failed-preparation");
        second["reproduction_commands"] = json!([["/aaqe-results/project-runner", "fail"]]);
        saved["artifacts"]["test-developer"]["tests"]
            .as_array_mut()
            .unwrap()
            .push(second);
        aaqe::state::write_json(&dir.join("harness.json"), &saved).unwrap();
        aaqe::state::write_json(
            &dir.join("artifacts/test-developer.json"),
            &saved["artifacts"]["test-developer"],
        )
        .unwrap();
        let failed = binary()
            .env("HOME", temp.path())
            .args(["reproduce", &id])
            .output()
            .unwrap();
        assert_eq!(failed.status.code(), Some(2));
        let mut saved = read_json(&dir.join("harness.json")).unwrap();
        let attempt = &saved["state"]["reproductions"][0];
        assert_eq!(attempt["status"], "failed");
        assert_eq!(attempt["records"][0]["test_id"], "negative-input");
        assert_eq!(
            attempt["preparations"].as_array().unwrap().last().unwrap()["result"]["returncode"],
            9
        );
        assert_eq!(saved["execution"], report["execution"]);
        assert_eq!(saved["verdicts"], report["verdicts"]);
        saved["artifacts"]["test-developer"] = developer.clone();
        aaqe::state::write_json(&dir.join("harness.json"), &saved).unwrap();
        aaqe::state::write_json(&dir.join("artifacts/test-developer.json"), &developer).unwrap();
    }
    let replay = binary()
        .env("HOME", temp.path())
        .args(["reproduce", &id, "--case", "negative-input"])
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stdout)
    );
    let records: Value = serde_json::from_slice(&replay.stdout).unwrap();
    assert_eq!(records[0]["evidence"]["complete"], true);
    assert_eq!(records[0]["result"]["returncode"], 1);
    if early_inline {
        assert!(!dir.join("executions/installer-leaked").exists());
    }
    assert_ne!(
        records[0]["directory"],
        report["execution"]["tests"][0]["directory"]
    );
    let updated = read_json(&dir.join("report.json")).unwrap();
    let reproduction = updated["reproductions"].as_array().unwrap().last().unwrap();
    assert_eq!(reproduction["coverage"]["status"], "passed");
    assert_eq!(reproduction["coverage"]["percent"], 100.0);
    assert_ne!(
        reproduction["coverage"]["record"]["directory"],
        report["execution"]["coverage"]["record"]["directory"]
    );
    assert_eq!(updated["execution"], report["execution"]);
    assert_eq!(updated["verdicts"], report["verdicts"]);
    // Rendering failure must not prevent actual Docker cleanup or state persistence.
    fs::remove_file(dir.join("report.md")).unwrap();
    fs::create_dir(dir.join("report.md")).unwrap();
    let failed_report = binary()
        .env("HOME", temp.path())
        .args(["cleanup", &id])
        .output()
        .unwrap();
    assert_eq!(failed_report.status.code(), Some(2));
    let cleaned = read_json(&dir.join("run.json")).unwrap();
    assert!(cleaned["environment_cleaned_at"].is_string());
    assert_eq!(cleaned["environment_retained"], false);
    let runtime = ComposeEnvironment::new(
        Path::new(cleaned["workspace"]["path"].as_str().unwrap()),
        &dir,
        &id,
        cleaned["environment"].clone(),
        120,
    );
    let mut ps = runtime.arguments(true).unwrap();
    ps.extend(["ps".into(), "--all".into(), "--quiet".into()]);
    let remaining = Command::new(&ps[0]).args(&ps[1..]).output().unwrap();
    assert!(remaining.status.success() && remaining.stdout.is_empty());
    fs::remove_dir(dir.join("report.md")).unwrap();
    for _ in 0..2 {
        let cleanup = binary()
            .env("HOME", temp.path())
            .args(["cleanup", &id])
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
    for stage in aaqe::agents::stage::STAGES
        .into_iter()
        .chain(["test-developer/attempts/0002"])
    {
        assert!(!dir.join("stages").join(stage).join("tmux.json").exists());
    }
    let rejected = binary()
        .env("HOME", temp.path())
        .args(["reproduce", &id])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(2));
}
