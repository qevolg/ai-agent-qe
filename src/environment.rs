use crate::{
    patchgate::safe_path,
    process::{ProcessResult, run, save_result},
    state::{absolute_path, read_json, write_json},
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

type Executor = Arc<dyn Fn(&[String], u64) -> ProcessResult + Send + Sync>;

pub struct ComposeEnvironment {
    pub workspace: PathBuf,
    pub run_dir: PathBuf,
    pub project: String,
    pub spec: Value,
    pub env_dir: PathBuf,
    pub results: PathBuf,
    timeout: u64,
    execute: Executor,
}

pub fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

impl ComposeEnvironment {
    pub fn new(workspace: &Path, run_dir: &Path, run_id: &str, spec: Value, timeout: u64) -> Self {
        let run_dir = absolute_path(run_dir);
        Self {
            workspace: absolute_path(workspace),
            env_dir: run_dir.join("environment"),
            results: run_dir.join("executions"),
            run_dir,
            project: format!("aaqe-{}", run_id.to_lowercase()),
            spec,
            timeout,
            execute: Arc::new(|argv, timeout| run(argv, None, timeout, None)),
        }
    }
    pub fn with_executor(mut self, execute: Executor) -> Self {
        self.execute = execute;
        self
    }
    pub fn arguments(&self, resolved: bool) -> Result<Vec<String>> {
        let mut argv = vec![
            "docker".into(),
            "compose".into(),
            "--project-name".into(),
            self.project.clone(),
            "--project-directory".into(),
            self.workspace.to_string_lossy().into_owned(),
        ];
        let snapshot = self.env_dir.join("compose.resolved.json");
        if resolved {
            ensure!(
                snapshot.is_file(),
                "Compose snapshot is missing; cannot inspect or change owned resources"
            );
            let ownership = read_json(&self.env_dir.join("ownership.json"))
                .context("Compose snapshot integrity check failed")?;
            ensure!(
                ownership["project"].as_str() == Some(&self.project),
                "Environment ownership does not match run ID"
            );
            let digest = format!("{:x}", Sha256::digest(fs::read(&snapshot)?));
            ensure!(
                ownership["compose_sha256"].as_str() == Some(&digest),
                "Compose snapshot integrity check failed"
            );
            argv.extend(["-f".into(), snapshot.to_string_lossy().into_owned()]);
            return Ok(argv);
        }
        let compose = strings(&self.spec["compose_files"]);
        let inline = self.inline_compose()?;
        ensure!(
            inline.is_none() || compose.is_empty(),
            "Cannot supply both compose_files and inline compose"
        );
        if inline.is_some() {
            argv.extend([
                "-f".into(),
                self.env_dir
                    .join("compose.generated.json")
                    .to_string_lossy()
                    .into_owned(),
            ]);
        }
        ensure!(
            !compose.is_empty() || inline.is_some(),
            "No Docker Compose environment was found; supply project configuration"
        );
        for relative in compose {
            let path = safe_path(&self.workspace, &relative)?;
            ensure!(path.is_file(), "Compose file is missing: {relative}");
            argv.extend(["-f".into(), path.to_string_lossy().into_owned()]);
        }
        if let Some(env) = self.spec["env_file"].as_str().filter(|s| !s.is_empty()) {
            argv.extend(["--env-file".into(), env.into()]);
        }
        argv.extend([
            "-f".into(),
            self.env_dir
                .join("aaqe.override.json")
                .to_string_lossy()
                .into_owned(),
        ]);
        Ok(argv)
    }
    fn checked(&self, argv: &[String], label: &str) -> Result<ProcessResult> {
        let result = (self.execute)(argv, self.timeout);
        save_result(
            &result,
            &self
                .env_dir
                .join(label)
                .join(Uuid::new_v4().simple().to_string()),
        )?;
        ensure!(
            result.returncode == 0,
            "Docker {label} failed (exit {}); environment retained",
            result.returncode
        );
        Ok(result)
    }
    pub fn start(&self) -> Result<()> {
        ensure!(
            !absolute_path(&self.run_dir).starts_with(absolute_path(&self.workspace)),
            "Environment artifacts must be outside the worktree"
        );
        self.validate_spec()?;
        fs::create_dir_all(&self.env_dir)?;
        fs::create_dir_all(&self.results)?;
        let runner = self.spec["runner_service"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("No test runner service was identified")?;
        let owned = self.env_dir.join("ownership.json").exists()
            || self.env_dir.join("compose.resolved.json").exists();
        if owned {
            self.arguments(true)?;
            let ownership = read_json(&self.env_dir.join("ownership.json"))?;
            if let Some(digest) = ownership["spec_sha256"].as_str() {
                ensure!(
                    digest == self.spec_digest()?,
                    "Compose environment definition is immutable once ownership exists"
                );
            }
        } else {
            self.create_snapshot(runner)?;
        }
        let mut services = strings(&self.spec["services"]);
        if !services.iter().any(|s| s == runner) {
            services.push(runner.into());
        }
        let mut unique = Vec::new();
        for service in services {
            if !unique.contains(&service) {
                unique.push(service);
            }
        }
        let mut argv = self.arguments(true)?;
        argv.extend([
            "up".into(),
            "-d".into(),
            "--wait".into(),
            "--wait-timeout".into(),
            crate::config::unsigned_integer(&self.spec["readiness_timeout"])
                .unwrap_or(180)
                .to_string(),
        ]);
        argv.extend(unique);
        self.checked(&argv, "up")?;
        if let Some(commands) = self.spec["readiness"].as_array() {
            for (index, command) in commands.iter().enumerate() {
                let mut argv = self.arguments(true)?;
                argv.extend(["exec".into(), "-T".into(), runner.into()]);
                argv.extend(strings(command));
                self.checked(&argv, &format!("readiness-{index}"))?;
            }
        }
        Ok(())
    }
    fn spec_digest(&self) -> Result<String> {
        let mut spec = self.spec.clone();
        spec.sort_all_objects();
        Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(&spec)?)))
    }
    fn inline_compose(&self) -> Result<Option<Value>> {
        let Some(value) = self.spec.get("compose").filter(|value| !value.is_null()) else {
            return Ok(None);
        };
        let document = if let Some(encoded) = value.as_str() {
            serde_json::from_str(encoded).context("Inline compose must encode a JSON object")?
        } else {
            value.clone()
        };
        ensure!(document.is_object(), "Inline compose must be a JSON object");
        Ok(Some(document))
    }
    fn validate_spec(&self) -> Result<()> {
        let files = strings(&self.spec["compose_files"]);
        if self.inline_compose()?.is_some() {
            ensure!(
                files.is_empty(),
                "Cannot supply both compose_files and inline compose"
            );
        }
        if let Some(readiness) = self.spec.get("readiness") {
            let commands = readiness
                .as_array()
                .context("Readiness must contain argv arrays")?;
            ensure!(commands.len() <= 32, "Too many readiness commands");
            for command in commands {
                let argv = command
                    .as_array()
                    .context("Readiness must contain argv arrays")?;
                ensure!(
                    !argv.is_empty()
                        && argv.len() <= 64
                        && argv
                            .iter()
                            .all(|arg| arg.as_str().is_some_and(|s| !s.is_empty()
                                && s.len() <= 8192
                                && !s.contains('\0'))),
                    "Invalid readiness argv"
                );
            }
        }
        Ok(())
    }
    fn create_snapshot(&self, runner: &str) -> Result<()> {
        for name in strings(&self.spec["external_containers"]) {
            let result = self.checked(
                &[
                    "docker".into(),
                    "inspect".into(),
                    "--type".into(),
                    "container".into(),
                    name.clone(),
                ],
                &format!("external-{}", &Uuid::new_v4().simple().to_string()[..8]),
            )?;
            let inspected: Value = serde_json::from_str(&result.stdout)
                .context(format!("Cannot inspect external test container: {name}"))?;
            ensure!(
                inspected[0]["State"]["Running"].as_bool() == Some(true),
                "External test container is not running: {name}"
            );
        }
        let workdir = self.spec["runner_workdir"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("/workspace");
        ensure!(
            (workdir == "/workspace" || workdir.starts_with("/workspace/"))
                && !workdir.split('/').any(|s| s == ".."),
            "Runner working directory must be inside /workspace"
        );
        write_json(
            &self.env_dir.join("aaqe.override.json"),
            &json!({"services":{runner:{
                "working_dir":workdir,"volumes":[{"type":"bind","source":self.workspace,"target":"/workspace","read_only":true},{"type":"bind","source":self.results,"target":"/aaqe-results"}],"labels":{"io.aaqe.run":self.project}
            }}}),
        )?;
        if let Some(compose) = self.inline_compose()? {
            write_json(&self.env_dir.join("compose.generated.json"), &compose)?;
        }
        let mut argv = self.arguments(false)?;
        argv.extend(["config".into(), "--format".into(), "json".into()]);
        let rendered = self.checked(&argv, "config")?;
        let document: Value = serde_json::from_str(&rendered.stdout)
            .context("Docker Compose returned invalid configuration")?;
        let services = document["services"]
            .as_object()
            .context("Runner service is absent from Compose configuration")?;
        ensure!(
            services.contains_key(runner),
            "Runner service is absent from Compose configuration"
        );
        for service in services.values() {
            ensure!(
                service["container_name"].as_str().is_none_or(str::is_empty),
                "Owned Compose services must not use fixed container_name"
            );
        }
        for section in ["volumes", "networks"] {
            if let Some(resources) = document[section].as_object() {
                for resource in resources.values() {
                    if let Some(name) = resource["name"].as_str().filter(|s| !s.is_empty()) {
                        ensure!(
                            resource["external"].as_bool() == Some(true)
                                || name.starts_with(&format!("{}_", self.project)),
                            "Owned Docker resources must be scoped to the run's project"
                        );
                    }
                }
            }
        }
        let snapshot = self.env_dir.join("compose.resolved.json");
        write_json(&snapshot, &document)?;
        write_json(
            &self.env_dir.join("ownership.json"),
            &json!({"project":self.project,"external_containers":self.spec.get("external_containers").cloned().unwrap_or(json!([])),"compose_sha256":format!("{:x}",Sha256::digest(fs::read(snapshot)?)),"spec_sha256":self.spec_digest()?}),
        )?;
        Ok(())
    }
    pub fn execute_test(&self, command: &[String], test_id: &str, timeout: u64) -> Result<Value> {
        let attempt = Uuid::new_v4().simple().to_string();
        let directory = self.results.join(&attempt);
        fs::create_dir_all(&self.results)?;
        fs::create_dir(&directory)?;
        let result_dir = format!("/aaqe-results/{attempt}");
        let mut argv = self.arguments(true)?;
        argv.extend([
            "exec".into(),
            "-T".into(),
            "-e".into(),
            format!("AAQE_RESULT_DIR={result_dir}"),
            self.spec["runner_service"]
                .as_str()
                .context("No test runner service was identified")?
                .into(),
        ]);
        argv.extend(
            command
                .iter()
                .map(|part| part.replace("{result_dir}", &result_dir)),
        );
        let result = (self.execute)(&argv, timeout);
        save_result(&result, &directory)?;
        let evidence =
            json!({"test_id":test_id,"attempt":attempt,"directory":directory,"result":result});
        write_json(&directory.join("execution.json"), &evidence)?;
        Ok(evidence)
    }
    pub fn status(&self) -> Result<ProcessResult> {
        let mut argv = self.arguments(true)?;
        argv.extend([
            "ps".into(),
            "--all".into(),
            "--format".into(),
            "json".into(),
        ]);
        Ok((self.execute)(&argv, 30))
    }
    pub fn logs(&self) -> Result<()> {
        let mut argv = self.arguments(true)?;
        argv.extend(["logs".into(), "--no-color".into(), "--timestamps".into()]);
        self.checked(&argv, "logs")?;
        Ok(())
    }
    pub fn cleanup(&self) -> Result<()> {
        let path = self.env_dir.join("ownership.json");
        if !path.is_file() {
            bail!("No owned environment was created for this run");
        }
        let ownership = read_json(&path)?;
        ensure!(
            ownership["project"].as_str() == Some(&self.project),
            "Environment ownership does not match run ID"
        );
        let mut argv = self.arguments(true)?;
        argv.extend(["down".into(), "--volumes".into()]);
        self.checked(&argv, "cleanup")?;
        Ok(())
    }
}
