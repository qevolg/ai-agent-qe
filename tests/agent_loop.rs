mod support;
use aaqe::{
    audit::harness::AuditHarness,
    audit::{
        interfaces::{AuditAgent, TestRuntime},
        workflow::run_audit_with,
    },
    config::Config,
    state::read_json,
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn repository(directory: &Path) -> PathBuf {
    support::repository(directory, true)
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
                    fs::read_to_string(workspace.join("tests/generated.rs"))?,
                    "#[test] fn regression() { assert!(false); }\n"
                );
                assert_eq!(
                    fs::read_to_string(workspace.join("app.rs"))?,
                    "pub fn value() -> i32 { 1 }\n"
                );
                return harness.finish("blocked", "replacement rejected; prior patch retained");
            }
            assert_eq!(result["status"], "completed");
            assert_ne!(old, harness.status()["execution"]["tests"][0]["directory"]);
        }
        if self.mode == "crash-baseline" || self.mode == "policy-recovery-failure" {
            let status = harness.status();
            let root = PathBuf::from(status["workspace"]["path"].as_str().unwrap());
            let directory = PathBuf::from(status["state"]["config"]["state_dir"].as_str().unwrap())
                .join(status["state"]["id"].as_str().unwrap());
            let patch = fs::read_to_string(directory.join("tests.diff"))?;
            let result = aaqe::runtime::process::run(
                &["git".into(), "apply".into(), "--reverse".into(), "-".into()],
                Some(&root),
                10,
                Some(&patch),
            );
            assert_eq!(result.returncode, 0);
            let mut saved = read_json(&directory.join("harness.json"))?;
            saved["state"]["baseline_restore_patch"] = json!(patch);
            aaqe::state::write_json(&directory.join("harness.json"), &saved)?;
            if self.mode == "policy-recovery-failure" {
                fs::create_dir_all(root.join("tests"))?;
                fs::write(root.join("tests/generated.rs"), "obstruction\n")?;
                aaqe::state::write_json(
                    &directory.join("policy-denial.json"),
                    &json!({"code":"policy_denied","message":"安全策略拒绝：original provider cause"}),
                )?;
            }
            bail!("MCP lost while the baseline patch was reversed");
        }
        assert_eq!(
            harness.run_stage("test-validator", "independently review")?["status"],
            "completed"
        );
        if self.mode.starts_with("loss-") {
            // Fault injection: the committed snapshot still describes a confirmed
            // first patch, while a killed MCP left a filesystem transition behind.
            let status = harness.status();
            let root = PathBuf::from(status["workspace"]["path"].as_str().unwrap());
            let directory = harness.directory.clone();
            let previous = status["state"]["applied_patch"].as_str().unwrap();
            let replacement = "--- /dev/null\n+++ b/tests/generated.rs\n@@ -0,0 +1,2 @@\n+// replacement\n+#[test] fn regression() { assert!(false); }\n";
            aaqe::state::write_json(
                &directory.join("patch-transition.json"),
                &json!({"previous":previous,"replacement":if self.mode=="loss-replacement" {json!(replacement)}else{Value::Null}}),
            )?;
            if self.mode != "loss-recovered" {
                let reversed = aaqe::runtime::process::run(
                    &["git".into(), "apply".into(), "--reverse".into(), "-".into()],
                    Some(&root),
                    10,
                    Some(previous),
                );
                assert_eq!(reversed.returncode, 0);
            }
            if self.mode == "loss-replacement" {
                aaqe::guards::patch::PatchGate::new(
                    &root,
                    &harness.config.test_paths,
                    status["workspace"]["commit"].as_str(),
                )?
                .apply(replacement)?;
                let mut artifact = status["artifacts"]["test-developer"].clone();
                artifact["patch"] = json!(replacement);
                aaqe::state::write_json(
                    &directory.join("artifacts/test-developer.json"),
                    &artifact,
                )?;
                fs::write(directory.join("tests.diff"), replacement)?;
            } else {
                fs::remove_file(directory.join("artifacts/bug-hunter.json"))?;
                fs::remove_file(directory.join("artifacts/test-developer.json"))?;
                fs::remove_file(directory.join("tests.diff"))?;
            }
            bail!("simulated MCP loss during patch transition");
        }
        if self.mode == "generated-reference" || self.mode == "line-reference" {
            let result =
                harness.run_stage("code-explorer", "review sources at the recorded base")?;
            assert_eq!(result["status"], "failed", "{result}");
            assert_eq!(
                harness.status()["artifacts"]["code-explorer"]["references"][0]["path"],
                "app.rs"
            );
        }
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
        assert!(
            inputs["previous_attempts"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["status"] != "running")
        );
        if stage == "code-explorer" {
            assert!(
                inputs["artifacts"].as_object().unwrap().is_empty(),
                "Explorer must receive scope feedback without unrelated complete artifacts"
            );
        }
        if stage == "bug-hunter" {
            assert!(inputs["artifacts"].get("code-explorer").is_some());
            assert!(inputs["artifacts"].get("test-developer").is_none());
        }
        if stage == "test-developer" && retry {
            assert!(!inputs["execution"]["tests"].as_array().unwrap().is_empty());
            assert!(inputs["instruction"].as_str().unwrap().contains("feedback"));
        }
        let mut output = match stage {
            "code-explorer" => {
                json!({"summary":"scope","scope":["app.rs"],"references":[{"id":"ref","path":"app.rs","line":1,"claim":"one"}],"uncertainties":[],"environment":{"compose_files":[],"services":[],"runner_service":"runner","runner_workdir":"/workspace","external_containers":[],"readiness_timeout":30},"baseline":[["baseline"]]})
            }
            "bug-hunter" => {
                json!({"summary":"candidate","candidates":[{"id":"bug","title":"wrong value","expected":"two","observed":"one","trigger":"value()","evidence_ids":["ref"],"confidence":"high"}],"excluded_areas":[]})
            }
            "test-designer" => {
                json!({"summary":"plan","plans":[{"candidate_id":"bug","test_id":"test","expectation":"two","oracle":"documented behavior","steps":["call"],"reproduction":["rerun"],"blocked_reason":null}]})
            }
            "test-developer" => {
                json!({"summary":"patch","coverage":{"command":["coverage"],"report":"coverage.json","files":["app.rs"]},"patch":if self.mode=="rollback" && retry {"--- a/app.rs\n+++ b/app.rs\n@@ -1 +1 @@\n-pub fn value() -> i32 { 1 }\n+pub fn value() -> i32 { 2 }\n"} else if retry {"--- /dev/null\n+++ b/tests/generated.rs\n@@ -0,0 +1,2 @@\n+// repaired regression\n+#[test] fn regression() { assert!(false); }\n"} else {"--- /dev/null\n+++ b/tests/generated.rs\n@@ -0,0 +1 @@\n+#[test] fn regression() { assert!(false); }\n"},"tests":[{"id":"test","candidate_id":"bug","command":["test"],"report":"junit.xml","expected_names":["test"],"reproduction_commands":[]}],"limitations":[]})
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
                json!({"summary":"review","coverage_assessment":{"scope_valid":true,"measurement_valid":true,"meaningful_assertions":true,"rationale":"Reviewed native evidence and complete production scope"},"verdicts":[{"candidate_id":"bug","test_ids":["test"],"status":"confirmed","assessment":{"executed":true,"assertion_failed":true,"infrastructure_failure":false},"rationale":"target failed","evidence":[evidence]}]})
            }
            _ => bail!("unexpected stage"),
        };
        if self.mode == "line-reference" && stage == "test-developer" {
            output["patch"] = json!(
                "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1,4 @@\n pub fn value() -> i32 { 1 }\n+#[cfg(test)]\n+mod tests {\n+}\n"
            );
        }
        if retry && stage == "code-explorer" {
            if self.mode == "generated-reference" {
                output["references"][0]["path"] = json!("tests/generated.rs");
            }
            if self.mode == "line-reference" {
                output["references"][0]["path"] = json!("src/lib.rs");
                output["references"][0]["line"] = json!(3);
            }
        }
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
        if id.starts_with("coverage-") {
            fs::write(directory.join("raw.txt"), "measurement")?;
            fs::write(directory.join("coverage.json"), json!({"files":[{"path":"app.rs","covered_lines":1,"executable_lines":1}],"raw_reports":["raw.txt"]}).to_string())?;
            return Ok(
                json!({"test_id":id,"directory":directory,"result":{"returncode":0,"timed_out":false}}),
            );
        }
        let baseline = id.starts_with("baseline");
        if !baseline {
            fs::write(
                directory.join("junit.xml"),
                "<testsuite><testcase name=\"test\"><failure/></testcase></testsuite>",
            )?;
        }
        Ok(
            json!({"test_id":id,"directory":directory,"result":{"argv":command,"returncode":if baseline && command[0]!="bad-baseline" {0} else {1},"timed_out":false}}),
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
            .join("tests/generated.rs")
            .exists()
    );
    assert!(state["baseline_restore_patch"].is_null());
}

#[test]
fn policy_cause_remains_primary_when_patch_recovery_also_fails() {
    let (_temp, state, directory) = audit("policy-recovery-failure");
    assert_eq!(state["status"], "blocked");
    assert!(state["recovery_error"].is_string(), "{state}");
    assert_eq!(state["error"], "安全策略拒绝：original provider cause");
    let report = fs::read_to_string(directory.join("report.md")).unwrap();
    assert!(report.contains("安全策略拒绝：original provider cause"));
}

#[test]
fn per_role_budget_prevents_reusing_a_stage_window() {
    let (_temp, state, _) = audit("attempt-budget");
    assert_eq!(state["status"], "blocked");
    assert_eq!(state["stages"].as_array().unwrap().len(), 4);
}

fn assert_transition_recovery(mode: &'static str) {
    let (_temp, state, directory) = audit(mode);
    assert_eq!(state["status"], "blocked");
    let workspace = Path::new(state["workspace"]["path"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(workspace.join("tests/generated.rs")).unwrap(),
        "#[test] fn regression() { assert!(false); }\n"
    );
    let artifact = read_json(&directory.join("artifacts/test-developer.json")).unwrap();
    assert_eq!(artifact["patch"], state["applied_patch"]);
    assert_eq!(
        fs::read_to_string(directory.join("tests.diff")).unwrap(),
        state["applied_patch"].as_str().unwrap()
    );
    assert!(directory.join("artifacts/bug-hunter.json").is_file());
    assert_eq!(
        read_json(&directory.join("patch-check.json")).unwrap()["accepted"],
        true
    );
    assert_eq!(
        read_json(&directory.join("report.json")).unwrap()["verdicts"][0]["status"],
        "confirmed"
    );
    assert!(!directory.join("patch-transition.json").exists());
}

#[test]
fn loss_after_patch_reversal_recovers_committed_evidence() {
    assert_transition_recovery("loss-upstream");
}

#[test]
fn loss_after_replacement_application_recovers_committed_evidence() {
    assert_transition_recovery("loss-replacement");
}

#[test]
fn revised_explorer_cannot_accept_generated_files() {
    assert_eq!(audit("generated-reference").1["status"], "completed");
}

#[test]
fn revised_explorer_cannot_accept_lines_outside_the_base() {
    assert_eq!(audit("line-reference").1["status"], "completed");
}

#[test]
fn interrupted_recovery_reconciles_aliases_without_reversing_restored_patch() {
    assert_transition_recovery("loss-recovered");
}

struct PreparationAgent {
    mode: &'static str,
}
impl AuditAgent for PreparationAgent {
    fn invoke(
        &self,
        stage: &str,
        workspace: &Path,
        directory: &Path,
        inputs: &Value,
        progress: &dyn Fn(&str),
    ) -> Result<Value> {
        if stage == "test-validator" && self.mode == "blocked" {
            return Ok(
                json!({"summary":"blocked","verdicts":[{"candidate_id":"bug","test_ids":["test"],"status":"confirmed","rationale":"no execution","evidence":[]}]}),
            );
        }
        if stage == "test-developer" && self.mode == "prepare" {
            assert_eq!(inputs["environment_ready"], true);
            assert_eq!(inputs["environment"]["readiness"], json!([["ready"]]));
            assert!(!inputs["environment_actions"].as_array().unwrap().is_empty());
        }
        let mut out =
            Agent { mode: "normal" }.invoke(stage, workspace, directory, inputs, progress)?;
        if stage == "test-designer" {
            out["plans"][0]["execution_blocked_reason"] =
                json!("Host Maven unavailable; runner can provide it");
            if self.mode == "blocked" {
                out["plans"][0]["blocked_reason"] = json!("No reliable oracle");
            }
        }
        Ok(out)
    }

    fn coordinate(&self, h: &mut AuditHarness<'_>) -> Result<()> {
        h.run_stage("code-explorer", "inspect")?;
        if self.mode == "prepare" {
            assert_eq!(
                h.prepare_environment(Some(json!({"readiness":[["ready"]]})))?["status"],
                "completed"
            );
            assert_eq!(h.status()["state"]["environment_ready"], true);
            assert_eq!(h.prepare_environment(None)?["status"], "completed");
            assert_eq!(
                h.prepare_environment(Some(json!({"runner_service":"other"})))?["status"],
                "failed"
            );
            assert_eq!(
                h.inspect_environment(&json!({"kind":"tools","target":"host","tools":["git"]}))?["status"],
                "completed"
            );
        }
        h.run_stage("bug-hunter", "hunt")?;
        let result = h.run_stage("test-designer", "design")?;
        assert_eq!(result["status"], "completed", "{result}");
        if self.mode == "blocked" {
            assert!(
                h.status()["allowed_stages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|s| s == "test-validator")
            );
            assert!(
                h.run_stage("test-developer", "override blocked plan")
                    .is_err()
            );
            assert_eq!(
                h.status()["state"]["dispatch_rejections"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(
                h.run_stage("test-validator", "independently review blocked candidates")?["status"],
                "completed"
            );
            assert_eq!(h.status()["verdicts"][0]["status"], "blocked");
            return h.finish("completed", "reviewed blocked plan");
        }
        if self.mode == "prepare" {
            assert_eq!(
                h.run_stage("test-developer", "implement")?["status"],
                "completed"
            );
            h.run_stage("test-validator", "validate first round")?;
            assert_eq!(h.status()["verdicts"][0]["status"], "confirmed");
            let old = h.status()["execution"]["tests"][0]["directory"].clone();
            let current = h.status();
            assert_eq!(
                h.prepare_environment_with_setup(None, Some(vec![]))?["status"],
                "failed"
            );
            assert_eq!(h.status()["execution"], current["execution"]);
            assert_eq!(h.status()["verdicts"], current["verdicts"]);
            assert_eq!(
                h.prepare_environment_with_setup(
                    None,
                    Some(vec![vec!["install-missing-dependency".into()]])
                )?["status"],
                "completed"
            );
            assert_eq!(h.status()["execution"]["tests"], json!([]));
            assert_eq!(h.status()["verdicts"], json!([]));
            assert!(h.status()["artifacts"]["test-validator"].is_null());
            assert_eq!(
                h.status()["state"]["environment_setup"],
                json!([["install-missing-dependency"]])
            );
            assert_eq!(
                h.execute_tests(Some(vec!["test".into()]))?["status"],
                "completed"
            );
            assert_ne!(old, h.status()["execution"]["tests"][0]["directory"]);
            assert_eq!(
                h.status()["state"]["execution_attempts"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert_eq!(h.status()["verdicts"], json!([]));
            assert!(h.status()["artifacts"].get("test-validator").is_none());
            assert_eq!(
                h.execute_tests(Some(vec!["unknown".into()]))?["status"],
                "failed"
            );
        }
        h.finish("blocked", "execution prerequisite does not block design")
    }
}

#[test]
fn execution_blocker_is_optional_and_does_not_block_design() {
    let temp = tempfile::tempdir().unwrap();
    let root = repository(temp.path());
    let config = Config {
        state_dir: temp.path().join("runs"),
        ..Config::default()
    };
    let state = run_audit_with(
        "audit",
        &root,
        &config,
        &PreparationAgent { mode: "execution" },
        &|_, d, _, _, _| {
            Box::new(Runtime {
                root: d.join("executions"),
            })
        },
        &|_| {},
    )
    .unwrap();
    assert_eq!(state["stages"][2]["status"], "completed");
}

#[test]
fn blocked_plans_offer_validator_and_record_dispatch_rejection() {
    let temp = tempfile::tempdir().unwrap();
    let root = repository(temp.path());
    let config = Config {
        state_dir: temp.path().join("runs"),
        ..Config::default()
    };
    run_audit_with(
        "audit",
        &root,
        &config,
        &PreparationAgent { mode: "blocked" },
        &|_, d, _, _, _| {
            Box::new(Runtime {
                root: d.join("executions"),
            })
        },
        &|_| {},
    )
    .unwrap();
}

#[test]
fn verdict_requires_every_test_and_only_its_applicable_baseline() {
    let artifact = json!({"verdicts":[{"candidate_id":"bug","test_ids":["a","b"],"status":"confirmed","rationale":"both","evidence":["/a/execution.json"]}]});
    let mut execution = json!({"baseline":[{"for_test_id":"a","result":{"returncode":0}}],"tests":[{"test_id":"a","directory":"/a","junit":{"complete":true,"cases":[{"status":"failed"}]}}]});
    assert_eq!(
        aaqe::audit::workflow::grounded_verdicts(&artifact, &execution)[0]["status"],
        "blocked"
    );
    execution["baseline"]
        .as_array_mut()
        .unwrap()
        .push(json!({"for_test_id":"b","result":{"returncode":1}}));
    let single = json!({"verdicts":[{"candidate_id":"bug","test_ids":["a"],"status":"confirmed","rationale":"a","evidence":["/a/execution.json"]}]});
    assert_eq!(
        aaqe::audit::workflow::grounded_verdicts(&single, &execution)[0]["status"],
        "confirmed"
    );
}

#[test]
fn prepared_environment_is_reused_and_selected_tests_rerun_without_developer() {
    let temp = tempfile::tempdir().unwrap();
    let root = repository(temp.path());
    let config = Config {
        state_dir: temp.path().join("runs"),
        ..Config::default()
    };
    let state = run_audit_with(
        "audit",
        &root,
        &config,
        &PreparationAgent { mode: "prepare" },
        &|_, d, _, _, _| {
            Box::new(Runtime {
                root: d.join("executions"),
            })
        },
        &|_| {},
    )
    .unwrap();
    assert_eq!(state["stages"].as_array().unwrap().len(), 5);
}

struct IndependentAgent;
impl AuditAgent for IndependentAgent {
    fn invoke(
        &self,
        stage: &str,
        w: &Path,
        d: &Path,
        i: &Value,
        p: &dyn Fn(&str),
    ) -> Result<Value> {
        let mut out = Agent { mode: "normal" }.invoke(stage, w, d, i, p)?;
        if stage == "test-designer" {
            let mut second = out["plans"][0].clone();
            second["test_id"] = json!("independent");
            out["plans"].as_array_mut().unwrap().push(second);
        }
        if stage == "test-developer" {
            out["tests"][0]["baseline"] = json!([["bad-baseline"]]);
            let mut second = out["tests"][0].clone();
            second["id"] = json!("independent");
            second["baseline"] = json!([["good-baseline"]]);
            out["tests"].as_array_mut().unwrap().push(second);
        }
        Ok(out)
    }

    fn coordinate(&self, h: &mut AuditHarness<'_>) -> Result<()> {
        for stage in ["code-explorer", "bug-hunter", "test-designer"] {
            assert_eq!(h.run_stage(stage, "inspect")?["status"], "completed");
        }
        let result = h.run_stage("test-developer", "implement independent experiments")?;
        assert_eq!(result["status"], "failed");
        assert!(
            result["recovery"]["permitted_next_actions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a == "execute_tests")
        );
        let status = h.status();
        assert!(status["artifacts"].get("test-developer").is_some());
        assert_eq!(status["execution"]["tests"].as_array().unwrap().len(), 1);
        assert_eq!(status["execution"]["tests"][0]["test_id"], "independent");
        assert_eq!(status["execution"]["baseline"][0]["for_test_id"], "test");
        assert_eq!(
            status["execution"]["baseline"][1]["for_test_id"],
            "independent"
        );
        assert_ne!(
            status["execution"]["baseline"][0]["test_id"],
            status["execution"]["baseline"][1]["test_id"]
        );
        assert!(
            Path::new(status["workspace"]["path"].as_str().unwrap())
                .join("tests/generated.rs")
                .is_file()
        );
        assert_eq!(
            h.execute_tests(Some(vec!["independent".into()]))?["status"],
            "completed"
        );
        h.finish(
            "blocked",
            "independent experiment completed; other baseline unavailable",
        )
    }
}

#[test]
fn failed_scoped_baseline_preserves_patch_and_executes_next_independent_test() {
    let temp = tempfile::tempdir().unwrap();
    let root = repository(temp.path());
    let config = Config {
        state_dir: temp.path().join("runs"),
        ..Config::default()
    };
    run_audit_with(
        "audit",
        &root,
        &config,
        &IndependentAgent,
        &|_, d, _, _, _| {
            Box::new(Runtime {
                root: d.join("executions"),
            })
        },
        &|_| {},
    )
    .unwrap();
}

#[test]
fn environment_operations_bound_runtime_timeouts_to_remaining_audit_budget() {
    struct BudgetRuntime(Runtime);
    impl TestRuntime for BudgetRuntime {
        fn start(&self) -> Result<()> {
            self.0.start()
        }

        fn logs(&self) -> Result<()> {
            self.0.logs()
        }

        fn execute_test(&self, c: &[String], id: &str, timeout: u64) -> Result<Value> {
            assert!(
                timeout <= 10,
                "execution timeout exceeds audit budget: {timeout}"
            );
            self.0.execute_test(c, id, timeout)
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let root = repository(temp.path());
    let config = Config {
        state_dir: temp.path().join("runs"),
        run_timeout: 10,
        ..Config::default()
    };
    run_audit_with(
        "audit",
        &root,
        &config,
        &PreparationAgent { mode: "prepare" },
        &|_, d, _, _, timeout| {
            assert!(
                timeout <= 10,
                "runtime timeout exceeds audit budget: {timeout}"
            );
            Box::new(BudgetRuntime(Runtime {
                root: d.join("executions"),
            }))
        },
        &|_| {},
    )
    .unwrap();
}

#[test]
fn readiness_failure_locks_owned_spec_and_retries_same_runner() {
    use std::{cell::Cell, rc::Rc};
    struct RetryAgent;
    impl AuditAgent for RetryAgent {
        fn invoke(
            &self,
            _: &str,
            _: &Path,
            _: &Path,
            _: &Value,
            _: &dyn Fn(&str),
        ) -> Result<Value> {
            bail!("no worker expected")
        }

        fn coordinate(&self, h: &mut AuditHarness<'_>) -> Result<()> {
            let spec = json!({"runner_service":"runner","compose":{"services":{"runner":{"image":"fixture"}}},"readiness":[["ready"]]});
            assert_eq!(h.prepare_environment(None)?["status"], "failed");
            assert_eq!(
                h.prepare_environment(Some(spec.clone()))?["status"],
                "failed"
            );
            assert_ne!(h.status()["state"]["environment_ready"], true);
            assert_eq!(
                h.prepare_environment(Some(json!({"runner_service":"changed"})))?["status"],
                "failed"
            );
            assert_eq!(h.prepare_environment(Some(spec))?["status"], "completed");
            assert_eq!(h.prepare_environment(None)?["status"], "completed");
            assert_eq!(
                h.status()["state"]["environment_actions"]
                    .as_array()
                    .unwrap()
                    .len(),
                5
            );
            h.finish("blocked", "retry behavior verified")
        }
    }
    struct RetryRuntime {
        starts: Rc<Cell<u32>>,
        directory: PathBuf,
    }
    impl TestRuntime for RetryRuntime {
        fn start(&self) -> Result<()> {
            fs::create_dir_all(self.directory.join("environment"))?;
            fs::write(self.directory.join("environment/ownership.json"), "{}")?;
            let count = self.starts.get() + 1;
            self.starts.set(count);
            if count == 1 {
                bail!("runner readiness failed");
            }
            Ok(())
        }

        fn logs(&self) -> Result<()> {
            Ok(())
        }

        fn execute_test(&self, _: &[String], _: &str, _: u64) -> Result<Value> {
            bail!("no test expected")
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let root = repository(temp.path());
    let starts = Rc::new(Cell::new(0));
    let config = Config {
        state_dir: temp.path().join("runs"),
        ..Config::default()
    };
    run_audit_with(
        "audit",
        &root,
        &config,
        &RetryAgent,
        &|_, d, _, _, _| {
            Box::new(RetryRuntime {
                starts: starts.clone(),
                directory: d.into(),
            })
        },
        &|_| {},
    )
    .unwrap();
    assert_eq!(
        starts.get(),
        2,
        "changed specs and already-ready retries must not restart runtime"
    );
}

#[test]
fn saved_setup_retry_invalidates_validator_on_success_and_failure() {
    use std::{cell::Cell, rc::Rc};
    struct RetryAgent;
    impl AuditAgent for RetryAgent {
        fn invoke(
            &self,
            stage: &str,
            w: &Path,
            d: &Path,
            i: &Value,
            p: &dyn Fn(&str),
        ) -> Result<Value> {
            if stage == "test-validator" {
                return Ok(
                    json!({"summary":"blocked","verdicts":[{"candidate_id":"bug","test_ids":["test"],"status":"blocked","rationale":"installation failed","evidence":[]}]}),
                );
            }
            Agent { mode: "normal" }.invoke(stage, w, d, i, p)
        }

        fn coordinate(&self, h: &mut AuditHarness<'_>) -> Result<()> {
            for stage in [
                "code-explorer",
                "bug-hunter",
                "test-designer",
                "test-developer",
            ] {
                h.run_stage(stage, "inspect")?;
            }
            for expected in ["failed", "completed"] {
                assert_eq!(
                    h.run_stage("test-validator", "review blocked evidence")?["status"],
                    "completed"
                );
                assert!(!h.status()["artifacts"]["test-validator"].is_null());
                assert_eq!(h.prepare_environment(None)?["status"], expected);
                assert!(h.status()["artifacts"]["test-validator"].is_null());
                assert_eq!(h.status()["verdicts"], json!([]));
            }
            h.finish("blocked", "recovery invalidation verified")
        }
    }
    struct RetryRuntime {
        starts: Rc<Cell<usize>>,
        runtime: Runtime,
    }
    impl TestRuntime for RetryRuntime {
        fn start(&self) -> Result<()> {
            let starts = self.starts.get() + 1;
            self.starts.set(starts);
            if starts <= 2 {
                bail!("dependency installation failed");
            }
            Ok(())
        }

        fn logs(&self) -> Result<()> {
            Ok(())
        }

        fn execute_test(&self, c: &[String], id: &str, t: u64) -> Result<Value> {
            self.runtime.execute_test(c, id, t)
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let root = repository(temp.path());
    let starts = Rc::new(Cell::new(0));
    let config = Config {
        state_dir: temp.path().join("runs"),
        environment: json!({"setup":[["install"]]}),
        ..Config::default()
    };
    run_audit_with(
        "audit",
        &root,
        &config,
        &RetryAgent,
        &|_, d, _, _, _| {
            Box::new(RetryRuntime {
                starts: starts.clone(),
                runtime: Runtime {
                    root: d.join("executions"),
                },
            })
        },
        &|_| {},
    )
    .unwrap();
}

#[test]
fn trusted_global_baseline_overrides_test_specific_prerequisites() {
    struct TrustedAgent;
    impl AuditAgent for TrustedAgent {
        fn invoke(
            &self,
            s: &str,
            w: &Path,
            d: &Path,
            i: &Value,
            p: &dyn Fn(&str),
        ) -> Result<Value> {
            IndependentAgent.invoke(s, w, d, i, p)
        }

        fn coordinate(&self, h: &mut AuditHarness<'_>) -> Result<()> {
            for stage in [
                "code-explorer",
                "bug-hunter",
                "test-designer",
                "test-developer",
            ] {
                assert_eq!(
                    h.run_stage(stage, "trusted baseline")?["status"],
                    "completed"
                );
            }
            let status = h.status();
            assert_eq!(status["execution"]["tests"].as_array().unwrap().len(), 2);
            assert_eq!(status["execution"]["baseline"].as_array().unwrap().len(), 1);
            assert!(status["execution"]["baseline"][0]["for_test_id"].is_null());
            h.finish("blocked", "trusted baseline precedence verified")
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let root = repository(temp.path());
    let config = Config {
        state_dir: temp.path().join("runs"),
        baseline: vec![vec!["baseline".into()]],
        ..Config::default()
    };
    run_audit_with(
        "audit",
        &root,
        &config,
        &TrustedAgent,
        &|_, d, _, _, _| {
            Box::new(Runtime {
                root: d.join("executions"),
            })
        },
        &|_| {},
    )
    .unwrap();
}

#[test]
fn baseline_patch_restore_failure_stops_before_empty_baseline_target_and_recovers() {
    struct RestoreAgent;
    impl AuditAgent for RestoreAgent {
        fn invoke(
            &self,
            stage: &str,
            w: &Path,
            d: &Path,
            i: &Value,
            p: &dyn Fn(&str),
        ) -> Result<Value> {
            let mut output = IndependentAgent.invoke(stage, w, d, i, p)?;
            if stage == "test-developer" {
                output["tests"][1]["baseline"] = json!([]);
            }
            Ok(output)
        }

        fn coordinate(&self, h: &mut AuditHarness<'_>) -> Result<()> {
            for stage in ["code-explorer", "bug-hunter", "test-designer"] {
                assert_eq!(h.run_stage(stage, "inspect")?["status"], "completed");
            }
            let result = h.run_stage("test-developer", "exercise restoration failure")?;
            assert_eq!(result["status"], "failed");
            let status = h.status();
            assert_eq!(
                status["execution"]["tests"],
                json!([]),
                "no target may execute while the accepted patch requires restoration"
            );
            assert!(status["state"]["baseline_restore_patch"].is_string());
            assert_eq!(status["execution"]["baseline"].as_array().unwrap().len(), 1);
            let workspace = PathBuf::from(status["workspace"]["path"].as_str().unwrap());
            fs::remove_file(workspace.join("tests/generated.rs"))?;
            h.finish(
                "blocked",
                "remove injected obstruction and recover accepted patch",
            )
        }
    }
    struct RestoreRuntime {
        root: PathBuf,
        execution: Runtime,
    }
    impl TestRuntime for RestoreRuntime {
        fn start(&self) -> Result<()> {
            Ok(())
        }

        fn logs(&self) -> Result<()> {
            Ok(())
        }

        fn execute_test(&self, command: &[String], id: &str, timeout: u64) -> Result<Value> {
            if id.starts_with("baseline-") {
                // The generated file was removed for baseline execution. Create a
                // conflicting allowed test file so protected reapplication fails.
                fs::create_dir_all(self.root.join("tests"))?;
                fs::write(
                    self.root.join("tests/generated.rs"),
                    "# restoration obstruction\n",
                )?;
            }
            self.execution.execute_test(command, id, timeout)
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let root = repository(temp.path());
    let config = Config {
        state_dir: temp.path().join("runs"),
        ..Config::default()
    };
    let state = run_audit_with(
        "audit",
        &root,
        &config,
        &RestoreAgent,
        &|w, d, _, _, _| {
            Box::new(RestoreRuntime {
                root: w.into(),
                execution: Runtime {
                    root: d.join("executions"),
                },
            })
        },
        &|_| {},
    )
    .unwrap();
    assert_eq!(state["status"], "blocked");
    assert!(state["baseline_restore_patch"].is_null());
    assert_eq!(
        fs::read_to_string(
            Path::new(state["workspace"]["path"].as_str().unwrap()).join("tests/generated.rs")
        )
        .unwrap(),
        "#[test] fn regression() { assert!(false); }\n"
    );
}
