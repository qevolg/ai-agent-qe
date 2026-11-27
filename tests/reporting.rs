use aaqe::reporting::{generate_report, refresh_report};
use serde_json::json;
use std::fs;

#[test]
fn reports_rebuild_from_snapshot_when_derived_files_are_missing_or_corrupt() {
    use aaqe::{
        config::Config,
        state::{Store, read_json, write_json},
    };
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path());
    let mut state = store
        .create("audit", &Config::default(), temp.path())
        .unwrap();
    let id = state["id"].as_str().unwrap().to_owned();
    let directory = store.path(&id).unwrap();
    state["status"] = json!("completed");
    let execution =
        json!({"baseline":[],"tests":[{"test_id":"original-test","result":{"returncode":1}}]});
    let verdicts =
        json!([{"candidate_id":"bug","status":"confirmed","rationale":"original audit"}]);
    write_json(&directory.join("harness.json"), &json!({"schema_version":1,"state":state,"artifacts":{"bug-hunter":{"candidates":[{"id":"bug","title":"Original candidate"}]}},"execution":execution,"verdicts":verdicts})).unwrap();
    for corrupt in [false, true] {
        if corrupt {
            fs::write(directory.join("report.json"), "broken derived report").unwrap();
        }
        let report = refresh_report(&directory, &state).unwrap();
        assert_eq!(report["execution"], execution);
        assert_eq!(report["verdicts"], verdicts);
        assert_eq!(report["candidates"][0]["title"], "Original candidate");
        assert!(
            fs::read_to_string(directory.join("report.md"))
                .unwrap()
                .contains("Original candidate")
        );
        assert_eq!(
            read_json(&directory.join("report.json")).unwrap()["execution"],
            execution
        );
    }
}

#[test]
fn reports_keep_candidate_blockers_and_generated_execution_evidence_distinct() {
    let temp = tempfile::tempdir().unwrap();
    let state = json!({"id":"report-test","status":"blocked","error":"run-level interruption"});
    let artifacts = json!({
        "bug-hunter":{"candidates":[
            {"id":"oracle","title":"Missing oracle"},
            {"id":"runner","title":"Missing runner"},
            {"id":"executed","title":"Executed candidate"}
        ]},
        "test-designer":{"plans":[
            {"candidate_id":"oracle","test_id":"oracle-test","blocked_reason":"No documented expected value"},
            {"candidate_id":"runner","test_id":"runner-test","blocked_reason":null,"execution_blocked_reason":"Runner compiler unavailable"},
            {"candidate_id":"executed","test_id":"executed-test","blocked_reason":null}
        ]},
        "test-developer":{"tests":[{"id":"runner-test","candidate_id":"runner"},{"id":"executed-test","candidate_id":"executed"}]}
    });
    let execution = json!({"baseline":[{"test_id":"baseline-unique","for_test_id":"executed-test","result":{"returncode":0}}],"tests":[{"test_id":"executed-test","directory":"evidence/executed","result":{"returncode":1},"junit":{"complete":true}}]});
    let report = generate_report(temp.path(), &state, &artifacts, &execution, &json!([])).unwrap();
    assert_eq!(
        report["verdicts"][0]["rationale"],
        "No documented expected value"
    );
    assert_eq!(
        report["verdicts"][1]["rationale"],
        "Runner compiler unavailable"
    );
    let results = report["candidate_results"].as_array().unwrap();
    assert_eq!(
        results[0]["plans"][0]["blocked_reason"],
        "No documented expected value"
    );
    assert_eq!(results[0]["generated_tests"], json!([]));
    assert_eq!(results[1]["generated_tests"][0]["id"], "runner-test");
    assert_eq!(results[1]["execution"]["tests"], json!([]));
    assert_eq!(results[1]["execution"]["baseline"], json!([]));
    assert_eq!(
        results[2]["execution"]["tests"][0]["test_id"],
        "executed-test"
    );
    assert_eq!(
        results[2]["execution"]["baseline"][0]["test_id"],
        "baseline-unique"
    );
    let md = fs::read_to_string(temp.path().join("report.md")).unwrap();
    let runner = md
        .split("### 候选 `runner` 的测试与执行")
        .nth(1)
        .unwrap()
        .split("### 候选 `executed` 的测试与执行")
        .next()
        .unwrap();
    assert!(runner.contains("Runner compiler unavailable"));
    assert!(runner.contains("已生成：是；当前执行次数：0"));
    assert!(!runner.contains("No documented expected value"));
    assert!(!runner.contains("evidence/executed"));
}

#[test]
fn reports_separate_dispatch_rejections_from_launched_workers_and_preserve_history() {
    let temp = tempfile::tempdir().unwrap();
    let state = json!({"id":"history","status":"blocked","loop_started_at":"now",
        "stages":[{"name":"code-explorer","attempt":1,"status":"accepted"}],
        "dispatch_rejections":[{"stage":"test-developer","gate":"dispatch","error":"Missing plans","recovery":{"permitted_stages":["test-designer"]}}],
        "environment_actions":[{"operation":"inspect_environment","status":"ok","result":{"success":false,"directory":"environment/probes/probe-one"}}],
        "execution_attempts":[{"directory":"executions/rounds/previous","test_ids":["old-test"],"status":"failed"}],
        "environment_ready":false});
    let report = generate_report(temp.path(), &state, &json!({}), &json!({}), &json!([])).unwrap();
    assert_eq!(report["stages"].as_array().unwrap().len(), 1);
    assert_eq!(report["dispatch_rejections"][0]["error"], "Missing plans");
    assert_eq!(report["environment_actions"][0]["result"]["success"], false);
    assert_eq!(
        report["execution_attempts"][0]["directory"],
        "executions/rounds/previous"
    );
    let md = fs::read_to_string(temp.path().join("report.md")).unwrap();
    assert!(md.contains("已启动阶段进程：1；遭拒调度：1"));
    assert!(md.contains("Missing plans"));
    assert!(md.contains("environment/probes/probe-one"));
    assert!(md.contains("executions/rounds/previous"));
    let refreshed = refresh_report(temp.path(), &state).unwrap();
    assert_eq!(
        refreshed["dispatch_rejections"],
        report["dispatch_rejections"]
    );
}

#[test]
fn chinese_summary_uses_current_verdicts_and_escapes_multiline_table_cells() {
    let temp = tempfile::tempdir().unwrap();
    let statuses = [
        "confirmed",
        "not_reproduced",
        "blocked",
        "invalid_test",
        "flaky",
    ];
    let candidates: Vec<_> = statuses.iter().enumerate().map(|(n, _)| json!({"id":format!("BUG-{n}"), "title":"边界 | case\n下一行", "expected":"有效行为", "observed":"实际结果", "trigger":"边界输入"})).collect();
    let verdicts: Vec<_> = statuses.iter().enumerate().map(|(n, status)| json!({"candidate_id":format!("BUG-{n}"),"status":status,"rationale":"独立验证 | 依据\n第二行","evidence":[]})).collect();
    let state = json!({"id":"20261125081514-ab12cd34","status":"completed","prompt":"检查边界", "execution_attempts":[{"test_ids":["historical-only"]}],"reproductions":[{"at":"2026-11-25T08:15:14+08:00","records":[{"test_id":"replayed-only","result":{"returncode":0}}]}]});
    let artifacts =
        json!({"bug-hunter":{"candidates":candidates},"code-explorer":{"scope":["边界模块"]}});
    let report = generate_report(
        temp.path(),
        &state,
        &artifacts,
        &json!({}),
        &json!(verdicts),
    )
    .unwrap();
    let md = fs::read_to_string(temp.path().join("report.md")).unwrap();
    let summary = md.split("## 验证结论").next().unwrap();
    assert!(summary.contains("| 验证对象／检查项 | 验证结论 | 简要说明 |"));
    assert_eq!(
        summary
            .lines()
            .filter(|line| line.starts_with("| BUG-"))
            .count(),
        5
    );
    for label in ["已确认缺陷", "未复现", "受阻", "测试无效", "结果不稳定"] {
        assert!(summary.contains(label), "{label}: {summary}");
    }
    assert!(summary.contains("边界 \\| case 下一行"));
    assert!(summary.contains("独立验证 \\| 依据 第二行"));
    assert!(!summary.contains("historical-only") && !summary.contains("replayed-only"));
    assert!(md.contains("审计完成不等于产品验收通过"));
    for heading in [
        "验证范围与进度",
        "环境与证据",
        "验证结果",
        "问题记录",
        "验证路径调整",
        "验证限制",
        "环境保留与再次验证",
    ] {
        assert!(md.contains(&format!("## {heading}")), "{heading}");
    }
    for (n, status) in statuses.iter().enumerate() {
        assert_eq!(report["verdicts"][n]["status"], *status);
    }
}

#[test]
fn absent_analysis_and_examined_empty_scope_have_different_summary_results() {
    let temp = tempfile::tempdir().unwrap();
    let state = json!({"id":"empty","status":"blocked"});
    generate_report(temp.path(), &state, &json!({}), &json!({}), &json!([])).unwrap();
    let pending = fs::read_to_string(temp.path().join("report.md")).unwrap();
    assert!(pending.contains("尚未形成有效的缺陷分析结果"));
    assert!(!pending.contains("已检查范围内未发现缺陷候选"));
    generate_report(
        temp.path(),
        &state,
        &json!({"bug-hunter":{"candidates":[]}}),
        &json!({}),
        &json!([]),
    )
    .unwrap();
    let examined = fs::read_to_string(temp.path().join("report.md")).unwrap();
    assert!(examined.contains("已检查范围内未发现缺陷候选"));
    assert!(examined.contains("不代表产品全面通过"));
}
