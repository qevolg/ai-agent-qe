use super::{read_json, write_json};
use crate::config::Config;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// The commit point for current runs. Other JSON files are compatibility exports.
#[derive(Serialize, Deserialize)]
pub struct HarnessSnapshot {
    #[serde(default)]
    pub schema_version: u32,
    pub state: Value,
    pub artifacts: Value,
    pub execution: Value,
    pub verdicts: Value,
}

impl HarnessSnapshot {
    pub fn load(directory: &Path) -> Result<Self> {
        let mut saved: Self = serde_json::from_value(read_json(&directory.join("harness.json"))?)
            .context("Invalid harness snapshot fields")?;
        ensure!(
            saved.schema_version <= 1,
            "Unsupported harness snapshot version {}",
            saved.schema_version
        );
        if saved.schema_version == 0 {
            // Before versioning, only these post-audit fields could be newer in run.json.
            if let Ok(legacy) = read_json(&directory.join("run.json")) {
                ensure!(
                    legacy["id"] == saved.state["id"],
                    "Run identity differs between snapshot and run.json"
                );
                for key in ["reproductions", "tmux_cleaned_at", "environment_cleaned_at"] {
                    if !legacy[key].is_null() {
                        saved.state[key] = legacy[key].clone();
                    }
                }
                if !legacy["environment_cleaned_at"].is_null() {
                    saved.state["environment_retained"] = Value::Bool(false);
                }
            }
        }
        saved.validate(directory)?;
        Ok(saved)
    }

    fn validate(&self, directory: &Path) -> Result<()> {
        let id = self.state["id"]
            .as_str()
            .context("Snapshot state.id is missing")?;
        ensure!(
            directory.file_name().and_then(|name| name.to_str()) == Some(id),
            "Snapshot state.id differs from run directory"
        );
        ensure!(
            matches!(
                self.state["status"].as_str(),
                Some(
                    "initializing" | "running" | "completed" | "blocked" | "failed" | "interrupted"
                )
            ),
            "Invalid snapshot state.status"
        );
        ensure!(
            self.state["stages"].is_array(),
            "Invalid snapshot state.stages"
        );
        let _: Config = serde_json::from_value(self.state["config"].clone())
            .context("Invalid snapshot state.config")?;
        ensure!(self.artifacts.is_object(), "Invalid snapshot artifacts");
        ensure!(
            self.execution["baseline"].is_array() && self.execution["tests"].is_array(),
            "Invalid snapshot execution.baseline/tests"
        );
        ensure!(self.verdicts.is_array(), "Invalid snapshot verdicts");
        Ok(())
    }

    pub fn save(&self, directory: &Path) -> Result<()> {
        self.validate(directory)?;
        let mut document = serde_json::to_value(self)?;
        document["schema_version"] = Value::from(1);
        write_json(&directory.join("harness.json"), &document)?;
        write_json(&directory.join("run.json"), &self.state)?;
        write_json(&directory.join("execution.json"), &self.execution)
    }
}
