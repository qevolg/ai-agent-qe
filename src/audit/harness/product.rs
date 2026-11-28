use super::*;
use crate::{agents::stage::PRODUCT_STAGES, guards::product as guard};

impl AuditHarness<'_> {
    pub fn product_workflow(&self) -> bool {
        self.state["workflow"] == "product"
    }

    pub(super) fn stages(&self) -> &'static [&'static str] {
        if self.product_workflow() {
            &PRODUCT_STAGES
        } else {
            &STAGES
        }
    }

    pub(super) fn clear_product_review(&mut self) -> Result<()> {
        self.artifacts
            .as_object_mut()
            .unwrap()
            .remove("result-reviewer");
        let path = self.directory.join("artifacts/result-reviewer.json");
        if path.exists() {
            fs::remove_file(path)?;
        }
        self.verdicts = json!([]);
        Ok(())
    }

    pub(super) fn accept_product(
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
        self.gate()?.verify_worktree()?;
        if stage == "product-explorer" {
            guard::validate_services(&output, self.state["prompt"].as_str().unwrap())?;
            if self.state["environment_started"] == true {
                ensure!(
                    self.artifacts[stage]["services"] == output["services"],
                    "Service targets are pinned after preparation; cannot replace an explicit endpoint"
                );
            }
        }
        if stage == "scenario-designer" {
            let previous = self.artifacts[stage]["scenarios"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let current = guard::items(&output, "scenarios")?;
            if let Some(records) = self.execution["operations"].as_array_mut() {
                records.retain(|r| {
                    current.iter().any(|s| {
                        s["id"] == r["scenario_id"] && r["scenario_version"] == guard::version(s)
                    })
                });
            }
            self.state["document_revision"] =
                json!(self.state["document_revision"].as_u64().unwrap_or(0) + 1);
            crate::audit::product::write_steps(
                &self.directory,
                &output,
                self.state["document_revision"].as_u64().unwrap(),
            )?;
            let change = json!({"revision":self.state["document_revision"],"at":now(),"previous_scenarios":previous.len(),"current_scenarios":current.len(),"instruction":inputs["instruction"]});
            self.state["document_changes"]
                .as_array_mut()
                .unwrap()
                .push(change);
        }
        // Later stages are conclusions about the accepted upstream artifacts.
        for downstream in &PRODUCT_STAGES[index + 1..] {
            self.artifacts.as_object_mut().unwrap().remove(*downstream);
            let path = self
                .directory
                .join("artifacts")
                .join(format!("{downstream}.json"));
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        if index < 4 {
            self.clear_product_review()?;
        }
        if index == 0 {
            self.execution = json!({"baseline":[],"tests":[],"operations":[]});
        }
        let mut accepted = output.clone();
        if stage == "result-reviewer" {
            accepted = guard::ground_review(&output, &self.artifacts, &self.execution);
            self.verdicts = accepted["issue_results"].clone();
        }
        self.artifacts[stage] = accepted.clone();
        write_json(&directory.join("accepted.json"), &accepted)?;
        write_json(
            &self
                .directory
                .join("artifacts")
                .join(format!("{stage}.json")),
            &accepted,
        )?;
        self.save()?;
        if matches!(stage, "product-executor" | "issue-reproducer")
            && !guard::items(&output, "operations")?.is_empty()
        {
            self.execute_product_operations(&output["operations"], directory)?;
            // Environment recovery can invalidate downstream conclusions.
            self.artifacts[stage] = accepted.clone();
            write_json(
                &self
                    .directory
                    .join("artifacts")
                    .join(format!("{stage}.json")),
                &accepted,
            )?;
            self.save()?;
        }
        Ok(accepted)
    }

    pub(super) fn execute_product_operations(
        &mut self,
        operations: &Value,
        directory: &Path,
    ) -> Result<()> {
        self.operation_gate()?;
        self.clear_product_review()?;
        // Invalidate the entire submitted batch before any environment or
        // collection call can fail. Earlier successful attempts remain history.
        let submitted = operations
            .as_array()
            .context("Missing product operations")?;
        self.execution["operations"]
            .as_array_mut()
            .unwrap()
            .retain(|r| !submitted.iter().any(|o| o["id"] == r["operation_id"]));
        self.save()?;
        let prepared = self.prepare_environment(None)?;
        ensure!(
            prepared["status"] == "completed",
            "Product environment is unavailable: {}",
            prepared["error"]
        );
        let runtime = (self.factory)(
            &self.root()?,
            &self.directory,
            self.state["id"].as_str().unwrap(),
            self.state["environment"].clone(),
            self.config.docker_timeout.min(self.remaining_time()?),
        );
        let services = guard::items(&self.artifacts["product-explorer"], "services")?.clone();
        let scenarios = guard::items(&self.artifacts["scenario-designer"], "scenarios")?.clone();
        let entry = self.state["execution_attempts"]
            .as_array()
            .map_or(0, Vec::len);
        if !self.state["execution_attempts"].is_array() {
            self.state["execution_attempts"] = json!([]);
        }
        self.state["execution_attempts"].as_array_mut().unwrap().push(json!({"at":now(),"directory":directory,"status":"running","operation_ids":operations.as_array().unwrap().iter().map(|o|o["id"].clone()).collect::<Vec<_>>()}));
        self.save()?;
        let outcome = (|| -> Result<()> {
            for operation in operations.as_array().unwrap() {
                self.gate()?.verify_worktree()?;
                let scenario = scenarios
                    .iter()
                    .find(|s| s["id"] == operation["scenario_id"])
                    .context("Unknown documented scenario")?;
                let record = crate::audit::product::collect_operation(
                    runtime.as_ref(),
                    operation,
                    scenario,
                    &services,
                    self.config.test_timeout.min(self.remaining_time()?),
                )?;
                let records = self.execution["operations"].as_array_mut().unwrap();
                records.retain(|r| r["operation_id"] != operation["id"]);
                records.push(record);
                self.save_execution(directory)?;
            }
            Ok(())
        })();
        let logs = runtime.logs();
        self.state["execution_attempts"][entry]["status"] = json!(if outcome.is_ok() {
            "completed"
        } else {
            "failed"
        });
        self.state["execution_attempts"][entry]["execution_path"] =
            json!(directory.join("execution.json"));
        if let Err(error) = &outcome {
            self.state["execution_attempts"][entry]["error"] = json!(format!("{error:#}"));
        }
        if let Err(error) = logs {
            self.state["execution_attempts"][entry]["logs_error"] = json!(format!("{error:#}"));
        }
        self.save_execution(directory)?;
        outcome
    }

    pub(super) fn finish_product(&mut self, status: &str, reason: &str) -> Result<()> {
        if status == "completed" {
            for stage in PRODUCT_STAGES {
                ensure!(
                    self.artifacts.get(stage).is_some(),
                    "Missing product completion prerequisite: {stage}"
                );
            }
            self.gate()?.verify_worktree()?;
        }
        let incomplete = status == "completed"
            && (guard::items(&self.artifacts["scenario-designer"], "scenarios")?.is_empty()
                || guard::items(&self.artifacts["scenario-designer"], "coverage")?.is_empty()
                || guard::items(&self.artifacts["result-reviewer"], "scenario_results")?
                    .iter()
                    .any(|r| !matches!(r["status"].as_str(), Some("passed" | "failed" | "flaky")))
                || guard::items(&self.artifacts["scenario-designer"], "coverage")?
                    .iter()
                    .any(|c| c["status"] == "gap")
                || !guard::items(&self.artifacts["issue-reproducer"], "followups")?.is_empty()
                || guard::items(&self.artifacts["result-reviewer"], "issue_results")?
                    .iter()
                    .any(|r| r["status"] == "blocked"));
        self.state["status"] = json!(if incomplete { "blocked" } else { status });
        self.state["finish_reason"] = json!(if incomplete {
            format!("仍有受阻或未执行场景、覆盖遗漏或待补充线索；{reason}")
        } else {
            reason.to_owned()
        });
        if self.state["status"] == "blocked" {
            self.state["error"] = self.state["finish_reason"].clone();
        }
        self.save()
    }

    pub(super) fn reexecute_product(&mut self, scenario_ids: Option<Vec<String>>) -> Result<Value> {
        let outcome = (|| -> Result<()> {
            self.operation_gate()?;
            let records = self.execution["operations"]
                .as_array()
                .context("No saved product operations")?;
            let scenarios = guard::items(&self.artifacts["scenario-designer"], "scenarios")?;
            if let Some(ids) = &scenario_ids {
                let unique: std::collections::HashSet<_> = ids.iter().collect();
                ensure!(
                    !ids.is_empty()
                        && unique.len() == ids.len()
                        && ids
                            .iter()
                            .all(|id| scenarios.iter().any(|s| s["id"] == *id)),
                    "Unknown or duplicate scenario ID"
                );
            }
            let operations: Vec<_> = records.iter().filter(|r| scenario_ids.as_ref().is_none_or(|ids| ids.iter().any(|id| r["scenario_id"] == *id)))
                .map(|r| json!({"id":r["operation_id"],"scenario_id":r["scenario_id"],"step_id":r["step_id"],"command":r["command"],"purpose":r["purpose"]})).collect();
            ensure!(
                !operations.is_empty(),
                "No saved product operations for the selected scenarios"
            );
            self.artifacts
                .as_object_mut()
                .unwrap()
                .remove("issue-reproducer");
            let directory = self
                .directory
                .join("executions/rounds")
                .join(uuid::Uuid::new_v4().simple().to_string());
            self.execute_product_operations(&json!(operations), &directory)
        })();
        match outcome {
            Ok(()) => Ok(json!({"status":"completed","execution":self.execution})),
            Err(error) => Ok(
                json!({"status":"failed","error":format!("{error:#}"),"execution":self.execution,"recovery":self.recovery()}),
            ),
        }
    }
}
