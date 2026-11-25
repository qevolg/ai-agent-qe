use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::state::{absolute_path, expand_user};

// JSON Schema's integer type includes mathematically integral JSON numbers
// such as 3.0; serde_json's as_u64 alone only accepts integer storage.
pub fn unsigned_integer(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0 && n.fract() == 0.0 && *n < u64::MAX as f64)
            .map(|n| n as u64)
    })
}

fn deserialize_timeout<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let value = Value::deserialize(deserializer)?;
    unsigned_integer(&value)
        .ok_or_else(|| serde::de::Error::custom("Expected a nonnegative integer timeout"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub state_dir: PathBuf,
    pub remote: String,
    pub branch: String,
    pub codex_command: Vec<String>,
    pub model: String,
    pub environment: Value,
    pub references: Vec<String>,
    pub test_paths: Vec<String>,
    pub baseline: Vec<Vec<String>>,
    #[serde(deserialize_with = "deserialize_timeout")]
    pub agent_timeout: u64,
    #[serde(deserialize_with = "deserialize_timeout")]
    pub test_timeout: u64,
    #[serde(deserialize_with = "deserialize_timeout")]
    pub docker_timeout: u64,
    #[serde(deserialize_with = "deserialize_timeout")]
    pub max_stage_calls: u64,
    #[serde(deserialize_with = "deserialize_timeout")]
    pub max_stage_attempts: u64,
    #[serde(deserialize_with = "deserialize_timeout")]
    pub run_timeout: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            state_dir: absolute_path(&expand_user(Path::new("~/.aaqe/runs"))),
            remote: "origin".into(),
            branch: "main".into(),
            codex_command: vec!["codex".into()],
            model: String::new(),
            environment: json!({}),
            references: vec![],
            test_paths: [
                "tests/**",
                "test/**",
                "**/tests/**",
                "**/test/**",
                ".aaqe/tests/**",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            baseline: vec![],
            agent_timeout: 1800,
            test_timeout: 1200,
            docker_timeout: 1200,
            max_stage_calls: 20,
            max_stage_attempts: 4,
            run_timeout: 14400,
        }
    }
}

impl Config {
    pub fn load(directory: &Path, filename: Option<&Path>) -> Result<Self> {
        let path = filename
            .map(absolute_path)
            .unwrap_or_else(|| absolute_path(directory).join("aaqe.yaml"));
        if !path.exists() {
            if filename.is_some() {
                bail!("Configuration does not exist: {}", path.display());
            }
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(&path).context("Invalid configuration")?;
        let mut yaml: serde_yaml::Value =
            serde_yaml::from_str(&contents).context("Invalid configuration")?;
        yaml.apply_merge().context("Invalid configuration")?;
        let mut data: Value = serde_json::to_value(yaml).context("Invalid configuration")?;
        if data.is_null() || data == json!(false) || data == json!("") || data == json!([]) {
            data = json!({});
        }
        let schema: Value = serde_json::from_str(include_str!("resources/schemas/config.json"))?;
        let validator = jsonschema::validator_for(&schema)
            .map_err(|e| anyhow::anyhow!("Invalid configuration schema: {e}"))?;
        if let Err(error) = validator.validate(&data) {
            bail!("Invalid configuration: {error}");
        }
        let mut config = Self::default();
        let parent = path.parent().context("Invalid configuration path")?;
        if let Some(state) = data["state_dir"].as_str() {
            config.state_dir = absolute_path(&parent.join(expand_user(Path::new(state))));
        }
        if let Some(remote) = data["git"]["remote"].as_str() {
            config.remote = remote.into();
        }
        if let Some(branch) = data["git"]["branch"].as_str() {
            config.branch = branch.into();
        }
        if let Some(command) = data["codex"].get("command") {
            config.codex_command = serde_json::from_value(command.clone())?;
        }
        if let Some(model) = data["codex"]["model"].as_str() {
            config.model = model.into();
        }
        if let Some(environment) = data.get("environment") {
            config.environment = environment.clone();
        }
        if let Some(env_file) = config.environment["env_file"]
            .as_str()
            .filter(|s| !s.is_empty())
        {
            config.environment["env_file"] = json!(absolute_path(&parent.join(env_file)));
        }
        if let Some(references) = data.get("references") {
            config.references = serde_json::from_value(references.clone())?;
        }
        if let Some(paths) = data.get("test_paths") {
            config.test_paths = serde_json::from_value(paths.clone())?;
        }
        if let Some(baseline) = data.get("baseline") {
            config.baseline = serde_json::from_value(baseline.clone())?;
        }
        if let Some(timeout) = unsigned_integer(&data["timeouts"]["agent"]) {
            config.agent_timeout = timeout;
        }
        if let Some(timeout) = unsigned_integer(&data["timeouts"]["test"]) {
            config.test_timeout = timeout;
        }
        if let Some(timeout) = unsigned_integer(&data["timeouts"]["docker"]) {
            config.docker_timeout = timeout;
        }
        if let Some(value) = unsigned_integer(&data["loop"]["max_stage_calls"]) {
            config.max_stage_calls = value;
        }
        if let Some(value) = unsigned_integer(&data["loop"]["max_stage_attempts"]) {
            config.max_stage_attempts = value;
        }
        if let Some(value) = unsigned_integer(&data["loop"]["timeout"]) {
            config.run_timeout = value;
        }
        Ok(config)
    }
}
