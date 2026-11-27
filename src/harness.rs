//! Persistent guardrails and evidence; the Codex coordinator chooses the next role.
use crate::{
    config::Config,
    contracts::{validate, validate_links},
    gitops::create_workspace,
    patchgate::{PatchGate, safe_path},
    process::{ignoring_interrupts, interrupted, run},
    reporting::generate_report,
    state::{HarnessSnapshot, RunLock, Store, now, read_json, write_json},
    workflow::{AuditAgent, RuntimeFactory, STAGES, collect_test, grounded_verdicts},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

mod environment;
mod inputs;
pub(crate) use inputs::evidence_view;

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
        let saved = HarnessSnapshot::load(directory)?;
        Ok(Self {
            directory: directory.to_owned(),
            config: serde_json::from_value(saved.state["config"].clone())?,
            state: saved.state,
            artifacts: saved.artifacts,
            execution: saved.execution,
            verdicts: saved.verdicts,
            agent,
            factory,
            progress,
        })
    }

    pub fn reload(&mut self) -> Result<()> {
        let saved = HarnessSnapshot::load(&self.directory)?;
        self.state = saved.state;
        self.artifacts = saved.artifacts;
        self.execution = saved.execution;
        self.verdicts = saved.verdicts;
        Ok(())
    }

    fn save(&self) -> Result<()> {
        HarnessSnapshot {
            schema_version: 1,
            state: self.state.clone(),
            artifacts: self.artifacts.clone(),
            execution: self.execution.clone(),
            verdicts: self.verdicts.clone(),
        }
        .save(&self.directory)
    }

    pub fn status(&self) -> Value {
        let availability: Vec<Value> = STAGES
            .iter()
            .map(|stage| match self.stage_gate(stage, "status") {
                Ok(_) => json!({"stage":stage,"available":true}),
                Err(error) => {
                    json!({"stage":stage,"available":false,"reason":format!("{error:#}")})
                }
            })
            .collect();
        let allowed: Vec<_> = availability
            .iter()
            .filter(|v| v["available"] == true)
            .map(|v| v["stage"].clone())
            .collect();
        json!({"state":self.state,"workspace":self.state["workspace"],"artifacts":self.artifacts,"execution":self.execution,"verdicts":self.verdicts,"allowed_stages":allowed,"stages":STAGES,"stage_availability":availability,"recovery":self.recovery(),"budgets":{"stage_calls":self.config.max_stage_calls,"attempts_per_stage":self.config.max_stage_attempts,"timeout":self.config.run_timeout}})
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

    fn stage_gate(&self, stage: &str, instruction: &str) -> Result<(usize, u64)> {
        ensure!(
            self.state["status"] == "running",
            "Audit is already finished"
        );
        ensure!(!interrupted(), "Interrupted by user");
        ensure!(
            !self.directory.join("patch-transition.json").exists()
                && self.state["baseline_restore_patch"].is_null(),
            "An unfinished patch transition or baseline requires recovery before another stage"
        );
        ensure!(
            !instruction.trim().is_empty(),
            "A dispatch instruction is required"
        );
        let index = STAGES
            .iter()
            .position(|role| *role == stage)
            .context("Unknown stage")?;
        for prerequisite in &STAGES[..if stage == "test-validator" { 3 } else { index }] {
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
        Ok((index, attempt))
    }

    pub fn run_stage(&mut self, stage: &str, instruction: &str) -> Result<Value> {
        let (index, attempt) = match self.stage_gate(stage, instruction) {
            Ok(value) => value,
            Err(error) => {
                let record = json!({"at":now(),"stage":stage,"instruction":instruction,"gate":"dispatch","error":format!("{error:#}"),"recovery":self.recovery()});
                if !self.state["dispatch_rejections"].is_array() {
                    self.state["dispatch_rejections"] = json!([]);
                }
                self.state["dispatch_rejections"]
                    .as_array_mut()
                    .unwrap()
                    .push(record);
                self.save()?;
                return Err(error);
            }
        };
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
        let inputs = self.build_stage_inputs(stage, attempt, instruction);
        // The harness also saves boundary inputs/results for non-Codex test agents.
        write_json(&directory.join("input.json"), &inputs)?;
        let mut outcome = self.accept(stage, index, &directory, &inputs);
        if outcome.is_err() && self.directory.join("patch-transition.json").exists() {
            // The snapshot is the commit point. Do not recover from in-memory
            // values that may have changed before an unsuccessful commit.
            self.reload()?;
            if let Err(error) = ignoring_interrupts(|| self.recover_transition()) {
                outcome =
                    Err(error.context("Stage failed and its patch transition could not recover"));
            }
        }
        let result = match outcome {
            Ok(output) => {
                json!({"status":"completed","stage":stage,"attempt":attempt,"directory":directory,"artifact":output,"execution":self.execution,"verdicts":self.verdicts})
            }
            Err(error) => {
                json!({"status":"failed","stage":stage,"attempt":attempt,"directory":directory,"error":format!("{error:#}"),"gate":"stage_execution","recovery":self.recovery(),"execution":self.execution})
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
                let commit = self.state["workspace"]["commit"]
                    .as_str()
                    .context("Missing base commit")?;
                let entry = run(
                    &[
                        "git".into(),
                        "--literal-pathspecs".into(),
                        "ls-tree".into(),
                        "-z".into(),
                        commit.into(),
                        "--".into(),
                        relative.into(),
                    ],
                    Some(&root),
                    600,
                    None,
                );
                ensure!(
                    entry.returncode == 0
                        && matches!(
                            entry.stdout.split_whitespace().next(),
                            Some("100644" | "100755")
                        ),
                    "Source reference is not a regular file in the recorded base: {relative}"
                );
                let base = run(
                    &["git".into(), "show".into(), format!("{commit}:{relative}")],
                    Some(&root),
                    600,
                    None,
                );
                ensure!(
                    base.returncode == 0,
                    "Cannot read source reference at recorded base: {relative}"
                );
                ensure!(
                    reference["line"]
                        .as_f64()
                        .context("Invalid reference line")?
                        <= base.stdout.lines().count() as f64,
                    "Source reference line is outside the recorded base file: {relative}"
                );
            }
        }
        let gate = self.gate()?;
        gate.verify_worktree()?;
        let replacement = if index < 3 {
            Value::Null
        } else if index == 3 {
            output["patch"].clone()
        } else {
            self.state["applied_patch"].clone()
        };
        // Keep this journal until both the authoritative snapshot and all active
        // file aliases agree. Recovery is safe to repeat after another loss.
        write_json(
            &self.directory.join("patch-transition.json"),
            &json!({"previous":self.state["applied_patch"],"replacement":replacement}),
        )?;
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
        if stage == "test-validator" {
            self.verdicts = grounded_verdicts(&output, &self.execution);
        }
        write_json(&directory.join("accepted.json"), &output)?;
        write_json(
            &self
                .directory
                .join("artifacts")
                .join(format!("{stage}.json")),
            &output,
        )?;
        self.save()?;
        self.recover_transition()?;
        if stage == "test-developer" {
            self.execute(&output, directory)?;
        }
        Ok(output)
    }

    fn save_execution(&self, attempt: &Path) -> Result<()> {
        write_json(&attempt.join("execution.json"), &self.execution)?;
        self.save()
    }

    fn patch_applied(&self, patch: &str) -> Result<bool> {
        Ok(run(
            &[
                "git".into(),
                "apply".into(),
                "--reverse".into(),
                "--check".into(),
                "-".into(),
            ],
            Some(&self.root()?),
            600,
            Some(patch),
        )
        .returncode
            == 0)
    }

    fn recover_transition(&mut self) -> Result<()> {
        ignoring_interrupts(|| {
            let journal = self.directory.join("patch-transition.json");
            if !journal.exists() {
                return Ok(());
            }
            let transition = read_json(&journal)?;
            let desired = self.state["applied_patch"].as_str();
            if !desired
                .map(|patch| self.patch_applied(patch))
                .transpose()?
                .unwrap_or(false)
            {
                for candidate in [&transition["replacement"], &transition["previous"]] {
                    if let Some(patch) = candidate.as_str()
                        && self.patch_applied(patch)?
                    {
                        self.reverse(patch)?;
                        break;
                    }
                }
                if let Some(patch) = desired {
                    self.gate()?.apply(patch)?;
                }
            }
            let paths = self.gate()?.verify_worktree()?;
            // These aliases may have been partially updated before snapshot commit.
            // Restore them from that snapshot, leaving attempt history untouched.
            for role in STAGES {
                let path = self
                    .directory
                    .join("artifacts")
                    .join(format!("{role}.json"));
                if let Some(artifact) = self.artifacts.get(role) {
                    write_json(&path, artifact)?;
                } else if path.exists() {
                    fs::remove_file(path)?;
                }
            }
            if let Some(patch) = desired {
                fs::write(self.directory.join("tests.diff"), patch)?;
                write_json(
                    &self.directory.join("patch-check.json"),
                    &json!({"accepted":true,"paths":paths}),
                )?;
            } else {
                for name in ["tests.diff", "patch-check.json"] {
                    let path = self.directory.join(name);
                    if path.exists() {
                        fs::remove_file(path)?;
                    }
                }
            }
            fs::remove_file(journal)?;
            Ok(())
        })
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
            ensure!(
                !self.directory.join("patch-transition.json").exists()
                    && self.state["baseline_restore_patch"].is_null(),
                "Patch recovery must finish before audit completion"
            );
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
                    if *stage == "test-developer" && !self.implementable() {
                        continue;
                    }
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
        {
            let _lock = RunLock::acquire(&harness.directory)?;
            harness.state["workspace"] = create_workspace(caller, &harness.directory, config, &id)?;
            harness.state["status"] = json!("running");
            harness.state["loop_started_at"] = json!(now());
            harness.save()?;
        }
        let result = agent.coordinate(&mut harness);
        harness.reload()?;
        result?;
        ensure!(
            harness.state["status"] != "running",
            "Coordinator exited without calling finish; audit is incomplete"
        );
        Ok(())
    })();
    let _lock = RunLock::wait(&harness.directory)?;
    harness.reload()?;
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
    let recovery = harness
        .recover_baseline()
        .and_then(|()| harness.recover_transition());
    if let Err(error) = recovery {
        harness.state["status"] = json!("blocked");
        harness.state["recovery_error"] = json!(format!("{error:#}"));
        harness.state["error"] = json!(format!("Patch recovery failed: {error:#}"));
        for verdict in harness.verdicts.as_array_mut().unwrap() {
            verdict["agent_status"] = verdict["status"].clone();
            verdict["status"] = json!("blocked");
            verdict["rationale"] = json!(
                "Patch recovery failed; retained historical evidence does not establish the current worktree result"
            );
        }
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
