//! Documented operations shared by staged execution and saved replay.
use crate::{audit::interfaces::TestRuntime, guards::product, state::write_json};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::Path;

pub(super) fn reproduce(
    store: &crate::state::Store,
    state: &mut Value,
    directory: &Path,
    scenario_id: Option<&str>,
) -> Result<Value> {
    use crate::{
        config::Config,
        guards::patch::PatchGate,
        runtime::compose::ComposeEnvironment,
        state::{HarnessSnapshot, now},
    };
    let saved = HarnessSnapshot::load(directory)?;
    let explorer = &saved.artifacts["product-explorer"];
    let designer = &saved.artifacts["scenario-designer"];
    let scenarios = product::items(designer, "scenarios")?;
    anyhow::ensure!(
        scenario_id.is_none_or(|id| scenarios.iter().any(|s| s["id"] == id)),
        "Unknown scenario ID"
    );
    product::validate_services(
        explorer,
        state["prompt"].as_str().context("Missing run prompt")?,
    )?;
    anyhow::ensure!(
        state["environment"]["product_services"] == explorer["services"],
        "Saved product service targets differ from the accepted manifest"
    );
    let operations = product::items(&saved.execution, "operations")?.iter()
        .filter(|r| scenario_id.is_none_or(|id| r["scenario_id"] == id))
        .filter(|r| scenarios.iter().any(|s| s["id"] == r["scenario_id"] && r["scenario_version"] == product::version(s)))
        .map(|r| json!({"id":r["operation_id"],"scenario_id":r["scenario_id"],"step_id":r["step_id"],"command":r["command"],"purpose":r["purpose"]})).collect::<Vec<_>>();
    anyhow::ensure!(
        !operations.is_empty(),
        "No saved product operations for the selected scenarios"
    );
    product::validate_links(
        "product-executor",
        &json!({"operations":operations}),
        &saved.artifacts,
    )?;
    let config: Config = serde_json::from_value(state["config"].clone())?;
    let root = Path::new(
        state["workspace"]["path"]
            .as_str()
            .context("Missing saved worktree")?,
    );
    let gate = PatchGate::new(
        root,
        &config.test_paths,
        state["workspace"]["commit"].as_str(),
    )?;
    gate.verify_worktree()?;
    let runtime = ComposeEnvironment::new(
        root,
        directory,
        state["id"].as_str().unwrap(),
        state["environment"].clone(),
        config.docker_timeout,
    );
    if !state["reproductions"].is_array() {
        state["reproductions"] = json!([]);
    }
    for attempt in state["reproductions"].as_array_mut().unwrap() {
        if attempt["status"] == "running" {
            attempt["status"] = json!("interrupted");
            attempt["finished_at"] = json!(now());
            attempt["error"] = json!("Previous reproduction exited before recording completion");
        }
    }
    let index = state["reproductions"].as_array().unwrap().len();
    let attempt = uuid::Uuid::new_v4().simple().to_string();
    let evidence = directory.join("reproductions").join(&attempt);
    state["reproductions"].as_array_mut().unwrap().push(json!({"id":attempt,"at":now(),"status":"running","phase":"environment_start","directory":evidence,"scenario_id":scenario_id,"records":[]}));
    let save = |state: &Value| -> Result<()> {
        write_json(
            &evidence.join("reproduction.json"),
            &state["reproductions"][index],
        )?;
        store.save(state)
    };
    save(state)?;
    let outcome = (|| -> Result<()> {
        runtime.start()?;
        for operation in &operations {
            state["reproductions"][index]["phase"] = json!("product_operation");
            state["reproductions"][index]["operation_id"] = operation["id"].clone();
            save(state)?;
            let scenario = scenarios
                .iter()
                .find(|s| s["id"] == operation["scenario_id"])
                .unwrap();
            let record = collect_operation(
                &runtime,
                operation,
                scenario,
                product::items(explorer, "services")?,
                config.test_timeout,
            )?;
            state["reproductions"][index]["records"]
                .as_array_mut()
                .unwrap()
                .push(record.clone());
            save(state)?;
            anyhow::ensure!(
                record["evidence"]["complete"] == true,
                "Product operation was blocked or interrupted; inspect the saved replay evidence"
            );
            gate.verify_worktree()?;
        }
        Ok(())
    })();
    if let Err(error) = crate::runtime::process::ignoring_interrupts(|| runtime.logs()) {
        state["reproductions"][index]["logs_error"] = json!(format!("{error:#}"));
    }
    state["reproductions"][index]["status"] = json!(if crate::runtime::process::interrupted() {
        "interrupted"
    } else if outcome.is_ok() {
        "completed"
    } else {
        "failed"
    });
    state["reproductions"][index]["finished_at"] = json!(now());
    if let Err(error) = &outcome {
        state["reproductions"][index]["error"] = json!(format!("{error:#}"));
    }
    save(state)?;
    let report = crate::reporting::refresh_report(directory, state);
    outcome?;
    report?;
    Ok(state["reproductions"][index]["records"].clone())
}

pub fn write_steps(directory: &Path, designer: &Value, revision: u64) -> Result<()> {
    let mut md = format!(
        "# 产品操作步骤\n\n文档版本：{revision}\n\n以下步骤先于执行保存；实际结果和证据见 [验证报告](report.md)。\n\n"
    );
    for scenario in product::items(designer, "scenarios")? {
        md.push_str(&format!(
            "## {}：{}\n\n功能：{}；入口：{}；类别：{}\n\n预期依据：{}\n\n",
            scenario["id"].as_str().unwrap(),
            scenario["title"].as_str().unwrap(),
            scenario["feature"].as_str().unwrap(),
            scenario["entry_point"].as_str().unwrap(),
            scenario["category"].as_str().unwrap(),
            scenario["oracle"].as_str().unwrap()
        ));
        for (key, label) in [("preconditions", "前置条件"), ("cleanup", "清理与恢复")] {
            if key == "cleanup" {
                continue;
            }
            md.push_str(&format!("### {label}\n\n"));
            for item in product::items(scenario, key)? {
                md.push_str(&format!("- {}\n", item.as_str().unwrap()));
            }
            md.push('\n');
        }
        if let Some(reason) = scenario["blocked_reason"].as_str() {
            md.push_str(&format!("受阻原因：{reason}\n\n"));
        }
        if let Some(source) = scenario["derived_from"].as_str() {
            md.push_str(&format!(
                "补充场景来源：{source}\n\n补充原因：{}\n\n",
                scenario["reason"].as_str().unwrap_or("")
            ));
        }
        for step in product::items(scenario, "steps")? {
            md.push_str(&format!(
                "### 步骤 {}\n\n操作：{}\n\n预期结果：{}\n\n",
                step["id"].as_str().unwrap(),
                step["action"].as_str().unwrap(),
                step["expected"].as_str().unwrap()
            ));
        }
        md.push_str("### 清理与恢复\n\n");
        for item in product::items(scenario, "cleanup")? {
            md.push_str(&format!("- {}\n", item.as_str().unwrap()));
        }
        md.push('\n');
    }
    write_json(
        &directory
            .join("documents")
            .join(format!("{revision:04}.json")),
        designer,
    )?;
    let saved = directory
        .join("documents")
        .join(format!("{revision:04}.md"));
    std::fs::write(saved, &md)?;
    std::fs::write(directory.join("steps.md"), md)?;
    Ok(())
}

pub fn collect_operation(
    runtime: &dyn TestRuntime,
    operation: &Value,
    scenario: &Value,
    services: &[Value],
    timeout: u64,
) -> Result<Value> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
    let remaining = || -> Result<u64> {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let seconds = left.as_secs() + u64::from(left.subsec_nanos() > 0);
        anyhow::ensure!(seconds > 0, "Product operation deadline exhausted");
        Ok(seconds)
    };
    let mut checks = Vec::new();
    for id in product::items(scenario, "services")? {
        let service = services
            .iter()
            .find(|s| s["id"] == *id)
            .context("Missing real service")?;
        let argv = crate::runtime::compose::strings(&service["check"]);
        let check_id = format!(
            "{}-service-{}",
            operation["id"].as_str().unwrap(),
            id.as_str().unwrap()
        );
        let mut check = if service["source"] == "external" {
            runtime.execute_test(&argv, &check_id, remaining()?)?
        } else {
            runtime.execute_service_check(
                &argv,
                service["compose_service"].as_str().unwrap(),
                &check_id,
                remaining()?,
            )?
        };
        check["service"] = id.clone();
        check["source"] = service["source"].clone();
        check["endpoint"] = service["endpoint"].clone();
        check["compose_service"] = service["compose_service"].clone();
        check["command"] = service["check"].clone();
        let usable = check["result"]["returncode"] == 0 && check["result"]["timed_out"] == false;
        if let Some(path) = check["directory"].as_str() {
            write_json(&Path::new(path).join("execution.json"), &check)?;
        }
        checks.push(check);
        if !usable {
            return Ok(
                json!({"operation_id":operation["id"],"scenario_id":scenario["id"],"step_id":operation["step_id"],"command":operation["command"],"purpose":operation["purpose"],"scenario_version":product::version(scenario),"target_executed":false,"required_services":scenario["services"],"service_checks":checks,"evidence":{"complete":false},"error":"Real-service protocol check failed; target was not executed"}),
            );
        }
    }
    let mut record = runtime.execute_test(
        &crate::runtime::compose::strings(&operation["command"]),
        operation["id"].as_str().unwrap(),
        remaining()?,
    )?;
    let complete = record["result"]["returncode"]
        .as_i64()
        .is_some_and(|code| code >= 0 && ![124, 125, 126, 127, 130, 137, 143].contains(&code))
        && record["result"]["timed_out"] == false;
    record["operation_id"] = operation["id"].clone();
    record["scenario_id"] = scenario["id"].clone();
    record["step_id"] = operation["step_id"].clone();
    record["command"] = operation["command"].clone();
    record["purpose"] = operation["purpose"].clone();
    record["scenario_version"] = json!(product::version(scenario));
    record["target_executed"] = json!(true);
    record["required_services"] = scenario["services"].clone();
    record["service_checks"] = json!(checks);
    record["evidence"] = json!({"complete":complete});
    write_json(
        &Path::new(
            record["directory"]
                .as_str()
                .context("Missing operation evidence directory")?,
        )
        .join("execution.json"),
        &record,
    )?;
    Ok(record)
}
