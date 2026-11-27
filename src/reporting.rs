use crate::{
    environment::strings,
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
                    .unwrap_or("No complete validation was available")
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
        "workspace":state["workspace"],"stages":state["stages"],"finish_reason":state["finish_reason"],"coordinator":{"directory":directory.join("stages/coordinator"),"tool_log":directory.join("stages/coordinator/tools.jsonl")},"scope":array(&artifacts["code-explorer"]["scope"]),"uncertainties":array(&artifacts["code-explorer"]["uncertainties"]),
        "excluded_areas":array(&artifacts["bug-hunter"]["excluded_areas"]),"candidates":candidates,"verdicts":verdicts,"execution":execution,"environment":state["environment"],
        "candidate_results":candidate_results,"environment_started":state["environment_started"].as_bool().unwrap_or(false),"environment_ready":state["environment_ready"].as_bool().unwrap_or(false),
        "environment_actions":array(&state["environment_actions"]),"dispatch_rejections":array(&state["dispatch_rejections"]),"execution_attempts":array(&state["execution_attempts"]),
        "environment_retained":state["environment_retained"].as_bool().unwrap_or(false),"environment_cleaned_at":state["environment_cleaned_at"],"tmux_cleaned_at":state["tmux_cleaned_at"],"reproductions":array(&state["reproductions"])
    });
    write_json(&directory.join("report.json"), &document)?;
    let mut md = format!(
        "# AI Agent QE report\n\nRun: `{}`\n\nStatus: **{}**\n\n## Request\n\n{}\n\n## Code and scope\n\n",
        text(&state["id"]),
        text(&state["status"]),
        text(&state["prompt"])
    );
    if state["workspace"].is_object() {
        let workspace = &state["workspace"];
        md.push_str(&format!(
            "Base commit: `{}`\n\nBranch: `{}`\n\nWorktree: `{}`\n\n",
            text(&workspace["commit"]),
            text(&workspace["branch"]),
            text(&workspace["path"])
        ));
    }
    for scope in strings(&document["scope"]) {
        md.push_str(&format!("- {scope}\n"));
    }
    if let Some(reason) = state["finish_reason"].as_str() {
        md.push_str(&format!("\nCoordinator finish reason: {reason}\n\n"));
    }
    if state["loop_started_at"].is_string() {
        md.push_str(&format!(
            "Launched workers: {}; rejected dispatches: {}\n\n",
            array(&state["stages"]).len(),
            array(&state["dispatch_rejections"]).len()
        ));
        md.push_str("## Stage attempts\n\n| Role | Attempt | Status | Instruction | Directory |\n|---|---|---|---|---|\n");
        for entry in array(&state["stages"]) {
            let clean = |v: &Value| text(v).replace('|', "\\|").replace(['\n', '\r'], " ");
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                clean(&entry["name"]),
                entry["attempt"],
                clean(&entry["status"]),
                clean(&entry["instruction"]),
                clean(&entry["directory"])
            ));
        }
        md.push_str("\nCoordinator tool decisions: `stages/coordinator/tools.jsonl`. Historical attempt artifacts remain in their stage directories.\n\n");
    }
    if !array(&document["dispatch_rejections"]).is_empty() {
        md.push_str("## Rejected dispatches\n\nThese requests did not launch workers or consume worker attempts.\n\n");
        for rejection in array(&document["dispatch_rejections"]) {
            md.push_str(&format!(
                "- `{}`: {}; recovery: `{}`\n",
                text(&rejection["stage"]),
                text(&rejection["error"]),
                rejection["recovery"]
            ));
        }
    }
    md.push_str("\n## Findings\n\n");
    if candidates.is_empty() {
        md.push_str("No candidates were produced in the examined scope. This does not establish complete product coverage.\n\n");
    }
    for (candidate, result) in candidates.iter().zip(&candidate_results) {
        let verdict = verdicts
            .iter()
            .find(|v| v["candidate_id"] == candidate["id"])
            .expect("candidate verdict generated");
        md.push_str(&format!("### {}: {}\n\nValidation: **{}**\n\nExpected: {}\n\nObserved: {}\n\nTrigger: {}\n\n{}\n\nEvidence: {}\n\n",text(&candidate["id"]),text(&candidate["title"]),text(&verdict["status"]),text(&candidate["expected"]),text(&candidate["observed"]),text(&candidate["trigger"]),text(&verdict["rationale"]),strings(&verdict["evidence"]).join(", ")));
        for plan in array(&result["plans"]) {
            md.push_str(&format!(
                "- Plan `{}`: design blocker: {}; execution prerequisite: {}\n",
                text(&plan["test_id"]),
                plan["blocked_reason"].as_str().unwrap_or("none"),
                plan["execution_blocked_reason"].as_str().unwrap_or("none")
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
                "- Test `{}`: generated: {}; executions: {}\n",
                text(&id),
                if generated { "yes" } else { "no" },
                executions.len()
            ));
            for record in executions {
                md.push_str(&format!(
                    "  - Exit {}; complete JUnit: {}; evidence: `{}/execution.json`\n",
                    record["result"]["returncode"],
                    record["junit"]["complete"],
                    text(&record["directory"])
                ));
            }
        }
        md.push('\n');
    }
    md.push_str("## Unresolved and coverage limits\n\n");
    for note in strings(&document["uncertainties"])
        .into_iter()
        .chain(strings(&document["excluded_areas"]))
    {
        md.push_str(&format!("- {note}\n"));
    }
    if let Some(error) = state["error"].as_str() {
        md.push_str(&format!("\n{error}\n"));
    }
    md.push_str("\n## Reproduction and environment\n\n");
    md.push_str(&format!(
        "Environment started: {}; ready: {}.\n\n",
        document["environment_started"], document["environment_ready"]
    ));
    for (key, heading) in [
        ("environment_actions", "Environment actions"),
        ("execution_attempts", "Execution attempts"),
    ] {
        if !array(&document[key]).is_empty() {
            md.push_str(&format!(
                "### {heading}\n\n```json\n{}\n```\n\n",
                serde_json::to_string_pretty(&document[key])?
            ));
        }
    }
    if state["environment_retained"] == true {
        md.push_str(
            "Docker resources are retained. External containers remain externally owned.\n\n",
        );
    }
    for (key, label) in [
        ("environment_cleaned_at", "Docker resources"),
        ("tmux_cleaned_at", "tmux sessions"),
    ] {
        if let Some(at) = state[key].as_str() {
            md.push_str(&format!("Owned {label} cleaned at {at}.\n\n"));
        }
    }
    if !array(&state["reproductions"]).is_empty() {
        md.push_str("### Reproduction attempts\n\n");
        for attempt in array(&state["reproductions"]) {
            md.push_str(&format!("Attempt: {}\n\n", text(&attempt["at"])));
            if !attempt["status"].is_null() {
                md.push_str(&format!(
                    "Status: {}; phase: {}; attempt record: `{}/reproduction.json`\n\n",
                    text(&attempt["status"]),
                    text(&attempt["phase"]),
                    text(&attempt["directory"])
                ));
            }
            if let Some(error) = attempt["error"].as_str() {
                md.push_str(&format!("Error: {error}\n\n"));
            }
            for preparation in array(&attempt["preparations"]) {
                md.push_str(&format!(
                    "- Preparation `{}`: exit {}; evidence: `{}/execution.json`\n",
                    text(&preparation["test_id"]),
                    preparation["result"]["returncode"],
                    text(&preparation["directory"])
                ));
            }
            for record in array(&attempt["records"]) {
                md.push_str(&format!(
                    "- `{}`: exit {}; evidence: `{}/execution.json`\n",
                    text(&record["test_id"]),
                    record["result"]["returncode"],
                    text(&record["directory"])
                ));
            }
            md.push('\n');
        }
    }
    let id = text(&state["id"]);
    md.push_str(&format!("`aaqe reproduce {id} --case <test-id>`\n\n`aaqe env status {id}`\n\n`aaqe cleanup {id}`\n\nUse the same --state-dir when a custom run directory was configured. Cleanup retains reports and the Git worktree.\n\nRaw stage artifacts, test patch, baseline and per-attempt process/JUnit records are stored beside this report.\n"));
    fs::write(directory.join("report.md"), md)?;
    Ok(document)
}
