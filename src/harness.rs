//! Persistent guardrails and evidence; the Codex coordinator chooses the next role.
use crate::{
    config::Config,
    contracts::{validate, validate_links},
    gitops::create_workspace,
    patchgate::{PatchGate, safe_path},
    process::{ignoring_interrupts, interrupted, run},
    reporting::generate_report,
    state::{Store, now, read_json, write_json},
    workflow::{AuditAgent, RuntimeFactory, STAGES, collect_test, grounded_verdicts},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct AuditHarness<'a> {
    pub directory: PathBuf,
    pub config: Config,
    state: Value,
    artifacts: Value,
    execution: Value,
    verdicts: Value,
    agent: &'a dyn AuditAgent,
    factory: &'a RuntimeFactory<'a>,
    progress: &'a dyn Fn(&str),
}

impl<'a> AuditHarness<'a> {
    pub fn open(
        directory: &Path,
        agent: &'a dyn AuditAgent,
        factory: &'a RuntimeFactory<'a>,
        progress: &'a dyn Fn(&str),
    ) -> Result<Self> {
        let saved = read_json(&directory.join("harness.json"))?;
        Ok(Self {
            directory: directory.to_owned(),
            config: serde_json::from_value(saved["state"]["config"].clone())?,
            state: saved["state"].clone(),
            artifacts: saved["artifacts"].clone(),
            execution: saved["execution"].clone(),
            verdicts: saved["verdicts"].clone(),
            agent,
            factory,
            progress,
        })
    }

    pub fn reload(&mut self) -> Result<()> {
        let saved = read_json(&self.directory.join("harness.json"))?;
        self.state = saved["state"].clone();
        self.artifacts = saved["artifacts"].clone();
        self.execution = saved["execution"].clone();
        self.verdicts = saved["verdicts"].clone();
        Ok(())
    }

    fn save(&self) -> Result<()> {
        write_json(&self.directory.join("harness.json"), &self.status())?;
        write_json(&self.directory.join("run.json"), &self.state)?;
        write_json(&self.directory.join("execution.json"), &self.execution)?;
        Ok(())
    }

    pub fn status(&self) -> Value {
        json!({"state":self.state,"workspace":self.state["workspace"],"artifacts":self.artifacts,"execution":self.execution,"verdicts":self.verdicts,"allowed_stages":STAGES,"budgets":{"stage_calls":self.config.max_stage_calls,"attempts_per_stage":self.config.max_stage_attempts,"timeout":self.config.run_timeout}})
    }

    fn root(&self) -> Result<PathBuf> {
        Ok(PathBuf::from(
            self.state["workspace"]["path"]
                .as_str()
                .context("Missing audit worktree")?,
        ))
    }

    fn gate(&self) -> Result<PatchGate> {
        PatchGate::new(
            &self.root()?,
            &self.config.test_paths,
            self.state["workspace"]["commit"].as_str(),
        )
    }

    fn reverse(&self, patch: &str) -> Result<()> {
        let result = run(
            &["git".into(), "apply".into(), "--reverse".into(), "-".into()],
            Some(&self.root()?),
            600,
            Some(patch),
        );
        ensure!(
            result.returncode == 0,
            "Cannot restore recorded base for patch replacement/baseline: {}",
            result.stderr
        );
        Ok(())
    }

    fn invalidate(&mut self, index: usize) -> Result<()> {
        for role in &STAGES[index + 1..] {
            self.artifacts.as_object_mut().unwrap().remove(*role);
            let file = self
                .directory
                .join("artifacts")
                .join(format!("{role}.json"));
            if file.exists() {
                fs::remove_file(file)?;
            }
        }
        if index < 3 {
            if let Some(patch) = self.state["applied_patch"].as_str().map(str::to_owned) {
                self.reverse(&patch)?;
                self.state["applied_patch"] = Value::Null;
            }
            for file in ["tests.diff", "patch-check.json"] {
                let path = self.directory.join(file);
                if path.exists() {
                    fs::remove_file(path)?;
                }
            }
        }
        if index <= 3 {
            self.execution = json!({"baseline":[],"tests":[]});
            self.verdicts = json!([]);
        }
        Ok(())
    }

    pub fn run_stage(&mut self, stage: &str, instruction: &str) -> Result<Value> {
        ensure!(
            self.state["status"] == "running",
            "Audit is already finished"
        );
        ensure!(!interrupted(), "Interrupted by user");
        ensure!(
            !instruction.trim().is_empty(),
            "A dispatch instruction is required"
        );
        let index = STAGES
            .iter()
            .position(|role| *role == stage)
            .context("Unknown stage")?;
        for prerequisite in &STAGES[..index] {
            ensure!(
                self.artifacts.get(*prerequisite).is_some(),
                "Missing prerequisite: {prerequisite}"
            );
        }
        if index >= 2 {
            ensure!(
                !self.artifacts["bug-hunter"]["candidates"]
                    .as_array()
                    .context("Missing candidates")?
                    .is_empty(),
                "No candidates require testing"
            );
        }
        if stage == "test-developer" {
            ensure!(
                self.artifacts["test-designer"]["plans"]
                    .as_array()
                    .context("Missing test plans")?
                    .iter()
                    .any(|p| p["blocked_reason"].is_null() || p["blocked_reason"] == ""),
                "All test plans are blocked"
            );
        }
        let entries = self.state["stages"]
            .as_array()
            .context("Missing stage history")?;
        ensure!(
            (entries.len() as u64) < self.config.max_stage_calls,
            "Stage call budget exhausted"
        );
        let attempt = entries.iter().filter(|e| e["name"] == stage).count() as u64 + 1;
        ensure!(
            attempt <= self.config.max_stage_attempts,
            "Stage attempt budget exhausted for {stage}"
        );
        let elapsed = chrono::Utc::now()
            .signed_duration_since(chrono::DateTime::parse_from_rfc3339(
                self.state["loop_started_at"]
                    .as_str()
                    .context("Missing start time")?,
            )?)
            .num_seconds();
        ensure!(
            elapsed < self.config.run_timeout as i64,
            "Audit time budget exhausted"
        );
        let mut directory = self.directory.join("stages").join(stage);
        if attempt > 1 {
            directory = directory.join("attempts").join(format!("{attempt:04}"));
        }
        fs::create_dir_all(&directory)?;
        let entry = self.state["stages"].as_array().unwrap().len();
        self.state["stage"] = json!(index + 1);
        self.state["stages"].as_array_mut().unwrap().push(json!({"number":index+1,"name":stage,"attempt":attempt,"directory":directory,"instruction":instruction,"status":"running","started_at":now()}));
        self.save()?;
        (self.progress)(&format!("Dispatch {stage}, attempt {attempt}"));
        let inputs = json!({"stage":stage,"attempt":attempt,"instruction":instruction,"prompt":self.state["prompt"],"base_commit":self.state["workspace"]["commit"],"project_config":self.config,"artifacts":self.artifacts,"execution":self.execution,"previous_attempts":self.state["stages"],"artifact_directory":self.directory});
        // The harness also saves boundary inputs/results for non-Codex test agents.
        write_json(&directory.join("input.json"), &inputs)?;
        let outcome = self.accept(stage, index, &directory, &inputs);
        let result = match outcome {
            Ok(output) => {
                json!({"status":"completed","stage":stage,"attempt":attempt,"directory":directory,"artifact":output,"execution":self.execution,"verdicts":self.verdicts})
            }
            Err(error) => {
                json!({"status":"failed","stage":stage,"attempt":attempt,"directory":directory,"error":format!("{error:#}"),"execution":self.execution})
            }
        };
        self.state["stages"][entry]["status"] = result["status"].clone();
        self.state["stages"][entry]["finished_at"] = json!(now());
        self.state["stages"][entry]["error"] = result["error"].clone();
        write_json(&directory.join("result.json"), &result)?;
        if stage == "test-developer" && directory.join("accepted.json").exists() {
            write_json(&directory.join("execution.json"), &result["execution"])?;
        }
        self.save()?;
        Ok(result)
    }

    fn accept(
        &mut self,
        stage: &str,
        index: usize,
        directory: &Path,
        inputs: &Value,
    ) -> Result<Value> {
        let root = self.root()?;
        let output = self
            .agent
            .invoke(stage, &root, directory, inputs, self.progress)?;
        write_json(&directory.join("output.json"), &output)?;
        validate(stage, &output)?;
        validate_links(stage, &output, &self.artifacts)?;
        if stage == "code-explorer" {
            for reference in output["references"].as_array().unwrap() {
                let relative = reference["path"].as_str().unwrap();
                let source = safe_path(&root, relative)?;
                ensure!(
                    source.is_file(),
                    "Source reference does not exist: {relative}"
                );
                let bytes = fs::read(source)?;
                ensure!(
                    reference["line"]
                        .as_f64()
                        .context("Invalid reference line")?
                        <= String::from_utf8_lossy(&bytes).lines().count() as f64,
                    "Source reference line is outside the file: {relative}"
                );
            }
        }
        let gate = self.gate()?;
        gate.verify_worktree()?;
        if stage == "test-developer" {
            let patch = output["patch"].as_str().unwrap();
            let previous = self.state["applied_patch"].as_str().map(str::to_owned);
            if let Some(old) = &previous {
                self.reverse(old)?;
            }
            let applied = gate.apply(patch);
            if applied.is_err()
                && let Some(old) = &previous
            {
                ignoring_interrupts(|| gate.apply(old))
                    .context("Cannot restore previous accepted test patch")?;
            }
            let changed = applied?;
            self.state["applied_patch"] = json!(patch);
            fs::write(self.directory.join("tests.diff"), patch)?;
            write_json(
                &self.directory.join("patch-check.json"),
                &json!({"accepted":true,"paths":changed}),
            )?;
        }
        self.invalidate(index)?;
        self.artifacts[stage] = output.clone();
        write_json(&directory.join("accepted.json"), &output)?;
        write_json(
            &self
                .directory
                .join("artifacts")
                .join(format!("{stage}.json")),
            &output,
        )?;
        self.save()?;
        if stage == "test-developer" {
            self.execute(&output, directory)?;
        }
        if stage == "test-validator" {
            self.verdicts = grounded_verdicts(&output, &self.execution);
        }
        Ok(output)
    }

    fn execute(&mut self, output: &Value, attempt: &Path) -> Result<()> {
        let root = self.root()?;
        let mut environment = self.artifacts["code-explorer"]["environment"].clone();
        if let Some(overrides) = self.config.environment.as_object() {
            environment
                .as_object_mut()
                .unwrap()
                .extend(overrides.clone());
        }
        // One owned Compose snapshot per run; a revision cannot orphan the old stack.
        ensure!(
            self.state["environment"].is_null() || self.state["environment"] == environment,
            "Environment changed after startup; retain this run and start a new audit for the new environment"
        );
        self.state["environment"] = environment.clone();
        let runtime = (self.factory)(
            &root,
            &self.directory,
            self.state["id"].as_str().unwrap(),
            environment,
            self.config.docker_timeout,
        );
        let started = self.state["environment_started"] == true;
        self.state["environment_retained"] = json!(true);
        self.save()?;
        let outcome = (|| -> Result<()> {
            if !started {
                runtime.start()?;
                self.state["environment_started"] = json!(true);
                self.save()?;
            }
            let baseline: Vec<Vec<String>> = if self.config.baseline.is_empty() {
                serde_json::from_value(self.artifacts["code-explorer"]["baseline"].clone())?
            } else {
                self.config.baseline.clone()
            };
            let patch = output["patch"].as_str().unwrap();
            if !baseline.is_empty() {
                self.state["baseline_restore_patch"] = json!(patch);
                self.save()?;
                self.reverse(patch)?;
                let baseline_outcome = (|| -> Result<()> {
                    for (index, command) in baseline.iter().enumerate() {
                        let record = runtime.execute_test(
                            command,
                            &format!("baseline-{}", index + 1),
                            self.config.test_timeout,
                        )?;
                        let code = record["result"]["returncode"].as_i64().unwrap_or(127);
                        self.execution["baseline"]
                            .as_array_mut()
                            .unwrap()
                            .push(record);
                        self.save_execution(attempt)?;
                        ensure!(
                            code == 0,
                            "Baseline command failed; target verification is blocked"
                        );
                    }
                    Ok(())
                })();
                let restore = ignoring_interrupts(|| self.gate()?.apply(patch));
                restore?;
                self.state["baseline_restore_patch"] = Value::Null;
                self.save()?;
                baseline_outcome?;
            }
            for test in output["tests"].as_array().unwrap() {
                let record = collect_test(runtime.as_ref(), test, self.config.test_timeout)?;
                self.execution["tests"].as_array_mut().unwrap().push(record);
                self.save_execution(attempt)?;
                self.gate()?.verify_worktree()?;
            }
            Ok(())
        })();
        if let Err(error) = ignoring_interrupts(|| runtime.logs()) {
            self.state["log_collection_error"] = json!(format!("{error:#}"));
        }
        outcome
    }

    fn save_execution(&self, attempt: &Path) -> Result<()> {
        write_json(&attempt.join("execution.json"), &self.execution)?;
        self.save()
    }

    fn recover_baseline(&mut self) -> Result<()> {
        if let Some(patch) = self.state["baseline_restore_patch"]
            .as_str()
            .map(str::to_owned)
        {
            let root = self.root()?;
            let applied = ignoring_interrupts(|| {
                run(
                    &[
                        "git".into(),
                        "apply".into(),
                        "--reverse".into(),
                        "--check".into(),
                        "-".into(),
                    ],
                    Some(&root),
                    600,
                    Some(&patch),
                )
            });
            if applied.returncode != 0 {
                ignoring_interrupts(|| self.gate()?.apply(&patch))
                    .context("Cannot recover generated patch after coordinator loss")?;
            }
            self.state["baseline_restore_patch"] = Value::Null;
            self.save()?;
        }
        Ok(())
    }

    pub fn finish(&mut self, status: &str, reason: &str) -> Result<()> {
        ensure!(
            self.state["status"] == "running",
            "Audit is already finished"
        );
        ensure!(!reason.trim().is_empty(), "A finish reason is required");
        ensure!(
            ["completed", "blocked"].contains(&status),
            "Finish status must be completed or blocked"
        );
        if status == "completed" {
            for stage in &STAGES[..2] {
                ensure!(
                    self.artifacts.get(*stage).is_some(),
                    "Missing completion prerequisite: {stage}"
                );
            }
            if !self.artifacts["bug-hunter"]["candidates"]
                .as_array()
                .unwrap()
                .is_empty()
            {
                for stage in &STAGES[2..] {
                    ensure!(
                        self.artifacts.get(*stage).is_some(),
                        "Missing independent completion prerequisite: {stage}"
                    );
                }
            }
            self.gate()?.verify_worktree()?;
        }
        self.state["status"] = json!(status);
        self.state["finish_reason"] = json!(reason);
        if status == "blocked" {
            self.state["error"] = json!(reason);
        }
        self.save()
    }
}

pub fn audit(
    prompt: &str,
    caller: &Path,
    config: &Config,
    agent: &dyn AuditAgent,
    factory: &RuntimeFactory<'_>,
    progress: &dyn Fn(&str),
) -> Result<Value> {
    ensure!(!prompt.trim().is_empty(), "An audit prompt is required");
    let store = Store::new(&config.state_dir);
    let state = store.create(prompt, config, caller)?;
    let id = state["id"].as_str().unwrap().to_owned();
    let directory = store.path(&id)?;
    let mut harness = AuditHarness {
        directory,
        config: config.clone(),
        state,
        artifacts: json!({}),
        execution: json!({"baseline":[],"tests":[]}),
        verdicts: json!([]),
        agent,
        factory,
        progress,
    };
    harness.state["environment_retained"] = json!(false);
    harness.save()?;
    let outcome = (|| -> Result<()> {
        progress(&format!(
            "Run {id}: fetch {}/{} and create worktree",
            config.remote, config.branch
        ));
        harness.state["workspace"] = create_workspace(caller, &harness.directory, config, &id)?;
        harness.state["status"] = json!("running");
        harness.state["loop_started_at"] = json!(now());
        harness.save()?;
        let result = agent.coordinate(&mut harness);
        harness.reload()?;
        result?;
        ensure!(
            harness.state["status"] != "running",
            "Coordinator exited without calling finish; audit is incomplete"
        );
        Ok(())
    })();
    if let Err(error) = outcome {
        harness.state["status"] = json!(if interrupted() {
            "interrupted"
        } else if error.chain().any(|e| e.is::<std::io::Error>()) {
            "failed"
        } else {
            "blocked"
        });
        harness.state["error"] = json!(format!("{error:#}"));
    }
    if let Err(error) = harness.recover_baseline() {
        harness.state["status"] = json!("blocked");
        harness.state["recovery_error"] = json!(format!("{error:#}"));
    }
    if interrupted() {
        harness.state["status"] = json!("interrupted");
    }
    for entry in harness.state["stages"].as_array_mut().unwrap() {
        if entry["status"] == "running" {
            entry["status"] = json!("failed");
            entry["finished_at"] = json!(now());
        }
    }
    harness.state["stage"] = json!(6);
    harness.state["finished_at"] = json!(now());
    harness.save()?;
    generate_report(
        &harness.directory,
        &harness.state,
        &harness.artifacts,
        &harness.execution,
        &harness.verdicts,
    )?;
    progress(&format!(
        "{}; report: {}",
        harness.state["status"].as_str().unwrap(),
        harness.directory.join("report.md").display()
    ));
    Ok(harness.state)
}
