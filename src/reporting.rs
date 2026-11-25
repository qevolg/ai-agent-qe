use crate::{
    environment::strings,
    state::{now, read_json, write_json},
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
    let mut verdicts = array(verdicts);
    for candidate in &candidates {
        if !verdicts
            .iter()
            .any(|v| v["candidate_id"] == candidate["id"])
        {
            verdicts.push(json!({"candidate_id":candidate["id"],"test_ids":[],"status":"blocked","rationale":state["error"].as_str().filter(|s|!s.is_empty()).unwrap_or("No complete validation was available"),"evidence":[]}));
        }
    }
    let document = json!({
        "schema_version":"1.0","run_id":state["id"],"generated_at":now(),"prompt":state["prompt"],"status":state["status"],"error":state["error"],
        "workspace":state["workspace"],"stages":state["stages"],"scope":array(&artifacts["code-explorer"]["scope"]),"uncertainties":array(&artifacts["code-explorer"]["uncertainties"]),
        "excluded_areas":array(&artifacts["bug-hunter"]["excluded_areas"]),"candidates":candidates,"verdicts":verdicts,"execution":execution,"environment":state["environment"],
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
    md.push_str("\n## Findings\n\n");
    if candidates.is_empty() {
        md.push_str("No candidates were produced in the examined scope. This does not establish complete product coverage.\n\n");
    }
    for candidate in &candidates {
        let verdict = verdicts
            .iter()
            .find(|v| v["candidate_id"] == candidate["id"])
            .expect("candidate verdict generated");
        md.push_str(&format!("### {}: {}\n\nValidation: **{}**\n\nExpected: {}\n\nObserved: {}\n\nTrigger: {}\n\n{}\n\nEvidence: {}\n\n",text(&candidate["id"]),text(&candidate["title"]),text(&verdict["status"]),text(&candidate["expected"]),text(&candidate["observed"]),text(&candidate["trigger"]),text(&verdict["rationale"]),strings(&verdict["evidence"]).join(", ")));
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
