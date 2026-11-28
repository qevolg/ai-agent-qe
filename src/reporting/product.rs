use super::{array, cell, json_section, text, yes_no};
use crate::{
    runtime::compose::strings,
    state::{now, write_json},
};
use anyhow::Result;
use serde_json::{Value, json};
use std::{fs, path::Path};

fn label(status: &str) -> &str {
    match status {
        "passed" => "通过",
        "not_executed" => "未执行",
        other => super::label(other),
    }
}

fn evidence_link(path: &str) -> String {
    // Angle brackets preserve paths containing spaces in Markdown destinations.
    let target = if path.contains([' ', '(', ')', '<', '>']) {
        format!("<{}>", path.replace('<', "%3C").replace('>', "%3E"))
    } else {
        path.to_owned()
    };
    format!(
        "[{}]({target})",
        path.replace('[', "\\[").replace(']', "\\]")
    )
}

fn record_link(record: &Value, title: &str) -> String {
    let directory = text(&record["directory"]).trim_end_matches('/');
    if directory.is_empty() {
        return "证据路径未记录".to_owned();
    }
    evidence_link(&format!("{directory}/execution.json")).replacen(
        &format!("[{directory}/execution.json]"),
        &format!("[{title}]"),
        1,
    )
}

fn evidence_links(value: &Value) -> String {
    let links = strings(value)
        .iter()
        .map(|path| evidence_link(path))
        .collect::<Vec<_>>();
    if links.is_empty() {
        "未记录".to_owned()
    } else {
        links.join("、")
    }
}

fn scenario_results(
    scenarios: &[Value],
    reviewer: &Value,
    execution: &Value,
    state: &Value,
) -> Vec<Value> {
    let reviewed = array(&reviewer["scenario_results"]);
    let operations = array(&execution["operations"]);
    scenarios
        .iter()
        .map(|scenario| {
            if let Some(result) = reviewed.iter().rev().find(|result| result["scenario_id"] == scenario["id"]) {
                // The harness grounds the saved reviewer artifact before publication.
                return result.clone();
            }
            let records = operations.iter().filter(|record| record["scenario_id"] == scenario["id"]).collect::<Vec<_>>();
            let blocker = scenario["blocked_reason"].as_str().filter(|reason| !reason.is_empty());
            let status = if blocker.is_some() || !records.is_empty() { "blocked" } else { "not_executed" };
            let rationale = blocker.map(str::to_owned).unwrap_or_else(|| {
                let reason = if records.is_empty() { "尚无当前场景的实际执行证据" } else { "已有操作记录，但尚无完整的独立验证结论；退出码不能单独证明产品行为正确" };
                match state["error"].as_str().filter(|error| !error.is_empty()) {
                    Some(error) => format!("{reason}；本轮错误：{error}"),
                    None => reason.to_owned(),
                }
            });
            let evidence = records.iter().filter_map(|record| record["directory"].as_str()).map(|directory| format!("{}/execution.json", directory.trim_end_matches('/'))).collect::<Vec<_>>();
            json!({"scenario_id":scenario["id"],"status":status,"rationale":rationale,"evidence":evidence,
                "assessment":{"executed":records.iter().any(|record| record["target_executed"] == true),"expectation_met":false,
                    "infrastructure_failure":records.iter().any(|record| record["target_executed"] == false || record["result"]["timed_out"] == true),
                    "no_mocks":false,"real_services_verified":false,"entry_point_verified":false}})
        })
        .collect()
}

pub(super) fn generate_report(
    directory: &Path,
    state: &Value,
    artifacts: &Value,
    execution: &Value,
    verdicts: &Value,
) -> Result<Value> {
    let explorer = &artifacts["product-explorer"];
    let designer = &artifacts["scenario-designer"];
    let reproducer = &artifacts["issue-reproducer"];
    let reviewer = &artifacts["result-reviewer"];
    let scenarios = array(&designer["scenarios"]);
    let results = scenario_results(&scenarios, reviewer, execution, state);
    let issues = array(&reproducer["issues"]);
    let saved_verdicts = array(verdicts);
    let issue_results = issues.iter().map(|issue| {
        saved_verdicts.iter().rev().find(|verdict| verdict["issue_id"] == issue["id"]).cloned().unwrap_or_else(|| {
            json!({"issue_id":issue["id"],"status":"blocked","rationale":state["error"].as_str().filter(|error| !error.is_empty()).unwrap_or("尚无完整的问题复现与独立验证结论"),"evidence":[]})
        })
    }).collect::<Vec<_>>();
    let document = json!({
        "schema_version":"2.0","workflow":"product","run_id":state["id"],"generated_at":now(),"prompt":state["prompt"],"status":state["status"],"error":state["error"],
        "workspace":state["workspace"],"publication":state["publication"],"stages":state["stages"],"finish_reason":state["finish_reason"],
        "coordinator":{"directory":directory.join("stages/coordinator"),"tool_log":directory.join("stages/coordinator/tools.jsonl")},
        "scope":array(&explorer["scope"]),"uncertainties":array(&explorer["uncertainties"]),"references":array(&explorer["references"]),
        "scenarios":scenarios,"scenario_results":results,"issues":issues,"issue_results":issue_results,"verdicts":issue_results,
        "scenario_coverage":array(&designer["coverage"]),"services":array(&explorer["services"]),"entry_points":array(&explorer["entry_points"]),"document_path":"steps.md",
        "execution":execution,"environment":state["environment"],
        "environment_started":state["environment_started"].as_bool().unwrap_or(false),"environment_ready":state["environment_ready"].as_bool().unwrap_or(false),
        "environment_actions":array(&state["environment_actions"]),"dispatch_rejections":array(&state["dispatch_rejections"]),"execution_attempts":array(&state["execution_attempts"]),
        "document_revision":state["document_revision"],"document_changes":array(&state["document_changes"]),
        "environment_retained":state["environment_retained"].as_bool().unwrap_or(false),
        "environment_cleaned_at":state["environment_cleaned_at"],"tmux_cleaned_at":state["tmux_cleaned_at"],"reproductions":array(&state["reproductions"])
    });
    write_json(&directory.join("report.json"), &document)?;
    fs::write(
        directory.join("report.md"),
        markdown(state, &document, artifacts)?,
    )?;
    Ok(document)
}

fn markdown(state: &Value, document: &Value, artifacts: &Value) -> Result<String> {
    let scenarios = array(&document["scenarios"]);
    let results = array(&document["scenario_results"]);
    let issues = array(&document["issues"]);
    let issue_results = array(&document["issue_results"]);
    let empty_result = "尚未形成产品场景文档；不能据此判断产品通过。";
    let mut md = format!(
        "# AAQE 验证报告\n\n运行编号：`{}`\n\n报告生成时间：{}（Asia/Shanghai；历史记录保留原时区）\n\n## 验证摘要\n\n| 验证对象／检查项 | 验证结论 | 简要说明 |\n| --- | --- | --- |\n",
        text(&state["id"]),
        text(&document["generated_at"])
    );
    if scenarios.is_empty() {
        md.push_str(&format!(
            "| 本轮产品场景 | 场景设计未完成 | {empty_result} |\n"
        ));
    }
    for (scenario, result) in scenarios.iter().zip(&results) {
        md.push_str(&format!(
            "| {}：{} | {} | {} |\n",
            cell(text(&scenario["id"])),
            cell(text(&scenario["title"])),
            label(text(&result["status"])),
            cell(text(&result["rationale"]))
        ));
    }
    md.push_str(&format!("\n## 验证结论\n\n任务状态：**{}（{}）**。任务完成不等于产品验收通过；场景结果、操作证据和问题结论分别记录。\n\n", label(text(&state["status"])), text(&state["status"])));
    if let Some(reason) = state["finish_reason"].as_str() {
        md.push_str(&format!("结束原因：{reason}\n\n"));
    }
    md.push_str("### 场景结论\n\n");
    for status in ["passed", "failed", "blocked", "not_executed", "flaky"] {
        let count = results
            .iter()
            .filter(|result| result["status"] == status)
            .count();
        md.push_str(&format!("- {}：{count} 项\n", label(status)));
    }
    md.push_str("\n### 问题结论\n\n");
    for status in [
        "confirmed",
        "not_reproduced",
        "blocked",
        "invalid_test",
        "flaky",
    ] {
        let count = issue_results
            .iter()
            .filter(|result| result["status"] == status)
            .count();
        md.push_str(&format!("- {}：{count} 项\n", label(status)));
    }
    md.push_str("\n汇总仅统计当前场景和当前问题各一次；历史重试、操作次数及复现记录不重复累计。没有问题记录不代表全部场景通过。\n\n");
    if scenarios.is_empty() {
        md.push_str(&format!("{empty_result}\n\n"));
    }
    md.push_str(&format!("## 验证范围与进度\n\n验证需求：\n\n{}\n\n操作步骤文档：[steps.md](steps.md)；场景修订及来源保存在文档中。执行必须引用已保存的场景和步骤。\n\n", text(&state["prompt"])));
    if state["workspace"].is_object() {
        md.push_str(&format!("源码提交：`{}`\n\n审计分支：`{}`\n\nWorktree：`{}`\n\n源码提交用于定位分析材料。外部服务实际版本单独列于服务记录，以真实协议检查的原始证据为准。\n\n", text(&state["workspace"]["commit"]), text(&state["workspace"]["branch"]), text(&state["workspace"]["path"])));
    }
    for scope in strings(&document["scope"]) {
        md.push_str(&format!("- {scope}\n"));
    }
    md.push('\n');
    json_section(&mut md, "入口适用性", &document["entry_points"])?;
    json_section(&mut md, "场景覆盖与空缺", &document["scenario_coverage"])?;
    if !array(&state["stages"]).is_empty() || state["loop_started_at"].is_string() {
        md.push_str(&format!("已启动阶段进程：{}；遭拒调度：{}\n\n### 阶段执行记录\n\n| 角色 | 尝试次数 | 状态 | 调度指令 | 证据目录 |\n| --- | --- | --- | --- | --- |\n", array(&state["stages"]).len(), array(&document["dispatch_rejections"]).len()));
        for entry in array(&state["stages"]) {
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                cell(text(&entry["name"])),
                entry["attempt"],
                label(text(&entry["status"])),
                cell(text(&entry["instruction"])),
                cell(text(&entry["directory"]))
            ));
        }
        md.push('\n');
    }
    md.push_str(&format!("## 环境与证据\n\n环境已启动：{}；就绪检查通过：{}。\n\n主调度记录：[stages/coordinator/tools.jsonl](stages/coordinator/tools.jsonl)；阶段产物及历史尝试保存在 `stages/`；当前有效产物在 `artifacts/`。\n\n### 服务来源与版本\n\n| 服务 | 来源 | 地址 | Compose 服务 | 服务版本（以原始检查为准） |\n| --- | --- | --- | --- | --- |\n", yes_no(document["environment_started"] == true), yes_no(document["environment_ready"] == true)));
    for service in array(&document["services"]) {
        md.push_str(&format!(
            "| {}：{} | {} | {} | {} | {} |\n",
            cell(text(&service["id"])),
            cell(text(&service["name"])),
            cell(text(&service["source"])),
            cell(text(&service["endpoint"])),
            cell(text(&service["compose_service"])),
            cell(
                service["version"]
                    .as_str()
                    .filter(|version| !version.is_empty())
                    .unwrap_or("未记录，不能由源码提交推定")
            )
        ));
    }
    md.push_str("\n每个服务使用提示词指定的外部地址或 Docker 真实实例。版本声明、真实协议检查及 Docker 容器身份随操作证据保留；地址不可用时相应验证受阻。\n\n");
    json_section(&mut md, "环境定义", &document["environment"])?;
    json_section(&mut md, "服务声明与检查命令", &document["services"])?;
    md.push_str("## 验证结果\n\n");
    if scenarios.is_empty() {
        md.push_str(&format!("{empty_result}\n\n"));
    }
    for (scenario, result) in scenarios.iter().zip(&results) {
        md.push_str(&format!("### 场景 `{}`：{}\n\n验证结论：**{}**\n\n功能：{}；入口：`{}`；类别：`{}`。\n\n预期判据：{}\n\n判断依据：{}\n\n证据：{}\n\n", text(&scenario["id"]), text(&scenario["title"]), label(text(&result["status"])), text(&scenario["feature"]), text(&scenario["entry_point"]), text(&scenario["category"]), text(&scenario["oracle"]), text(&result["rationale"]), evidence_links(&result["evidence"])));
        if let Some(reason) = scenario["blocked_reason"]
            .as_str()
            .filter(|reason| !reason.is_empty())
        {
            md.push_str(&format!("场景阻塞原因：{reason}\n\n"));
        }
        json_section(&mut md, "独立验证评估", &result["assessment"])?;
        let operations = array(&document["execution"]["operations"])
            .into_iter()
            .filter(|record| record["scenario_id"] == scenario["id"])
            .collect::<Vec<_>>();
        if operations.is_empty() {
            md.push_str("尚无当前场景的操作记录。\n\n");
        }
        for record in operations {
            md.push_str(&format!("- 操作 `{}`；步骤 `{}`；用途 `{}`；场景版本 `{}`；目标已执行：{}；退出码：{}；{}。\n", text(&record["operation_id"]), text(&record["step_id"]), text(&record["purpose"]), text(&record["scenario_version"]), yes_no(record["target_executed"] == true), record["result"]["returncode"], record_link(&record, "执行记录")));
            for check in array(&record["service_checks"]) {
                md.push_str(&format!(
                    "  - 服务 `{}`；来源 `{}`；地址 `{}`；{}。\n",
                    text(&check["service"]),
                    text(&check["source"]),
                    text(&check["endpoint"]),
                    record_link(&check, "服务检查")
                ));
            }
            md.push('\n');
            json_section(&mut md, "操作及原始服务检查", &record)?;
        }
    }
    md.push_str("## 问题记录\n\n");
    if issues.is_empty() {
        md.push_str("当前没有已记录的问题；产品结论以全部场景结果为准。\n\n");
    }
    for (issue, result) in issues.iter().zip(&issue_results) {
        md.push_str(&format!("### {}: {}\n\n验证结论：**{}**\n\n关联场景：{}\n\n预期行为：{}\n\n实际行为：{}\n\n触发条件：{}\n\n判断依据：{}\n\n证据：{}\n\n原始问题证据：{}\n\n", text(&issue["id"]), text(&issue["title"]), label(text(&result["status"])), strings(&issue["scenario_ids"]).join("、"), text(&issue["expected"]), text(&issue["observed"]), text(&issue["trigger"]), text(&result["rationale"]), evidence_links(&result["evidence"]), evidence_links(&issue["evidence"])));
        for step in strings(&issue["reproduction"]) {
            md.push_str(&format!("- {step}\n"));
        }
        md.push('\n');
    }
    md.push_str("## 验证路径调整\n\n新线索在本轮范围内先补写场景文档再执行；范围外只记录。修订保留历史，变更场景的旧证据不能替代当前结果。\n\n");
    for rejection in array(&document["dispatch_rejections"]) {
        md.push_str(&format!(
            "- `{}` 调度被拒：{}；可用恢复动作：`{}`。本次请求未启动阶段进程。\n",
            text(&rejection["stage"]),
            text(&rejection["error"]),
            rejection["recovery"]
        ));
    }
    md.push('\n');
    json_section(&mut md, "环境操作记录", &document["environment_actions"])?;
    json_section(&mut md, "场景修订记录", &document["document_changes"])?;
    json_section(
        &mut md,
        "追查建议",
        &artifacts["issue-reproducer"]["followups"],
    )?;
    json_section(&mut md, "历史执行轮次", &document["execution_attempts"])?;
    md.push_str("## 验证限制\n\n结论限于实际入口、当前场景版本、真实服务及已保存的操作证据；未执行、受阻和不稳定的场景不能计为通过。进程退出码本身不能证明业务判据满足；Explorer 页面验证需要实际浏览器操作。\n\n");
    for note in strings(&document["uncertainties"]).into_iter().chain(
        [
            "scenario-designer",
            "product-executor",
            "issue-reproducer",
            "result-reviewer",
        ]
        .into_iter()
        .flat_map(|stage| strings(&artifacts[stage]["limitations"])),
    ) {
        md.push_str(&format!("- {note}\n"));
    }
    if let Some(error) = state["error"].as_str() {
        md.push_str(&format!("\n本轮错误原文：\n\n```text\n{error}\n```\n"));
    }
    md.push_str("\n## 环境保留与再次验证\n\n");
    if document["environment_retained"] == true {
        md.push_str("本轮 Docker 资源已保留，便于复现；外部服务继续由其原所有者管理。\n\n");
    }
    for (key, name) in [
        ("environment_cleaned_at", "本轮 Docker 资源已清理"),
        ("tmux_cleaned_at", "本轮 tmux 会话已清理"),
    ] {
        if let Some(at) = state[key].as_str() {
            md.push_str(&format!("{name}，时间：{at}。\n\n"));
        }
    }
    json_section(&mut md, "复现记录", &document["reproductions"])?;
    let id = text(&state["id"]);
    md.push_str(&format!("### 再次验证命令\n\n```bash\naaqe reproduce {id}\naaqe env status {id}\naaqe env shell {id}\naaqe cleanup {id}\n```\n\n再次验证重放已保存的操作，保留原始独立验证结论；查看每次复现记录中的执行与服务检查结果。清理保留报告、操作步骤文档及 Worktree。\n\n阶段原始产物和每次操作的进程记录均保存在本报告所在的运行目录中。\n"));
    Ok(md)
}
