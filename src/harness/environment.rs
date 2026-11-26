use super::*;
use std::collections::HashSet;

fn merge(target: &mut Value, source: &Value) -> Result<()> {
    if source.is_null() {
        return Ok(());
    }
    let overrides = source
        .as_object()
        .context("Environment must be an object")?;
    if !source["compose"].is_null() {
        target["compose_files"] = json!([]);
    }
    if source["compose_files"]
        .as_array()
        .is_some_and(|v| !v.is_empty())
    {
        target.as_object_mut().unwrap().remove("compose");
    }
    target.as_object_mut().unwrap().extend(overrides.clone());
    Ok(())
}
impl AuditHarness<'_> {
    pub(super) fn implementable(&self) -> bool {
        self.artifacts["test-designer"]["plans"]
            .as_array()
            .is_some_and(|plans| {
                plans
                    .iter()
                    .any(|p| p["blocked_reason"].is_null() || p["blocked_reason"] == "")
            })
    }
    pub fn recovery(&self) -> Value {
        let blocked: Vec<_> = self.artifacts["test-designer"]["plans"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| !p["blocked_reason"].is_null() && p["blocked_reason"] != "")
            .map(|p| p["test_id"].clone())
            .collect();
        let mut actions = vec!["status"];
        let available: Vec<_> = STAGES
            .iter()
            .filter(|stage| self.stage_gate(stage, "status").is_ok())
            .copied()
            .collect();
        if self.state["status"] == "running" && !interrupted() {
            actions.push("finish");
            if !available.is_empty() {
                actions.push("run_stage");
            }
            if self.operation_gate().is_ok() {
                actions.extend(["inspect_environment", "prepare_environment"]);
                if self.artifacts.get("test-developer").is_some() {
                    actions.push("execute_tests");
                }
            }
        }
        json!({"blocked_plan_ids":blocked,"permitted_next_actions":actions,"permitted_stages":available,"classification_action":"Return to test-designer to change semantic blockers; dispatch instructions cannot override them"})
    }
    fn operation_gate(&self) -> Result<u64> {
        ensure!(
            self.state["status"] == "running",
            "Audit is already finished"
        );
        ensure!(!interrupted(), "Interrupted by user");
        ensure!(
            !self.directory.join("patch-transition.json").exists()
                && self.state["baseline_restore_patch"].is_null(),
            "An unfinished patch transition or baseline requires recovery"
        );
        let remaining = self.remaining_time()?;
        let count = self.state["environment_actions"]
            .as_array()
            .map_or(0, Vec::len)
            + self.state["execution_attempts"]
                .as_array()
                .map_or(0, Vec::len);
        ensure!(
            (count as u64) < self.config.max_stage_calls,
            "Environment operation budget exhausted"
        );
        Ok(remaining)
    }
    fn remaining_time(&self) -> Result<u64> {
        let elapsed = chrono::Utc::now()
            .signed_duration_since(chrono::DateTime::parse_from_rfc3339(
                self.state["loop_started_at"]
                    .as_str()
                    .context("Missing start time")?,
            )?)
            .num_seconds()
            .max(0) as u64;
        ensure!(
            elapsed < self.config.run_timeout,
            "Audit time budget exhausted"
        );
        Ok(self.config.run_timeout - elapsed)
    }
    fn action(&mut self, operation: &str, result: Result<Value>) -> Result<Value> {
        let value = match result {
            Ok(value) => {
                json!({"status":"completed","operation":operation,"at":now(),"result":value})
            }
            Err(error) => {
                json!({"status":"failed","operation":operation,"at":now(),"error":format!("{error:#}"),"gate":operation,"recovery":self.recovery()})
            }
        };
        if !self.state["environment_actions"].is_array() {
            self.state["environment_actions"] = json!([]);
        }
        self.state["environment_actions"]
            .as_array_mut()
            .unwrap()
            .push(value.clone());
        self.save()?;
        Ok(value)
    }
    pub fn inspect_environment(&mut self, probe: &Value) -> Result<Value> {
        let result = (|| {
            let remaining = self.operation_gate()?;
            crate::environment_probe::inspect(
                &self.root()?,
                &self.directory.join("environment/probes"),
                probe,
                self.config.docker_timeout.min(remaining),
            )
        })();
        self.action("inspect_environment", result)
    }
    pub fn prepare_environment(&mut self, environment: Option<Value>) -> Result<Value> {
        let result = (|| {
            let remaining = self.operation_gate()?;
            let locked = self.state["environment_started"] == true
                || self.directory.join("environment/ownership.json").exists();
            let mut spec = if self.state["environment"].is_object() {
                self.state["environment"].clone()
            } else {
                self.artifacts["code-explorer"]["environment"].clone()
            };
            if !spec.is_object() {
                spec = json!({});
            }
            if let Some(explicit) = environment {
                merge(&mut spec, &explicit)?;
            }
            merge(&mut spec, &self.config.environment)?;
            ensure!(
                spec["runner_service"]
                    .as_str()
                    .is_some_and(|v| !v.is_empty()),
                "No usable environment specification; inspect the project or supply an explicit runner_service"
            );
            ensure!(
                !locked || self.state["environment"] == spec,
                "Environment changed after startup; retain this run and start a new audit for the new environment"
            );
            self.state["environment"] = spec.clone();
            self.save()?;
            if self.state["environment_ready"] != true {
                let runtime = (self.factory)(
                    &self.root()?,
                    &self.directory,
                    self.state["id"].as_str().unwrap(),
                    spec.clone(),
                    self.config.docker_timeout.min(remaining),
                );
                self.state["environment_retained"] = json!(true);
                self.save()?;
                runtime.start()?;
                self.state["environment_started"] = json!(true);
                self.state["environment_ready"] = json!(true);
            }
            Ok(json!({"environment":spec,"environment_ready":true}))
        })();
        self.action("prepare_environment", result)
    }
    pub fn execute_tests(&mut self, test_ids: Option<Vec<String>>) -> Result<Value> {
        let result = (|| {
            self.operation_gate()?;
            let mut output = self
                .artifacts
                .get("test-developer")
                .context("Missing prerequisite: test-developer")?
                .clone();
            if let Some(ids) = test_ids {
                let selected: HashSet<_> = ids.iter().collect();
                ensure!(
                    !ids.is_empty() && selected.len() == ids.len(),
                    "Select a nonempty unique test subset"
                );
                let tests = output["tests"].as_array().context("Missing tests")?;
                ensure!(
                    ids.iter()
                        .all(|id| tests.iter().any(|t| t["id"].as_str() == Some(id))),
                    "Unknown test ID"
                );
                output["tests"] = json!(
                    tests
                        .iter()
                        .filter(|t| selected.contains(&t["id"].as_str().unwrap().to_owned()))
                        .cloned()
                        .collect::<Vec<_>>()
                );
            }
            let directory = self
                .directory
                .join("executions/rounds")
                .join(uuid::Uuid::new_v4().simple().to_string());
            self.execute(&output, &directory)?;
            Ok(json!({"execution":self.execution}))
        })();
        match result {
            Ok(value) => {
                Ok(json!({"status":"completed","result":value,"execution":self.execution}))
            }
            Err(error) => Ok(
                json!({"status":"failed","gate":"execute_tests","error":format!("{error:#}"),"recovery":self.recovery(),"execution":self.execution}),
            ),
        }
    }
    pub(super) fn execute(&mut self, output: &Value, attempt: &Path) -> Result<()> {
        self.operation_gate()?;
        self.gate()?.verify_worktree()?;
        self.verdicts = json!([]);
        self.artifacts
            .as_object_mut()
            .unwrap()
            .remove("test-validator");
        let validator = self.directory.join("artifacts/test-validator.json");
        if validator.exists() {
            fs::remove_file(validator)?;
        }
        self.execution = json!({"baseline":[],"tests":[]});
        let entry = self.state["execution_attempts"]
            .as_array()
            .map_or(0, Vec::len);
        if !self.state["execution_attempts"].is_array() {
            self.state["execution_attempts"] = json!([]);
        }
        self.state["execution_attempts"].as_array_mut().unwrap().push(json!({"at":now(),"directory":attempt,"test_ids":output["tests"].as_array().unwrap().iter().map(|t|t["id"].clone()).collect::<Vec<_>>(),"status":"running"}));
        self.save_execution(attempt)?;
        let outcome = self.execute_round(output, attempt);
        self.state["execution_attempts"][entry]["status"] = json!(if outcome.is_ok() {
            "completed"
        } else {
            "failed"
        });
        self.state["execution_attempts"][entry]["execution"] = self.execution.clone();
        if let Err(error) = &outcome {
            self.state["execution_attempts"][entry]["error"] = json!(format!("{error:#}"));
        }
        self.save_execution(attempt)?;
        outcome
    }
    fn execute_round(&mut self, output: &Value, attempt: &Path) -> Result<()> {
        let prepared = self.prepare_environment(None)?;
        ensure!(
            prepared["status"] == "completed",
            "Environment preparation failed: {}",
            prepared["error"]
        );
        let runtime = (self.factory)(
            &self.root()?,
            &self.directory,
            self.state["id"].as_str().unwrap(),
            self.state["environment"].clone(),
            self.config.docker_timeout.min(self.remaining_time()?),
        );
        let result = (|| {
            let global = if !self.config.baseline.is_empty() {
                Some(self.config.baseline.clone())
            } else {
                None
            };
            let mut failures = Vec::new();
            if let Some(commands) = &global {
                self.baselines(runtime.as_ref(), commands, None, output, attempt)?;
            }
            for test in output["tests"].as_array().unwrap() {
                if global.is_none() {
                    let commands: Vec<Vec<String>> =
                        serde_json::from_value(if test["baseline"].is_array() {
                            test["baseline"].clone()
                        } else {
                            self.artifacts["code-explorer"]["baseline"].clone()
                        })?;
                    if let Err(error) = self.baselines(
                        runtime.as_ref(),
                        &commands,
                        test["id"].as_str(),
                        output,
                        attempt,
                    ) {
                        failures.push(format!("{}: {error:#}", test["id"]));
                        continue;
                    }
                }
                let record = collect_test(
                    runtime.as_ref(),
                    test,
                    self.config.test_timeout.min(self.remaining_time()?),
                )?;
                self.execution["tests"].as_array_mut().unwrap().push(record);
                self.save_execution(attempt)?;
                self.gate()?.verify_worktree()?;
            }
            ensure!(
                failures.is_empty(),
                "Applicable baseline failed; target verification is blocked: {}",
                failures.join("; ")
            );
            Ok(())
        })();
        if let Err(error) = ignoring_interrupts(|| runtime.logs()) {
            self.state["log_collection_error"] = json!(format!("{error:#}"));
        }
        result
    }
    fn baselines(
        &mut self,
        runtime: &dyn crate::workflow::TestRuntime,
        commands: &[Vec<String>],
        for_test_id: Option<&str>,
        output: &Value,
        attempt: &Path,
    ) -> Result<()> {
        if commands.is_empty() {
            return Ok(());
        }
        let patch = output["patch"].as_str().unwrap();
        self.state["baseline_restore_patch"] = json!(patch);
        self.save()?;
        self.reverse(patch)?;
        let result = (|| {
            for command in commands {
                ensure!(!interrupted(), "Interrupted by user");
                let id = format!("baseline-{}", uuid::Uuid::new_v4().simple());
                let mut record = runtime.execute_test(
                    command,
                    &id,
                    self.config.test_timeout.min(self.remaining_time()?),
                )?;
                if let Some(test_id) = for_test_id {
                    record["for_test_id"] = json!(test_id);
                }
                let healthy =
                    record["result"]["returncode"] == 0 && record["result"]["timed_out"] != true;
                self.execution["baseline"]
                    .as_array_mut()
                    .unwrap()
                    .push(record);
                self.save_execution(attempt)?;
                ensure!(
                    healthy,
                    "Baseline command failed; target verification is blocked"
                );
            }
            Ok(())
        })();
        ignoring_interrupts(|| self.gate()?.apply(patch))?;
        self.state["baseline_restore_patch"] = Value::Null;
        self.save()?;
        result
    }
}
