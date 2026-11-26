use crate::{
    codex::Codex,
    config::Config,
    environment::{ComposeEnvironment, strings},
    evidence::parse_junit,
    patchgate::PatchGate,
    reporting::refresh_report,
    state::{Store, now, read_json, write_json},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path};

pub const STAGES: [&str; 5] = [
    "code-explorer",
    "bug-hunter",
    "test-designer",
    "test-developer",
    "test-validator",
];
pub trait AuditAgent {
    fn coordinate(&self, harness: &mut crate::harness::AuditHarness<'_>) -> Result<()>;
    fn invoke(
        &self,
        stage: &str,
        workspace: &Path,
        directory: &Path,
        inputs: &Value,
        progress: &dyn Fn(&str),
    ) -> Result<Value>;
}
impl AuditAgent for Codex {
    fn coordinate(&self, harness: &mut crate::harness::AuditHarness<'_>) -> Result<()> {
        Codex::coordinate(self, harness)
    }
    fn invoke(
        &self,
        stage: &str,
        workspace: &Path,
        directory: &Path,
        inputs: &Value,
        progress: &dyn Fn(&str),
    ) -> Result<Value> {
        Codex::invoke(self, stage, workspace, directory, inputs, progress)
    }
}
pub trait TestRuntime {
    fn start(&self) -> Result<()>;
    fn execute_test(&self, command: &[String], test_id: &str, timeout: u64) -> Result<Value>;
    fn logs(&self) -> Result<()>;
}
impl TestRuntime for ComposeEnvironment {
    fn start(&self) -> Result<()> {
        ComposeEnvironment::start(self)
    }
    fn execute_test(&self, command: &[String], test_id: &str, timeout: u64) -> Result<Value> {
        ComposeEnvironment::execute_test(self, command, test_id, timeout)
    }
    fn logs(&self) -> Result<()> {
        ComposeEnvironment::logs(self)
    }
}
pub type RuntimeFactory<'a> = dyn Fn(&Path, &Path, &str, Value, u64) -> Box<dyn TestRuntime> + 'a;

pub fn collect_test(runtime: &dyn TestRuntime, test: &Value, timeout: u64) -> Result<Value> {
    let mut record = runtime.execute_test(
        &strings(&test["command"]),
        test["id"].as_str().context("Missing test identity")?,
        timeout,
    )?;
    let report = Path::new(
        record["directory"]
            .as_str()
            .context("Missing execution directory")?,
    )
    .join(test["report"].as_str().context("Missing JUnit report")?);
    record["candidate_id"] = test["candidate_id"].clone();
    record["junit"] = parse_junit(&report, &strings(&test["expected_names"]));
    let failed = record["junit"]["cases"]
        .as_array()
        .is_some_and(|cases| cases.iter().any(|c| c["status"] == "failed"));
    let code = record["result"]["returncode"].as_i64().unwrap_or(127);
    let error = if record["result"]["timed_out"] == true
        || [124, 125, 126, 127, 137, 143].contains(&code)
        || code < 0
    {
        Some("execution_interrupted_or_start_failed")
    } else if (code != 0) != failed {
        Some("process_and_target_results_disagree")
    } else {
        None
    };
    if let Some(error) = error {
        record["junit"]["complete"] = json!(false);
        record["junit"]["errors"]
            .as_array_mut()
            .context("Invalid JUnit result")?
            .push(json!(error));
    }
    write_json(
        &Path::new(record["directory"].as_str().unwrap()).join("execution.json"),
        &record,
    )?;
    Ok(record)
}

pub fn grounded_verdicts(artifact: &Value, execution: &Value) -> Value {
    let empty = Vec::new();
    let records = execution["tests"].as_array().unwrap_or(&empty);
    let baseline = execution["baseline"].as_array().unwrap_or(&empty);

    let mut verdicts = Vec::new();
    for original in artifact["verdicts"].as_array().unwrap_or(&empty) {
        let mut verdict = original.clone();
        let ids = strings(&verdict["test_ids"]);
        let supporting: Vec<&Value> = ids
            .iter()
            .filter_map(|id| records.iter().find(|r| r["test_id"].as_str() == Some(id)))
            .collect();
        let complete = !ids.is_empty()
            && supporting.len() == ids.len()
            && supporting.iter().all(|r| r["junit"]["complete"] == true);
        let healthy = ids.iter().all(|id| {
            let applicable: Vec<_> = baseline
                .iter()
                .filter(|r| r["for_test_id"].is_null() || r["for_test_id"].as_str() == Some(id))
                .collect();
            !applicable.is_empty()
                && applicable
                    .iter()
                    .all(|r| r["result"]["returncode"] == 0 && r["result"]["timed_out"] != true)
        });
        let failed = supporting.iter().any(|r| {
            r["junit"]["cases"]
                .as_array()
                .is_some_and(|cases| cases.iter().any(|c| c["status"] == "failed"))
        });
        let allowed: HashSet<String> = supporting
            .iter()
            .filter_map(|r| r["directory"].as_str())
            .map(|dir| {
                Path::new(dir)
                    .join("execution.json")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        if verdict["status"] == "confirmed" || verdict["status"] == "not_reproduced" {
            let evidence = strings(&verdict["evidence"]);
            let valid = complete
                && healthy
                && !evidence.is_empty()
                && evidence.iter().all(|p| allowed.contains(p))
                && (if verdict["status"] == "confirmed" {
                    failed
                } else {
                    !failed
                });
            if !valid {
                verdict["agent_status"] = verdict["status"].clone();
                verdict["status"] = json!("blocked");
                verdict["rationale"] = json!(format!(
                    "Execution gate: incomplete, inconsistent or absent target/baseline evidence. Agent assessment: {}",
                    verdict["rationale"].as_str().unwrap_or("")
                ));
            }
        }
        verdict["execution_records"] = json!(
            supporting
                .iter()
                .filter_map(|r| r["directory"].as_str())
                .map(|p| Path::new(p)
                    .join("execution.json")
                    .to_string_lossy()
                    .into_owned())
                .collect::<Vec<_>>()
        );
        verdicts.push(verdict);
    }
    json!(verdicts)
}

pub fn run_audit(
    prompt: &str,
    caller: &Path,
    config: &Config,
    progress: &dyn Fn(&str),
) -> Result<Value> {
    let agent = Codex::new(config);
    run_audit_with(
        prompt,
        caller,
        config,
        &agent,
        &|workspace, directory, id, spec, timeout| {
            Box::new(ComposeEnvironment::new(
                workspace, directory, id, spec, timeout,
            ))
        },
        progress,
    )
}

pub fn run_audit_with(
    prompt: &str,
    caller: &Path,
    config: &Config,
    agent: &dyn AuditAgent,
    factory: &RuntimeFactory<'_>,
    progress: &dyn Fn(&str),
) -> Result<Value> {
    crate::harness::audit(prompt, caller, config, agent, factory, progress)
}

pub fn reproduce(store: &Store, state: &mut Value, case_id: Option<&str>) -> Result<Value> {
    let directory = store.path(state["id"].as_str().context("Missing run identity")?)?;
    ensure!(
        state["environment_cleaned_at"].is_null(),
        "This run's environment has already been cleaned"
    );
    ensure!(
        state["environment"].is_object(),
        "This run has no test environment"
    );
    let config: Config = serde_json::from_value(state["config"].clone())?;
    let artifact = read_json(&directory.join("artifacts/test-developer.json"))?;
    let tests: Vec<Value> = artifact["tests"]
        .as_array()
        .context("Missing saved tests")?
        .iter()
        .filter(|test| case_id.is_none_or(|id| test["id"].as_str() == Some(id)))
        .cloned()
        .collect();
    ensure!(!tests.is_empty(), "Unknown test ID");
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
        &directory,
        state["id"].as_str().unwrap(),
        state["environment"].clone(),
        config.docker_timeout,
    );
    let mut records = Vec::new();
    for test in tests {
        for (index, command) in test["reproduction_commands"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let preparation = runtime.execute_test(
                &strings(command),
                &format!("{}-prepare-{}", test["id"].as_str().unwrap(), index + 1),
                config.test_timeout,
            )?;
            ensure!(
                preparation["result"]["returncode"] == 0,
                "Reproduction preparation failed; original evidence retained"
            );
        }
        records.push(collect_test(&runtime, &test, config.test_timeout)?);
        gate.verify_worktree()?;
    }
    if !state["reproductions"].is_array() {
        state["reproductions"] = json!([]);
    }
    state["reproductions"]
        .as_array_mut()
        .unwrap()
        .push(json!({"at":now(),"records":records}));
    store.save(state)?;
    refresh_report(&directory, state)?;
    Ok(json!(records))
}
