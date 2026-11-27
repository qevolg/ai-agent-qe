use crate::{
    runtime::compose::strings,
    state::{HarnessSnapshot, now, read_json, write_json},
};
use anyhow::Result;
use serde_json::{Value, json};
use std::{fs, path::Path};

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}

fn array(value: &Value) -> Vec<Value> {
    value.as_array().cloned().unwrap_or_default()
}

pub fn refresh_report(directory: &Path, state: &Value) -> Result<Value> {
    if directory.join("harness.json").exists() {
        let saved = HarnessSnapshot::load(directory)?;
        return generate_report(
            directory,
            &saved.state,
            &saved.artifacts,
            &saved.execution,
            &saved.verdicts,
        );
    }
    let previous = read_json(&directory.join("report.json"))?;
    let mut artifacts = json!({});
    let artifact_dir = directory.join("artifacts");
    if artifact_dir.exists() {
        for entry in fs::read_dir(artifact_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json")
                && let Some(stage) = path.file_stem().and_then(|s| s.to_str())
            {
                artifacts[stage] = read_json(&path)?;
            }
        }
    }
    generate_report(
        directory,
        state,
        &artifacts,
        &previous["execution"],
        &previous["verdicts"],
    )
}

pub fn generate_report(
    directory: &Path,
    state: &Value,
    artifacts: &Value,
    execution: &Value,
    verdicts: &Value,
) -> Result<Value> {
    let candidates = array(&artifacts["bug-hunter"]["candidates"]);
    let plans = array(&artifacts["test-designer"]["plans"]);
    let generated_tests = array(&artifacts["test-developer"]["tests"]);
    let mut verdicts = array(verdicts);
    for candidate in &candidates {
        if !verdicts
            .iter()
            .any(|v| v["candidate_id"] == candidate["id"])
        {
            let blockers: Vec<&str> = plans
                .iter()
                .filter(|plan| plan["candidate_id"] == candidate["id"])
                .flat_map(|plan| {
                    ["blocked_reason", "execution_blocked_reason"].map(|key| &plan[key])
                })
                .filter_map(Value::as_str)
                .filter(|reason| !reason.is_empty())
                .collect();
            let rationale = if blockers.is_empty() {
                state["error"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("尚无完整验证结论")
                    .to_owned()
            } else {
                blockers.join("; ")
            };
            verdicts.push(json!({"candidate_id":candidate["id"],"test_ids":[],"status":"blocked","rationale":rationale,"evidence":[]}));
        }
    }
    let candidate_results: Vec<Value> = candidates
        .iter()
        .map(|candidate| {
            let plans: Vec<Value> = plans
                .iter()
                .filter(|p| p["candidate_id"] == candidate["id"])
                .cloned()
                .collect();
            let generated: Vec<Value> = generated_tests
                .iter()
                .filter(|t| t["candidate_id"] == candidate["id"])
                .cloned()
                .collect();
            let ids: Vec<&Value> = plans
                .iter()
                .map(|p| &p["test_id"])
                .chain(generated.iter().map(|t| &t["id"]))
                .collect();
            let tests: Vec<Value> = array(&execution["tests"])
                .into_iter()
                .filter(|record| ids.contains(&&record["test_id"]))
                .collect();
            let baseline: Vec<Value> = array(&execution["baseline"])
                .into_iter()
                .filter(|record| {
                    if record["for_test_id"].is_string() {
                        ids.contains(&&record["for_test_id"])
                    } else {
                        !generated.is_empty()
                    }
                })
                .collect();
            json!({"candidate_id":candidate["id"],"plans":plans,"generated_tests":generated,
            "execution":{"tests":tests,"baseline":baseline}})
        })
        .collect();
    let document = json!({
        "schema_version":"1.0","run_id":state["id"],"generated_at":now(),"prompt":state["prompt"],"status":state["status"],"error":state["error"],
        "workspace":state["workspace"],"publication":state["publication"],"coverage":execution["coverage"],"coverage_assessment":artifacts["test-validator"]["coverage_assessment"],"stages":state["stages"],"finish_reason":state["finish_reason"],"coordinator":{"directory":directory.join("stages/coordinator"),"tool_log":directory.join("stages/coordinator/tools.jsonl")},"scope":array(&artifacts["code-explorer"]["scope"]),"uncertainties":array(&artifacts["code-explorer"]["uncertainties"]),
        "excluded_areas":array(&artifacts["bug-hunter"]["excluded_areas"]),"candidates":candidates,"verdicts":verdicts,"execution":execution,"environment":state["environment"],
        "candidate_results":candidate_results,"environment_started":state["environment_started"].as_bool().unwrap_or(false),"environment_ready":state["environment_ready"].as_bool().unwrap_or(false),
        "environment_actions":array(&state["environment_actions"]),"dispatch_rejections":array(&state["dispatch_rejections"]),"execution_attempts":array(&state["execution_attempts"]),
        "environment_retained":state["environment_retained"].as_bool().unwrap_or(false),"environment_cleaned_at":state["environment_cleaned_at"],"tmux_cleaned_at":state["tmux_cleaned_at"],"reproductions":array(&state["reproductions"])
    });
    write_json(&directory.join("report.json"), &document)?;
    fs::write(
        directory.join("report.md"),
        markdown(state, &document, artifacts)?,
    )?;
    Ok(document)
}

fn label(status: &str) -> &str {
    match status {
        "confirmed" => "已确认缺陷",
        "not_reproduced" => "未复现",
        "blocked" => "受阻",
        "invalid_test" => "测试无效",
        "flaky" => "结果不稳定",
        "completed" => "已完成",
        "initializing" => "初始化中",
        "running" => "运行中",
        "failed" => "失败",
        "interrupted" | "cancelled" => "已中断",
        "accepted" => "已接受",
        "rejected" => "已拒绝",
        "ok" => "已记录",
        "" => "未记录",
        other => other,
    }
}

fn cell(value: &str) -> String {
    value.replace('|', "\\|").replace(['\n', '\r'], " ")
}

fn yes_no(value: bool) -> &'static str {
    if value { "是" } else { "否" }
}

fn json_section(md: &mut String, heading: &str, value: &Value) -> Result<()> {
    if !value.is_null() && value != &json!([]) && value != &json!({}) {
        md.push_str(&format!(
            "### {heading}\n\n```json\n{}\n```\n\n",
            serde_json::to_string_pretty(value)?
        ));
    }
    Ok(())
}

fn markdown(state: &Value, document: &Value, artifacts: &Value) -> Result<String> {
    let candidates = array(&document["candidates"]);
    let verdicts = array(&document["verdicts"]);
    let analyzed = artifacts["bug-hunter"]["candidates"].is_array();
    let empty_result = if analyzed {
        "已检查范围内未发现缺陷候选；不代表产品全面通过。"
    } else {
        "尚未形成有效的缺陷分析结果；不能据此判断产品通过。"
    };
    let mut md = format!(
        "# AAQE 验证报告\n\n运行编号：`{}`\n\n报告生成时间：{}（Asia/Shanghai；历史记录保留原时区）\n\n## 验证摘要\n\n| 验证对象／检查项 | 验证结论 | 简要说明 |\n| --- | --- | --- |\n",
        text(&state["id"]),
        text(&document["generated_at"])
    );
    if candidates.is_empty() {
        md.push_str(&format!(
            "| 本轮检查范围 | {} | {empty_result} |\n",
            if analyzed {
                "未发现候选"
            } else {
                "分析未完成"
            }
        ));
    }
    for candidate in &candidates {
        let verdict = verdicts
            .iter()
            .find(|v| v["candidate_id"] == candidate["id"])
            .expect("candidate verdict generated");
        md.push_str(&format!(
            "| {}：{} | {} | {} |\n",
            cell(text(&candidate["id"])),
            cell(text(&candidate["title"])),
            cell(label(text(&verdict["status"]))),
            cell(text(&verdict["rationale"]))
        ));
    }
    md.push_str(&format!("\n## 验证结论\n\n审计状态：**{}（{}）**。审计完成不等于产品验收通过；缺陷候选、实际执行及独立验证结论分别记录。\n\n", label(text(&state["status"])), text(&state["status"])));
    if let Some(reason) = state["finish_reason"].as_str() {
        md.push_str(&format!("结束原因：{reason}\n\n"));
    }
    if !candidates.is_empty() {
        for status in [
            "confirmed",
            "not_reproduced",
            "blocked",
            "invalid_test",
            "flaky",
        ] {
            let count = candidates
                .iter()
                .filter(|candidate| {
                    verdicts
                        .iter()
                        .any(|v| v["candidate_id"] == candidate["id"] && v["status"] == status)
                })
                .count();
            md.push_str(&format!("- {}：{count} 项\n", label(status)));
        }
        md.push_str("\n汇总仅统计当前有效候选和判定；历史重试、执行轮次及复现记录不重复累计。\n\n");
    } else {
        md.push_str(&format!("{empty_result}\n\n"));
    }
    md.push_str(&format!(
        "## 验证范围与进度\n\n验证需求：\n\n{}\n\n",
        text(&state["prompt"])
    ));
    if state["workspace"].is_object() {
        let workspace = &state["workspace"];
        md.push_str(&format!(
            "被测提交：`{}`\n\n审计分支：`{}`\n\nWorktree：`{}`\n\n",
            text(&workspace["commit"]),
            text(&workspace["branch"]),
            text(&workspace["path"])
        ));
    }
    for scope in strings(&document["scope"]) {
        md.push_str(&format!("- {scope}\n"));
    }
    md.push('\n');
    if state["loop_started_at"].is_string() {
        md.push_str(&format!("已启动阶段进程：{}；遭拒调度：{}\n\n### 阶段执行记录\n\n| 角色 | 尝试次数 | 状态 | 调度指令 | 证据目录 |\n| --- | --- | --- | --- | --- |\n", array(&state["stages"]).len(), array(&document["dispatch_rejections"]).len()));
        for entry in array(&state["stages"]) {
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                cell(text(&entry["name"])),
                entry["attempt"],
                cell(label(text(&entry["status"]))),
                cell(text(&entry["instruction"])),
                cell(text(&entry["directory"]))
            ));
        }
        md.push('\n');
    }
    md.push_str(&format!("## 环境与证据\n\n环境已启动：{}；就绪检查通过：{}。\n\n主调度记录：`stages/coordinator/tools.jsonl`；阶段产物及历史尝试保存在 `stages/`；当前有效产物在 `artifacts/`。\n\n基线、目标测试及原始执行证据通过执行记录中的目录关联。\n\n", yes_no(document["environment_started"] == true), yes_no(document["environment_ready"] == true)));
    json_section(&mut md, "环境定义", &document["environment"])?;
    json_section(
        &mut md,
        "目标模块行覆盖率（要求至少 80%）",
        &document["coverage"],
    )?;
    json_section(&mut md, "覆盖率独立验证", &document["coverage_assessment"])?;
    if let Some(url) = document["publication"]["mr_url"].as_str() {
        md.push_str(&format!("## 测试交付\n\nGitLab MR：{url}\n\n"));
    }
    json_section(&mut md, "提交与发布状态", &document["publication"])?;
    md.push_str("## 验证结果\n\n");
    if candidates.is_empty() {
        md.push_str(&format!("{empty_result}\n\n"));
    }
    for result in array(&document["candidate_results"]) {
        md.push_str(&format!(
            "### 候选 `{}` 的测试与执行\n\n",
            text(&result["candidate_id"])
        ));
        for plan in array(&result["plans"]) {
            md.push_str(&format!(
                "- 测试计划 `{}`：设计阻塞原因：{}；执行前置条件：{}\n",
                text(&plan["test_id"]),
                plan["blocked_reason"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("未记录"),
                plan["execution_blocked_reason"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("未记录")
            ));
        }
        let ids: Vec<Value> = array(&result["plans"])
            .iter()
            .map(|p| p["test_id"].clone())
            .chain(
                array(&result["generated_tests"])
                    .iter()
                    .map(|t| t["id"].clone()),
            )
            .collect();
        let mut shown = Vec::new();
        for id in ids {
            if shown.contains(&id) {
                continue;
            }
            shown.push(id.clone());
            let generated = array(&result["generated_tests"])
                .iter()
                .any(|test| test["id"] == id);
            let executions: Vec<Value> = array(&result["execution"]["tests"])
                .into_iter()
                .filter(|test| test["test_id"] == id)
                .collect();
            md.push_str(&format!(
                "- 测试 `{}`：已生成：{}；当前执行次数：{}\n",
                text(&id),
                yes_no(generated),
                executions.len()
            ));
            for record in executions {
                md.push_str(&format!(
                    "  - 退出码：{}；执行记录完整：{}；证据：`{}/execution.json`\n",
                    record["result"]["returncode"],
                    yes_no(
                        record["evidence"]["complete"] == true
                            || record["junit"]["complete"] == true
                    ),
                    text(&record["directory"])
                ));
            }
        }
        for baseline in array(&result["execution"]["baseline"]) {
            md.push_str(&format!(
                "- 基线 `{}`：退出码：{}；证据：`{}/execution.json`\n",
                text(&baseline["test_id"]),
                baseline["result"]["returncode"],
                text(&baseline["directory"])
            ));
        }
        md.push('\n');
    }
    md.push_str("## 问题记录\n\n");
    if candidates.is_empty() {
        md.push_str(&format!("{empty_result}\n\n"));
    }
    for candidate in &candidates {
        let verdict = verdicts
            .iter()
            .find(|v| v["candidate_id"] == candidate["id"])
            .expect("candidate verdict generated");
        md.push_str(&format!("### {}: {}\n\n验证结论：**{}**\n\n预期行为：{}\n\n实际行为：{}\n\n触发条件：{}\n\n判断依据：{}\n\n证据：{}\n\n", text(&candidate["id"]), text(&candidate["title"]), label(text(&verdict["status"])), text(&candidate["expected"]), text(&candidate["observed"]), text(&candidate["trigger"]), text(&verdict["rationale"]), strings(&verdict["evidence"]).join("、")));
    }
    md.push_str("## 验证路径调整\n\n环境准备、阶段重试、遭拒调度和执行轮次单独记录；历史结果不能替代当前有效验证。\n\n");
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
    json_section(&mut md, "历史执行轮次", &document["execution_attempts"])?;
    md.push_str("## 验证限制\n\n结论限于实际检查范围和已保存的执行证据；未执行、跳过或受阻的项目不能计为通过。\n\n");
    for note in strings(&document["uncertainties"])
        .into_iter()
        .chain(strings(&document["excluded_areas"]))
    {
        md.push_str(&format!("- {note}\n"));
    }
    if let Some(error) = state["error"].as_str() {
        md.push_str(&format!("\n本轮错误原文：\n\n```text\n{error}\n```\n"));
    }
    md.push_str("\n## 环境保留与再次验证\n\n");
    if state["environment_retained"] == true {
        md.push_str("本轮 Docker 资源已保留，便于复现；外部容器继续由其原所有者管理。\n\n");
    }
    for (key, name) in [
        ("environment_cleaned_at", "本轮 Docker 资源已清理"),
        ("tmux_cleaned_at", "本轮 tmux 会话已清理"),
    ] {
        if let Some(at) = state[key].as_str() {
            md.push_str(&format!("{name}，时间：{at}。\n\n"));
        }
    }
    if !array(&state["reproductions"]).is_empty() {
        md.push_str("### 复现记录\n\n复现记录追加保存，保留原始审计结论。\n\n");
        for attempt in array(&state["reproductions"]) {
            md.push_str(&format!("复现时间：{}\n\n", text(&attempt["at"])));
            if !attempt["status"].is_null() {
                md.push_str(&format!(
                    "状态：{}；阶段：`{}`；记录：`{}/reproduction.json`\n\n",
                    label(text(&attempt["status"])),
                    text(&attempt["phase"]),
                    text(&attempt["directory"])
                ));
            }
            if let Some(error) = attempt["error"].as_str() {
                md.push_str(&format!("错误原文：{error}\n\n"));
            }
            for preparation in array(&attempt["preparations"]) {
                md.push_str(&format!(
                    "- 准备步骤 `{}`：退出码：{}；证据：`{}/execution.json`\n",
                    text(&preparation["test_id"]),
                    preparation["result"]["returncode"],
                    text(&preparation["directory"])
                ));
            }
            for record in array(&attempt["records"]) {
                md.push_str(&format!(
                    "- 测试 `{}`：退出码：{}；证据：`{}/execution.json`\n",
                    text(&record["test_id"]),
                    record["result"]["returncode"],
                    text(&record["directory"])
                ));
            }
            md.push('\n');
        }
    }
    let id = text(&state["id"]);
    md.push_str(&format!("### 再次验证命令\n\n```bash\naaqe reproduce {id} --case TEST_ID\naaqe env status {id}\naaqe env shell {id}\naaqe cleanup {id}\n```\n\n`TEST_ID` 替换为实际测试 ID；省略 `--case` 重跑全部已保存测试。自定义运行目录时继续传入相同的 `--state-dir`。清理保留报告、测试补丁、分支及 Worktree。\n\n阶段原始产物、基线和每次执行的进程与测试记录均保存在本报告所在的运行目录中。\n"));
    Ok(md)
}
