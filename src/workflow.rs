use crate::{
    codex::Codex,
    config::Config,
    contracts::{validate, validate_links},
    environment::{ComposeEnvironment, strings},
    evidence::parse_junit,
    gitops::create_workspace,
    patchgate::{PatchGate, safe_path},
    process::{ignoring_interrupts, interrupted, run},
    reporting::{generate_report, refresh_report},
    state::{Store, now, read_json, write_json},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::HashSet, fs, path::Path};

pub const STAGES: [&str; 5] = [
    "code-explorer",
    "bug-hunter",
    "test-designer",
    "test-developer",
    "test-validator",
];
pub trait AuditAgent {
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
    let healthy = !baseline.is_empty() && baseline.iter().all(|r| r["result"]["returncode"] == 0);
    let mut verdicts = Vec::new();
    for original in artifact["verdicts"].as_array().unwrap_or(&empty) {
        let mut verdict = original.clone();
        let supporting: Vec<&Value> = strings(&verdict["test_ids"])
            .iter()
            .filter_map(|id| records.iter().find(|r| r["test_id"].as_str() == Some(id)))
            .collect();
        let complete =
            !supporting.is_empty() && supporting.iter().all(|r| r["junit"]["complete"] == true);
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
    ensure!(!prompt.trim().is_empty(), "An audit prompt is required");
    let store = Store::new(&config.state_dir);
    let mut state = store.create(prompt, config, caller)?;
    let id = state["id"]
        .as_str()
        .context("Missing run identity")?
        .to_owned();
    let directory = store.path(&id)?;
    let mut artifacts = json!({});
    let mut execution = json!({"baseline":[],"tests":[]});
    let mut verdicts = json!([]);
    let mut runtime: Option<Box<dyn TestRuntime>> = None;
    state["environment_retained"] = json!(false);
    let outcome = (|| -> Result<()> {
        progress(&format!(
            "Run {id}: Stage 0 — fetch {}/{} and create worktree",
            config.remote, config.branch
        ));
        let workspace = create_workspace(caller, &directory, config, &id)?;
        state["workspace"] = workspace.clone();
        let root = Path::new(
            workspace["path"]
                .as_str()
                .context("Missing worktree path")?,
        );
        let commit = workspace["commit"]
            .as_str()
            .context("Missing base commit")?;
        let gate = PatchGate::new(root, &config.test_paths, Some(commit))?;
        state["status"] = json!("running");
        store.save(&state)?;
        for (index, stage) in STAGES.iter().enumerate() {
            ensure!(!interrupted(), "Interrupted by user");
            let number = index + 1;
            state["stage"] = json!(number);
            state["stages"]
                .as_array_mut()
                .unwrap()
                .push(json!({"number":number,"name":stage,"status":"running","started_at":now()}));
            store.save(&state)?;
            progress(&format!("Stage {number} — {stage}"));
            let inputs = json!({"prompt":prompt,"base_commit":commit,"project_config":config,"artifacts":artifacts,"execution":if *stage=="test-validator" {execution.clone()} else {json!({})},"artifact_directory":directory});
            let output = agent.invoke(
                stage,
                root,
                &directory.join("stages").join(stage),
                &inputs,
                progress,
            )?;
            validate(stage, &output)?;
            validate_links(stage, &output, &artifacts)?;
            if *stage == "code-explorer" {
                for reference in output["references"].as_array().unwrap() {
                    let relative = reference["path"].as_str().unwrap();
                    let source = safe_path(root, relative)?;
                    ensure!(
                        source.is_file(),
                        "Source reference does not exist: {relative}"
                    );
                    let bytes = fs::read(source)?;
                    ensure!(
                        reference["line"]
                            .as_f64()
                            .context("Invalid source reference line")?
                            <= String::from_utf8_lossy(&bytes).lines().count() as f64,
                        "Source reference line is outside the file: {}",
                        reference["id"].as_str().unwrap()
                    );
                }
            }
            gate.verify_worktree()?;
            artifacts[*stage] = output.clone();
            write_json(
                &directory.join("artifacts").join(format!("{stage}.json")),
                &output,
            )?;
            let entry = &mut state["stages"][index];
            entry["status"] = json!("completed");
            entry["finished_at"] = json!(now());
            store.save(&state)?;
            if *stage == "bug-hunter" && output["candidates"].as_array().unwrap().is_empty() {
                break;
            }
            if *stage == "test-designer"
                && output["plans"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|p| p["blocked_reason"].as_str().is_some_and(|s| !s.is_empty()))
            {
                state["error"] = json!("All test plans are blocked; see test-designer artifact");
                state["status"] = json!("blocked");
                break;
            }
            if *stage == "test-developer" {
                let patch = output["patch"].as_str().unwrap();
                fs::write(directory.join("tests.diff"), patch)?;
                let changed = gate.apply(patch)?;
                write_json(
                    &directory.join("patch-check.json"),
                    &json!({"accepted":true,"paths":changed}),
                )?;
                let mut environment = artifacts["code-explorer"]["environment"].clone();
                if let Some(overrides) = config.environment.as_object() {
                    environment
                        .as_object_mut()
                        .unwrap()
                        .extend(overrides.clone());
                }
                state["environment"] = environment.clone();
                runtime = Some(factory(
                    root,
                    &directory,
                    &id,
                    environment,
                    config.docker_timeout,
                ));
                let runtime = runtime.as_ref().unwrap();
                state["environment_retained"] = json!(true);
                store.save(&state)?;
                progress("Stage 5 — start Docker environment and collect actual test evidence");
                runtime.start()?;
                let baseline: Vec<Vec<String>> = if config.baseline.is_empty() {
                    serde_json::from_value(artifacts["code-explorer"]["baseline"].clone())?
                } else {
                    config.baseline.clone()
                };
                if !baseline.is_empty() {
                    let reversed = run(
                        &["git".into(), "apply".into(), "--reverse".into(), "-".into()],
                        Some(root),
                        600,
                        Some(patch),
                    );
                    ensure!(
                        reversed.returncode == 0,
                        "Cannot restore the base code for baseline execution"
                    );
                    let baseline_outcome = (|| -> Result<()> {
                        for (index, command) in baseline.iter().enumerate() {
                            let record = runtime.execute_test(
                                command,
                                &format!("baseline-{}", index + 1),
                                config.test_timeout,
                            )?;
                            let code = record["result"]["returncode"].as_i64().unwrap_or(127);
                            execution["baseline"].as_array_mut().unwrap().push(record);
                            write_json(&directory.join("execution.json"), &execution)?;
                            ensure!(
                                code == 0,
                                "Baseline command failed; target verification is blocked"
                            );
                        }
                        Ok(())
                    })();
                    // Reapply even after a failing baseline, retaining generated tests for reproduction.
                    let restore = ignoring_interrupts(|| gate.apply(patch));
                    baseline_outcome?;
                    restore?;
                }
                for test in output["tests"].as_array().unwrap() {
                    execution["tests"]
                        .as_array_mut()
                        .unwrap()
                        .push(collect_test(runtime.as_ref(), test, config.test_timeout)?);
                    write_json(&directory.join("execution.json"), &execution)?;
                    gate.verify_worktree()?;
                }
            }
            if *stage == "test-validator" {
                verdicts = grounded_verdicts(&output, &execution);
            }
        }
        if state["status"] == "running" {
            state["status"] = json!("completed");
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        if interrupted() {
            state["status"] = json!("interrupted");
            state["error"] = json!("Interrupted by user; existing resources and evidence retained");
        } else {
            state["status"] = json!(if error.chain().any(|e| e.is::<std::io::Error>()) {
                "failed"
            } else {
                "blocked"
            });
            state["error"] = json!(format!("{error:#}"));
            if let Some(last) = state["stages"].as_array_mut().and_then(|s| s.last_mut())
                && last["status"] == "running"
            {
                last["status"] = json!("failed");
                last["finished_at"] = json!(now());
            }
        }
    }
    if let Some(runtime) = runtime
        && let Err(error) = ignoring_interrupts(|| runtime.logs())
    {
        state["log_collection_error"] = json!(format!("{error:#}"));
    }
    state["stage"] = json!(6);
    state["finished_at"] = json!(now());
    write_json(&directory.join("execution.json"), &execution)?;
    generate_report(&directory, &state, &artifacts, &execution, &verdicts)?;
    store.save(&state)?;
    progress(&format!(
        "Stage 6 — {}; report: {}",
        state["status"].as_str().unwrap_or("failed"),
        directory.join("report.md").display()
    ));
    Ok(state)
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
