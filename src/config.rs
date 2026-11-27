use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::state::{absolute_path, expand_user};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Publication {
    pub project: Option<String>,
    pub glab_command: Vec<String>,
}

impl Default for Publication {
    fn default() -> Self {
        Self {
            project: None,
            glab_command: vec!["glab".into()],
        }
    }
}

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
    pub publication: Publication,
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
            publication: Publication::default(),
            state_dir: absolute_path(&expand_user(Path::new("~/.aaqe/runs"))),
            remote: "origin".into(),
            branch: "main".into(),
            codex_command: vec!["codex".into()],
            model: "gpt-6.1-sol".into(),
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
            agent_timeout: 7200,
            test_timeout: 7200,
            docker_timeout: 3600,
            max_stage_calls: 20,
            max_stage_attempts: 4,
            run_timeout: 43200,
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
        let schema: Value = serde_json::from_str(include_str!("../resources/schemas/config.json"))?;
        let validator = jsonschema::validator_for(&schema)
            .map_err(|e| anyhow::anyhow!("Invalid configuration schema: {e}"))?;
        if let Err(error) = validator.validate(&data) {
            bail!("Invalid configuration: {error}");
        }
        let mut config = Self::default();
        if let Some(publication) = data.get("publication") {
            config.publication = serde_json::from_value(publication.clone())?;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn config_is_optional_and_resolves_paths_without_reading_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path();
        let default = Config::load(directory, None).unwrap();
        assert_eq!(default.remote, "origin");
        assert_eq!(default.branch, "main");
        assert_eq!(default.max_stage_calls, 20);
        assert_eq!(default.max_stage_attempts, 4);
        assert_eq!(default.model, "gpt-6.1-sol");
        assert_eq!(default.agent_timeout, 7200);
        assert_eq!(default.test_timeout, 7200);
        assert_eq!(default.docker_timeout, 3600);
        assert_eq!(default.run_timeout, 43200);
        assert!(default.state_dir.is_absolute());
        fs::write(
            directory.join("aaqe.yaml"),
            "loop: {max_stage_calls: 8, max_stage_attempts: 2, timeout: 3600}\nstate_dir: runs\ngit:\n  branch: develop\nenvironment:\n  env_file: private.env\n",
        )
        .unwrap();
        fs::write(directory.join("private.env"), "PASSWORD=do-not-publish\n").unwrap();
        let loaded = Config::load(directory, None).unwrap();
        assert_eq!(loaded.branch, "develop");
        assert_eq!(loaded.max_stage_calls, 8);
        assert_eq!(loaded.max_stage_attempts, 2);
        assert_eq!(loaded.run_timeout, 3600);
        assert_eq!(loaded.state_dir, directory.join("runs"));
        assert_eq!(
            loaded.environment["env_file"],
            directory.join("private.env").to_str().unwrap()
        );
        let snapshot = serde_json::to_value(&loaded).unwrap();
        assert!(!snapshot.to_string().contains("do-not-publish"));
        assert_eq!(
            serde_json::from_value::<Config>(snapshot).unwrap().branch,
            "develop"
        );
    }

    #[test]
    fn invalid_config_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        for content in [
            "unknown: true",
            "git: []",
            "environment: {services: nope}",
            "timeouts: {test: 0}",
            "git: {remote: --upload-pack=bad}",
            "codex: {command: []}",
            "loop: {max_stage_calls: 0}",
            "loop: {max_stage_attempts: -1}",
            "loop: {timeout: 0}",
        ] {
            fs::write(temp.path().join("aaqe.yaml"), content).unwrap();
            assert!(Config::load(temp.path(), None).is_err(), "{content}");
        }
        assert!(Config::load(temp.path(), Some(&temp.path().join("missing.yaml"))).is_err());
    }

    #[test]
    fn integral_float_config_timeouts_preserve_json_schema_integer_semantics() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("aaqe.yaml"),
            "timeouts: {agent: 3.0, test: 4.0, docker: 5.0}\n",
        )
        .unwrap();
        let config = crate::config::Config::load(temp.path(), None).unwrap();
        assert_eq!(
            (
                config.agent_timeout,
                config.test_timeout,
                config.docker_timeout
            ),
            (3, 4, 5)
        );
        assert_eq!(crate::config::unsigned_integer(&json!(7.0)), Some(7));
        assert_eq!(crate::config::unsigned_integer(&json!(7.1)), None);
        let saved: crate::config::Config = serde_json::from_value(
            json!({"agent_timeout":3.0,"test_timeout":4.0,"docker_timeout":5.0}),
        )
        .unwrap();
        assert_eq!(
            (
                saved.agent_timeout,
                saved.test_timeout,
                saved.docker_timeout
            ),
            (3, 4, 5)
        );
    }

    #[test]
    fn yaml_merge_keys_keep_inherited_settings_and_explicit_overrides() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("aaqe.yaml"), "environment:\n  <<: &runner_defaults\n    runner_workdir: /workspace\n    services: [runner]\n    runner_service: inherited\n  runner_service: runner\n").unwrap();
        let config = crate::config::Config::load(temp.path(), None).unwrap();
        assert_eq!(
            config.environment,
            json!({"runner_workdir":"/workspace", "services":["runner"], "runner_service":"runner"})
        );
    }
}
