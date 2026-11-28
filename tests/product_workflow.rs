mod support;

use aaqe::{
    audit::{
        harness::AuditHarness,
        interfaces::{AuditAgent, TestRuntime},
        workflow::run_audit_with,
    },
    config::Config,
    runtime::process::{run, save_result},
    state::{read_json, write_json},
};
use anyhow::Result;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

struct ProductAgent {
    mode: &'static str,
}

fn explorer() -> Value {
    json!({"summary":"测试查询功能","scope":["查询"],"references":[],"uncertainties":[],
      "entry_points":[{"id":"sql","kind":"sql","description":"SQL 查询","applicable":true,"reason":"支持查询"}],
      "services":[{"id":"database","name":"数据库","source":"external","endpoint":"http://db.example:6041","compose_service":null,"version":null,"check":["sh","-c","printf 'database version 1'"]}],
      "environment":{"compose_files":[],"services":["runner"],"runner_service":"runner","runner_workdir":"/workspace","external_containers":[],"readiness_timeout":10,"compose":{"services":{"runner":{"image":"alpine","command":["sleep","infinity"]}}},"readiness":[],"setup":[]}})
}

fn scenario(id: &str) -> Value {
    json!({"id":id,"title":"正常查询","feature":"查询","entry_point":"sql","category":"normal","preconditions":["数据库可访问"],"services":["database"],"oracle":"用户文档要求返回输入数据","steps":[{"id":"query","action":"通过 SQL 查询已准备的数据","expected":"返回 1"}],"cleanup":[],"blocked_reason":null,"derived_from":null,"reason":null})
}

impl AuditAgent for ProductAgent {
    fn product_workflow(&self) -> bool {
        true
    }

    fn coordinate(&self, harness: &mut AuditHarness<'_>) -> Result<()> {
        assert!(harness.finish("completed", "尚未执行").is_err());
        for stage in [
            "product-explorer",
            "scenario-designer",
            "product-executor",
            "issue-reproducer",
            "result-reviewer",
        ] {
            let result = harness.run_stage(stage, "按文档执行真实操作")?;
            if self.mode == "unmentioned" && stage == "product-explorer" {
                assert_eq!(result["status"], "failed");
                return harness.finish("blocked", "拒绝未经指定的外部地址");
            }
            assert_eq!(result["status"], "completed", "{result}");
        }
        if self.mode == "runtime_error" {
            let result = harness.run_stage("product-executor", "重复操作时协议检查抛出异常")?;
            assert_eq!(result["status"], "failed", "{result}");
            assert!(
                harness.status()["execution"]["operations"]
                    .as_array()
                    .unwrap()
                    .is_empty(),
                "失败重跑仍保留旧成功证据"
            );
            return harness.finish("blocked", "重跑异常，当前没有有效操作证据");
        }
        if self.mode == "append" || self.mode == "revise" {
            let original = harness.status()["execution"]["operations"][0]["directory"].clone();
            let result = harness.run_stage("scenario-designer", "补充或者修订场景")?;
            assert_eq!(result["status"], "completed", "{result}");
            let operations = harness.status()["execution"]["operations"].clone();
            if self.mode == "append" {
                assert_eq!(operations[0]["directory"], original);
            } else {
                assert_eq!(operations.as_array().unwrap().len(), 0);
            }
            assert!(harness.finish("completed", "旧复核已失效").is_err());
            return harness.finish("blocked", "新增场景等待执行");
        }
        harness.finish("completed", "所有场景已有独立复核")
    }

    fn invoke(
        &self,
        stage: &str,
        _workspace: &Path,
        _directory: &Path,
        inputs: &Value,
        _: &dyn Fn(&str),
    ) -> Result<Value> {
        Ok(match stage {
            "product-explorer" => explorer(),
            "scenario-designer" => {
                if self.mode == "empty" {
                    return Ok(
                        json!({"summary":"没有形成场景","scenarios":[],"coverage":[],"limitations":[]}),
                    );
                }
                let mut scenarios = vec![scenario("SC-1")];
                if inputs["attempt"].as_u64().unwrap() > 1 {
                    if self.mode == "append" {
                        scenarios.push(scenario("SC-2"));
                    }
                    if self.mode == "revise" {
                        scenarios[0]["steps"][0]["expected"] = json!("返回 2");
                    }
                }
                json!({"summary":"编写操作步骤","scenarios":scenarios,"coverage":[{"feature":"查询","entry_point":"sql","category":"normal","status":"planned","rationale":"正常场景","scenario_ids":["SC-1"]}],"limitations":[]})
            }
            "product-executor" => {
                if self.mode == "empty" {
                    return Ok(
                        json!({"summary":"没有操作","operations":[],"observations":[],"limitations":[]}),
                    );
                }
                assert!(
                    Path::new(inputs["artifact_directory"].as_str().unwrap())
                        .join("steps.md")
                        .is_file(),
                    "操作之前必须保存文档"
                );
                json!({"summary":"执行查询","operations":[{"id":"OP-1","scenario_id":"SC-1","step_id":"query","command":["sh","-c",if self.mode == "cancelled" {"exit 130"} else {"printf '1'"}],"purpose":"action"}],"observations":[],"limitations":[]})
            }
            "issue-reproducer" => {
                json!({"summary":"没有异常","operations":[],"observations":[],"limitations":[],"issues":[],"followups":[]})
            }
            "result-reviewer" => {
                if self.mode == "empty" {
                    return Ok(
                        json!({"summary":"空复核不能证明完成","scenario_results":[],"issue_results":[],"limitations":[]}),
                    );
                }
                let evidence = if self.mode == "fabricated" {
                    json!(["invented/execution.json"])
                } else {
                    json!([Path::new(
                        inputs["execution"]["operations"][0]["directory"]
                            .as_str()
                            .unwrap()
                    )
                    .join("execution.json")])
                };
                json!({"summary":"独立核对结果","scenario_results":[{"scenario_id":"SC-1","status":"passed","rationale":"实际输出为 1","evidence":evidence,"assessment":{"executed":true,"expectation_met":true,"infrastructure_failure":false,"no_mocks":true,"real_services_verified":true,"entry_point_verified":true}}],"issue_results":[],"limitations":[]})
            }
            _ => unreachable!("{stage}"),
        })
    }
}

struct Runtime {
    directory: PathBuf,
    checks: Arc<Mutex<Vec<String>>>,
    calls: Arc<Mutex<usize>>,
    fail_after: Option<usize>,
}

impl TestRuntime for Runtime {
    fn start(&self) -> Result<()> {
        Ok(())
    }
    fn execute_test(&self, argv: &[String], id: &str, timeout: u64) -> Result<Value> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        anyhow::ensure!(
            self.fail_after.is_none_or(|limit| *calls <= limit),
            "真实执行设施异常"
        );
        drop(calls);
        let directory = self
            .directory
            .join(uuid::Uuid::new_v4().simple().to_string());
        fs::create_dir_all(&directory)?;
        let result = run(argv, None, timeout, None);
        save_result(&result, &directory)?;
        let record = json!({"test_id":id,"directory":directory,"result":result});
        write_json(&directory.join("execution.json"), &record)?;
        Ok(record)
    }
    fn execute_service_check(&self, _: &[String], service: &str, _: &str, _: u64) -> Result<Value> {
        self.checks.lock().unwrap().push(service.into());
        anyhow::bail!("外部服务不得要求 Docker 容器身份")
    }
    fn logs(&self) -> Result<()> {
        Ok(())
    }
}

fn exercise(mode: &'static str) -> (tempfile::TempDir, Value, Value, Vec<String>) {
    let temp = tempfile::tempdir().unwrap();
    let root = support::repository(temp.path(), true);
    let config = Config {
        state_dir: temp.path().join("runs"),
        ..Config::default()
    };
    let checks = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(Mutex::new(0));
    let prompt = if mode == "unmentioned" {
        "检查 SQL 查询"
    } else {
        "检查 SQL 查询，数据库地址 http://db.example:6041"
    };
    let state = run_audit_with(
        prompt,
        &root,
        &config,
        &ProductAgent { mode },
        &|_, directory, _, _, _| {
            Box::new(Runtime {
                directory: directory.join("operations"),
                checks: checks.clone(),
                calls: calls.clone(),
                fail_after: (mode == "runtime_error").then_some(2),
            })
        },
        &|_| {},
    )
    .unwrap();
    let directory = config.state_dir.join(state["id"].as_str().unwrap());
    let snapshot = read_json(&directory.join("harness.json")).unwrap();
    let recorded = checks.lock().unwrap().clone();
    (temp, state, snapshot, recorded)
}

#[test]
fn documented_product_operations_complete_without_patch_or_code_coverage() {
    let (_temp, state, snapshot, checks) = exercise("normal");
    assert_eq!(state["workflow"], "product");
    assert_eq!(state["status"], "completed", "{state}");
    assert_eq!(
        snapshot["execution"]["operations"][0]["result"]["stdout"],
        "1"
    );
    assert!(checks.is_empty());
    assert!(state["publication"].is_null());
}

#[test]
fn external_services_must_be_explicit_in_the_run_prompt() {
    let (_temp, state, _, checks) = exercise("unmentioned");
    assert_eq!(state["status"], "blocked");
    assert!(checks.is_empty());
}

#[test]
fn fabricated_operation_evidence_cannot_pass_independent_review() {
    let (_temp, state, snapshot, _) = exercise("fabricated");
    assert_eq!(state["status"], "blocked");
    assert_eq!(
        snapshot["artifacts"]["result-reviewer"]["scenario_results"][0]["status"],
        "blocked"
    );
}

#[test]
fn appending_a_scenario_preserves_unchanged_execution_but_invalidates_review() {
    exercise("append");
}

#[test]
fn revising_a_scenario_invalidates_its_old_execution() {
    exercise("revise");
}

#[test]
fn empty_product_scenarios_cannot_complete_without_verifying_anything() {
    let (_, state, _, _) = exercise("empty");
    assert_eq!(state["status"], "blocked", "{state}");
}

#[test]
fn interrupted_operation_retry_invalidates_the_previous_success() {
    let (_, state, _, _) = exercise("runtime_error");
    assert_eq!(state["status"], "blocked");
}

#[test]
fn cancelled_product_operation_cannot_supply_complete_evidence() {
    let (_, state, snapshot, _) = exercise("cancelled");
    assert_eq!(
        snapshot["execution"]["operations"][0]["evidence"]["complete"],
        false
    );
    assert_eq!(state["status"], "blocked");
}

#[test]
fn saved_product_replay_rejects_unknown_scenarios_before_docker() {
    let (temp, mut state, _, _) = exercise("normal");
    let store = aaqe::state::Store::new(temp.path().join("runs"));
    let error =
        aaqe::audit::workflow::reproduce(&store, &mut state, Some("missing-scenario")).unwrap_err();
    assert!(
        error.to_string().contains("Unknown scenario ID"),
        "{error:#}"
    );
    assert!(state["reproductions"].is_null());
}
