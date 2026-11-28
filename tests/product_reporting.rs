use aaqe::{
    reporting::{generate_report, refresh_report},
    state::{read_json, write_json},
};
use serde_json::{Value, json};
use std::fs;

fn state() -> Value {
    json!({"id":"product-report","workflow":"product","status":"completed","prompt":"验证查询产品",
        "workspace":{"commit":"source-commit-123","branch":"audit/product","path":"/workspace/product"},
        "stages":[],"environment":{"runner_service":"runner"},"environment_started":true,"environment_ready":true,
        "execution_attempts":[{"directory":"executions/old-round","operation_ids":["old-operation"]}],
        "reproductions":[{"at":"2026-11-28T10:00:00+08:00","records":[]}],"dispatch_rejections":[],"environment_actions":[],
        "document_revision":2,"document_changes":[{"revision":2,"at":"now","instruction":"add boundary scenario"}]})
}

fn artifacts() -> Value {
    json!({
        "product-explorer":{"summary":"查询功能","scope":["查询"],"uncertainties":["未验证大集群"],
            "references":[],"entry_points":[{"id":"rest","kind":"api","description":"REST 查询","applicable":true,"reason":"查询入口"}],
            "services":[{"id":"remote","name":"TDengine","source":"external","endpoint":"https://db.example:6041","compose_service":null,"version":"3.3.6.0","check":["curl","https://db.example:6041/rest/sql"]},
                {"id":"sink","name":"InfluxDB","source":"docker","endpoint":"http://influx:8086","compose_service":"influx","version":null,"check":["curl","http://influx:8086/ping"]}]},
        "scenario-designer":{"summary":"设计查询场景","scenarios":[
            {"id":"normal","title":"正常 | 查询\n返回","feature":"查询","entry_point":"api","category":"normal","preconditions":[],"services":["remote"],"oracle":"返回预期行","steps":[{"id":"query","action":"查询","expected":"一行"}],"cleanup":[],"blocked_reason":null,"derived_from":null,"reason":null},
            {"id":"abnormal","title":"拒绝非法语句","feature":"查询","entry_point":"sql","category":"abnormal","preconditions":[],"services":["remote"],"oracle":"明确语法错误","steps":[{"id":"query","action":"非法查询","expected":"语法错误"}],"cleanup":[],"blocked_reason":null,"derived_from":null,"reason":null}],
            "coverage":[{"feature":"查询","entry_point":"api","category":"normal","status":"planned","rationale":"正常读取","scenario_ids":["normal"]}],"limitations":[]},
        "issue-reproducer":{"summary":"复现异常","operations":[],"observations":[],"limitations":[],"followups":[],"issues":[
            {"id":"issue-1","title":"错误响应缺失","scenario_ids":["abnormal"],"expected":"语法错误","observed":"空响应","trigger":"非法 SQL","reproduction":["提交非法查询"],"evidence":["executions/abnormal/stdout.txt"]}]},
        "result-reviewer":{"summary":"独立核验","scenario_results":[
            {"scenario_id":"normal","status":"passed","rationale":"返回预期行","evidence":["executions/normal/stdout.txt"],"assessment":{"executed":true,"expectation_met":true,"infrastructure_failure":false,"no_mocks":true,"real_services_verified":true,"entry_point_verified":true}},
            {"scenario_id":"abnormal","status":"failed","rationale":"缺少语法错误","evidence":["executions/abnormal/stdout.txt"],"assessment":{"executed":true,"expectation_met":false,"infrastructure_failure":false,"no_mocks":true,"real_services_verified":true,"entry_point_verified":true}}],
            "issue_results":[{"issue_id":"issue-1","status":"not_reproduced","rationale":"ungrounded reviewer value","evidence":[]}],"limitations":["只验证当前版本"]}
    })
}

fn execution() -> Value {
    json!({"operations":[
        {"operation_id":"op-normal","scenario_id":"normal","step_id":"query","command":["curl","https://db.example:6041/rest/sql"],"purpose":"check","scenario_version":"v1","directory":"executions/normal","result":{"returncode":0,"timed_out":false},"evidence":{"complete":true},"required_services":["remote"],"service_checks":[{"service":"remote","source":"external","endpoint":"https://db.example:6041","directory":"executions/normal/services/remote","result":{"returncode":0,"stdout":"server_version=3.3.6.0"}}],"target_executed":true},
        {"operation_id":"op-abnormal","scenario_id":"abnormal","step_id":"query","command":["taos","-s","invalid"],"purpose":"action","scenario_version":"v1","directory":"executions/abnormal","result":{"returncode":0,"timed_out":false},"evidence":{"complete":true},"required_services":["remote"],"service_checks":[],"target_executed":true}]})
}

#[test]
fn product_report_lists_all_scenarios_and_counts_issues_separately() {
    // Catches routing to the legacy candidate renderer or counting only defects as results.
    let temp = tempfile::tempdir().unwrap();
    let verdicts = json!([{"issue_id":"issue-1","status":"confirmed","rationale":"harness grounded","evidence":["executions/abnormal/stdout.txt"]}]);
    let report =
        generate_report(temp.path(), &state(), &artifacts(), &execution(), &verdicts).unwrap();
    assert_eq!(report["schema_version"], "2.0");
    assert_eq!(report["scenario_results"].as_array().unwrap().len(), 2);
    assert_eq!(report["scenario_results"][0]["status"], "passed");
    assert_eq!(report["scenario_results"][1]["status"], "failed");
    assert_eq!(report["issues"].as_array().unwrap().len(), 1);
    assert_eq!(report["issue_results"], verdicts);
    assert_eq!(
        report["scenario_coverage"][0]["scenario_ids"],
        json!(["normal"])
    );
    assert_eq!(
        report["execution_attempts"][0]["operation_ids"],
        json!(["old-operation"])
    );
    assert_eq!(report["execution"], execution());
    assert_eq!(report["document_revision"], 2);
    assert_eq!(
        report["document_changes"][0]["instruction"],
        "add boundary scenario"
    );
    assert_eq!(read_json(&temp.path().join("report.json")).unwrap(), report);
    let md = fs::read_to_string(temp.path().join("report.md")).unwrap();
    let summary = md.split("## 验证结论").next().unwrap();
    assert_eq!(
        summary
            .lines()
            .filter(|line| line.starts_with("| normal") || line.starts_with("| abnormal"))
            .count(),
        2
    );
    assert!(summary.contains("正常 \\| 查询 返回"));
    assert!(md.contains("通过：1 项") && md.contains("失败：1 项"));
    assert!(md.contains("已确认缺陷：1 项"));
    for chapter in [
        "验证摘要",
        "验证结论",
        "验证范围与进度",
        "环境与证据",
        "验证结果",
        "问题记录",
        "验证路径调整",
        "验证限制",
        "环境保留与再次验证",
    ] {
        assert!(md.contains(&format!("## {chapter}")), "{chapter}");
    }
    assert!(!md.contains("80%"));
    assert!(!md.contains("TEST_ID"));
}

#[test]
fn product_report_links_document_and_real_service_evidence_with_distinct_source_version() {
    // Catches reports that confuse a local source commit with the deployed remote product version.
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("steps.md"),
        "# accepted scenario document\n",
    )
    .unwrap();
    let report = generate_report(
        temp.path(),
        &state(),
        &artifacts(),
        &execution(),
        &json!([]),
    )
    .unwrap();
    assert_eq!(report["document_path"], "steps.md");
    assert_eq!(report["services"][0]["source"], "external");
    assert_eq!(report["services"][0]["version"], "3.3.6.0");
    assert_eq!(report["services"][1]["source"], "docker");
    assert_eq!(report["entry_points"][0]["kind"], "api");
    let md = fs::read_to_string(temp.path().join("report.md")).unwrap();
    assert!(md.contains("[steps.md](steps.md)"));
    assert!(md.contains("[执行记录](executions/normal/execution.json)"));
    assert!(md.contains("[executions/normal/stdout.txt](executions/normal/stdout.txt)"));
    assert!(md.contains("[服务检查](executions/normal/services/remote/execution.json)"));
    assert!(md.contains("external") && md.contains("docker"));
    assert!(md.contains("https://db.example:6041") && md.contains("3.3.6.0"));
    assert!(md.contains("源码提交：`source-commit-123`"));
    assert!(md.contains("外部服务实际版本"));
    assert_eq!(
        fs::read_to_string(temp.path().join("steps.md")).unwrap(),
        "# accepted scenario document\n"
    );
}

#[test]
fn incomplete_product_runs_never_promote_exit_codes_or_history_to_passed_results() {
    // Catches implicit success from exit code zero, missing review, or obsolete history.
    let temp = tempfile::tempdir().unwrap();
    let mut input = artifacts();
    input.as_object_mut().unwrap().remove("result-reviewer");
    input["scenario-designer"]["scenarios"][1]["blocked_reason"] = json!("缺少合法测试账号");
    input["scenario-designer"]["scenarios"].as_array_mut().unwrap().push(json!({"id":"untouched","title":"未开始的边界场景","steps":[{"id":"query","action":"边界查询","expected":"空结果"}],"blocked_reason":null}));
    let mut interrupted = state();
    interrupted["status"] = json!("interrupted");
    interrupted["error"] = json!("coordinator interrupted");
    let report =
        generate_report(temp.path(), &interrupted, &input, &execution(), &json!([])).unwrap();
    assert_eq!(report["scenario_results"].as_array().unwrap().len(), 3);
    assert_eq!(report["scenario_results"][0]["status"], "blocked");
    assert_eq!(report["scenario_results"][1]["status"], "blocked");
    assert_eq!(
        report["scenario_results"][1]["rationale"],
        "缺少合法测试账号"
    );
    assert_eq!(report["scenario_results"][2]["status"], "not_executed");
    assert_eq!(report["issue_results"][0]["status"], "blocked");
    assert_eq!(
        report["scenario_results"][0]["assessment"]["expectation_met"],
        false
    );
    let md = fs::read_to_string(temp.path().join("report.md")).unwrap();
    assert!(md.contains("未执行：1 项") && md.contains("受阻：2 项"));
    assert!(md.contains("coordinator interrupted"));
}

#[test]
fn empty_product_design_is_explicitly_incomplete_and_legacy_artifacts_do_not_supply_results() {
    // Catches the empty-design case appearing as a successful product acceptance.
    let temp = tempfile::tempdir().unwrap();
    let report = generate_report(
        temp.path(),
        &state(),
        &json!({"bug-hunter":{"candidates":[{"id":"legacy"}]}}),
        &json!({}),
        &json!([]),
    )
    .unwrap();
    assert_eq!(report["scenarios"], json!([]));
    assert_eq!(report["scenario_results"], json!([]));
    let md = fs::read_to_string(temp.path().join("report.md")).unwrap();
    assert!(md.contains("尚未形成产品场景文档"));
    assert!(md.contains("不能据此判断产品通过"));
}

#[test]
fn refreshed_product_reports_recover_from_snapshot_and_keep_current_results_unique() {
    // Catches snapshot refresh losing product routing or resurrecting obsolete scenario results.
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("product-report");
    fs::create_dir_all(&directory).unwrap();
    let mut saved_state = state();
    saved_state["config"] = serde_json::to_value(aaqe::config::Config::default()).unwrap();
    let mut input = artifacts();
    input["result-reviewer"]["scenario_results"]
        .as_array_mut()
        .unwrap()
        .push(json!({"scenario_id":"removed-scene","status":"passed"}));
    write_json(&directory.join("harness.json"), &json!({"schema_version":1,"state":saved_state,"artifacts":input,"execution":{"baseline":[],"tests":[],"operations":[]},"verdicts":[]})).unwrap();
    fs::write(directory.join("report.json"), "corrupt derived report").unwrap();
    let report = refresh_report(&directory, &saved_state).unwrap();
    assert_eq!(report["schema_version"], "2.0");
    assert_eq!(report["scenario_results"].as_array().unwrap().len(), 2);
    assert_eq!(report["scenario_results"][0]["scenario_id"], "normal");
    assert_eq!(report["scenario_results"][1]["scenario_id"], "abnormal");
}
