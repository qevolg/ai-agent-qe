use aaqe::{
    config::Config,
    gitops::git,
    harness::AuditHarness,
    state::read_json,
    workflow::{AuditAgent, TestRuntime, run_audit_with},
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn repository(directory: &Path) -> PathBuf {
    let remote = directory.join("remote.git");
    git(directory, &["init", "--bare", remote.to_str().unwrap()]).unwrap();
    let root = directory.join("repo");
    fs::create_dir(&root).unwrap();
    for args in [
        vec!["init", "-b", "main"],
        vec!["config", "user.name", "Test"],
        vec!["config", "user.email", "test@example.invalid"],
    ] {
        git(&root, &args).unwrap();
    }
    fs::write(root.join("app.py"), "def value():\n    return 1\n").unwrap();
    for args in [
        vec!["add", "."],
        vec!["commit", "-m", "base"],
        vec!["remote", "add", "origin", remote.to_str().unwrap()],
        vec!["push", "origin", "main"],
    ] {
        git(&root, &args).unwrap();
    }
    root
}

struct Agent {
    mode: &'static str,
}

impl AuditAgent for Agent {
    fn coordinate(&self, harness: &mut AuditHarness<'_>) -> Result<()> {
        assert!(harness.run_stage("test-developer", "too early").is_err());
        assert!(harness.finish("completed", "no evidence").is_err());
        for stage in [
            "code-explorer",
            "bug-hunter",
            "test-designer",
            "test-developer",
        ] {
            let result = harness.run_stage(stage, "inspect and retain evidence")?;
            assert_eq!(result["status"], "completed", "{result}");
        }
        if self.mode == "attempt-budget" {
            assert!(
                harness
                    .run_stage("test-developer", "no attempts remain")
                    .unwrap_err()
                    .to_string()
                    .contains("budget")
            );
            return harness.finish("blocked", "attempt budget exhausted");
        }
        if self.mode == "budget" {
            assert!(
                harness
                    .run_stage("test-developer", "over budget")
                    .unwrap_err()
                    .to_string()
                    .contains("budget")
            );
            return harness.finish("blocked", "stage call budget exhausted");
        }
        if self.mode == "retry" || self.mode == "rollback" || self.mode == "stale" {
            let before = harness.status();
            let old = before["execution"]["tests"][0]["directory"].clone();
            let result = harness.run_stage(
                "test-developer",
                "repair the test fixture using actual execution feedback",
            )?;
            if self.mode == "rollback" {
                assert_eq!(result["status"], "failed");
                let workspace =
                    PathBuf::from(harness.status()["workspace"]["path"].as_str().unwrap());
                assert_eq!(
                    fs::read_to_string(workspace.join("tests/test_generated.py"))?,
                    "assert False\n"
                );
                assert_eq!(
                    fs::read_to_string(workspace.join("app.py"))?,
                    "def value():\n    return 1\n"
                );
                return harness.finish("blocked", "replacement rejected; prior patch retained");
            }
            assert_eq!(result["status"], "completed");
            assert_ne!(old, harness.status()["execution"]["tests"][0]["directory"]);
        }
        if self.mode == "crash-baseline" {
            let status = harness.status();
            let root = PathBuf::from(status["workspace"]["path"].as_str().unwrap());
            let directory = PathBuf::from(status["state"]["config"]["state_dir"].as_str().unwrap())
                .join(status["state"]["id"].as_str().unwrap());
            let patch = fs::read_to_string(directory.join("tests.diff"))?;
            let result = aaqe::process::run(
                &["git".into(), "apply".into(), "--reverse".into(), "-".into()],
                Some(&root),
                10,
                Some(&patch),
            );
            assert_eq!(result.returncode, 0);
            let mut saved = read_json(&directory.join("harness.json"))?;
            saved["state"]["baseline_restore_patch"] = json!(patch);
            aaqe::state::write_json(&directory.join("harness.json"), &saved)?;
            bail!("MCP lost while the baseline patch was reversed");
        }
        assert_eq!(
            harness.run_stage("test-validator", "independently review")?["status"],
            "completed"
        );
        if self.mode == "revisit" {
            assert_eq!(harness.status()["verdicts"][0]["status"], "confirmed");
            harness.run_stage("code-explorer", "check the behavioral oracle again")?;
            let current = harness.status();
            assert!(current["artifacts"].get("bug-hunter").is_none());
            assert_eq!(current["execution"], json!({"baseline":[],"tests":[]}));
            assert_eq!(current["verdicts"], json!([]));
            assert!(harness.finish("completed", "stale conclusions").is_err());
            return harness.finish("blocked", "scope was revised; further analysis required");
        }
        harness.finish("completed", "all candidates independently checked")
    }

    fn invoke(
        &self,
        stage: &str,
        _: &Path,
        directory: &Path,
        inputs: &Value,
        _: &dyn Fn(&str),
    ) -> Result<Value> {
        let retry = inputs["attempt"].as_u64().unwrap() > 1;
        if stage == "test-developer" && retry {
            assert!(!inputs["execution"]["tests"].as_array().unwrap().is_empty());
            assert!(inputs["instruction"].as_str().unwrap().contains("feedback"));
        }
        let output = match stage {
            "code-explorer" => {
                json!({"summary":"scope","scope":["app.py"],"references":[{"id":"ref","path":"app.py","line":1,"claim":"one"}],"uncertainties":[],"environment":{"compose_files":[],"services":[],"runner_service":"runner","runner_workdir":"/workspace","external_containers":[],"readiness_timeout":30},"baseline":[["baseline"]]})
            }
            "bug-hunter" => {
                json!({"summary":"candidate","candidates":[{"id":"bug","title":"wrong value","expected":"two","observed":"one","trigger":"value()","evidence_ids":["ref"],"confidence":"high"}],"excluded_areas":[]})
            }
            "test-designer" => {
                json!({"summary":"plan","plans":[{"candidate_id":"bug","test_id":"test","expectation":"two","oracle":"documented behavior","steps":["call"],"reproduction":["rerun"],"blocked_reason":null}]})
            }
            "test-developer" => {
                json!({"summary":"patch","patch":if self.mode=="rollback" && retry {"--- a/app.py\n+++ b/app.py\n@@ -1,2 +1,2 @@\n def value():\n-    return 1\n+    return 2\n"} else if retry {"--- /dev/null\n+++ b/tests/test_generated.py\n@@ -0,0 +1,2 @@\n+# repaired fixture\n+assert False\n"} else {"--- /dev/null\n+++ b/tests/test_generated.py\n@@ -0,0 +1 @@\n+assert False\n"},"tests":[{"id":"test","candidate_id":"bug","command":["test"],"report":"junit.xml","expected_names":["test"],"reproduction_commands":[]}],"limitations":[]})
            }
            "test-validator" => {
                let evidence = if self.mode == "stale" {
                    directory
                        .parent()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .join("executions/old/execution.json")
                        .display()
                        .to_string()
                } else {
                    format!(
                        "{}/execution.json",
                        inputs["execution"]["tests"][0]["directory"]
                            .as_str()
                            .unwrap()
                    )
                };
                json!({"summary":"review","verdicts":[{"candidate_id":"bug","test_ids":["test"],"status":"confirmed","rationale":"target failed","evidence":[evidence]}]})
            }
            _ => bail!("unexpected stage"),
        };
        fs::create_dir_all(directory)?;
        aaqe::state::write_json(&directory.join("input.json"), inputs)?;
        aaqe::state::write_json(&directory.join("output.json"), &output)?;
        Ok(output)
    }
}

struct Runtime {
    root: PathBuf,
}
impl TestRuntime for Runtime {
    fn start(&self) -> Result<()> {
        Ok(())
    }
    fn logs(&self) -> Result<()> {
        Ok(())
    }
    fn execute_test(&self, command: &[String], id: &str, _: u64) -> Result<Value> {
        let directory = self.root.join(uuid::Uuid::new_v4().simple().to_string());
        fs::create_dir_all(&directory)?;
        let baseline = id.starts_with("baseline");
        if !baseline {
            fs::write(
                directory.join("junit.xml"),
                "<testsuite><testcase name=\"test\"><failure/></testcase></testsuite>",
            )?;
        }
        Ok(
            json!({"test_id":id,"directory":directory,"result":{"argv":command,"returncode":if baseline {0} else {1},"timed_out":false}}),
        )
    }
}

fn audit(mode: &'static str) -> (tempfile::TempDir, Value, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(temp.path());
    let config = Config {
        state_dir: temp.path().join("runs"),
        max_stage_calls: if mode == "budget" { 4 } else { 20 },
        max_stage_attempts: if mode == "attempt-budget" { 1 } else { 4 },
        ..Config::default()
    };
    let state = run_audit_with(
        "audit",
        &repo,
        &config,
        &Agent { mode },
        &|_, directory, _, _, _| {
            Box::new(Runtime {
                root: directory.join("executions"),
            })
        },
        &|_| {},
    )
    .unwrap();
    let directory = config.state_dir.join(state["id"].as_str().unwrap());
    (temp, state, directory)
}

#[test]
fn coordinator_retries_preserve_attempts_and_receive_real_execution_feedback() {
    let (_temp, state, directory) = audit("retry");
    assert_eq!(state["status"], "completed", "{state}");
    assert_eq!(state["stages"].as_array().unwrap().len(), 6);
    let first = read_json(&directory.join("stages/test-developer/output.json")).unwrap();
    let second =
        read_json(&directory.join("stages/test-developer/attempts/0002/output.json")).unwrap();
    assert_ne!(first["patch"], second["patch"]);
    assert!(
        directory
            .join("stages/test-developer/execution.json")
            .is_file()
    );
    assert!(
        directory
            .join("stages/test-developer/attempts/0002/execution.json")
            .is_file()
    );
    assert_eq!(
        read_json(&directory.join("report.json")).unwrap()["verdicts"][0]["status"],
        "confirmed"
    );
}

#[test]
fn upstream_revisit_invalidates_active_downstream_evidence_but_keeps_history() {
    let (_temp, state, directory) = audit("revisit");
    assert_eq!(state["status"], "blocked", "{state}");
    assert!(!directory.join("artifacts/test-validator.json").exists());
    assert!(directory.join("stages/test-validator/output.json").exists());
    assert!(
        directory
            .join("stages/test-developer/execution.json")
            .is_file()
    );
}

#[test]
fn rejected_replacement_restores_previous_patch_and_cannot_modify_production() {
    let (_temp, state, directory) = audit("rollback");
    assert_eq!(state["status"], "blocked", "{state}");
    assert_eq!(
        read_json(&directory.join("stages/test-developer/attempts/0002/result.json")).unwrap()["status"],
        "failed"
    );
}

#[test]
fn stage_budget_and_prerequisites_are_enforced_by_harness() {
    let (_temp, state, _) = audit("budget");
    assert_eq!(state["status"], "blocked", "{state}");
    assert_eq!(state["stages"].as_array().unwrap().len(), 4);
}

#[test]
fn old_execution_paths_cannot_confirm_a_revised_test() {
    let (_temp, state, directory) = audit("stale");
    assert_eq!(state["status"], "completed", "{state}");
    assert_eq!(
        read_json(&directory.join("report.json")).unwrap()["verdicts"][0]["status"],
        "blocked"
    );
}

#[test]
fn coordinator_loss_during_baseline_recovers_the_generated_test_patch() {
    let (_temp, state, _) = audit("crash-baseline");
    assert_eq!(state["status"], "blocked");
    assert!(
        Path::new(state["workspace"]["path"].as_str().unwrap())
            .join("tests/test_generated.py")
            .exists()
    );
    assert!(state["baseline_restore_patch"].is_null());
}

#[test]
fn per_role_budget_prevents_reusing_a_stage_window() {
    let (_temp, state, _) = audit("attempt-budget");
    assert_eq!(state["status"], "blocked");
    assert_eq!(state["stages"].as_array().unwrap().len(), 4);
}
