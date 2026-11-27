use crate::{
    guards::patch::safe_path,
    runtime::process::{ProcessResult, ignoring_interrupts, run, run_streamed, save_result},
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
    execute: Option<Executor>,
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

pub fn command_list(value: &Value, label: &str) -> Result<Vec<Vec<String>>> {
    let commands = value
        .as_array()
        .with_context(|| format!("{label} must contain argv arrays"))?;
    ensure!(
        commands.len() <= 32,
        "Too many {label} commands (maximum 32)"
    );
    for command in commands {
        let argv = command
            .as_array()
            .with_context(|| format!("{label} must contain argv arrays"))?;
        ensure!(
            !argv.is_empty()
                && argv.len() <= 64
                && argv.iter().all(|arg| arg
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && s.len() <= 8192 && !s.contains('\0'))),
            "Invalid {label} argv"
        );
    }
    Ok(serde_json::from_value(value.clone())?)
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
            execute: None,
        }
    }

    pub fn with_executor(mut self, execute: Executor) -> Self {
        self.execute = Some(execute);
        self
    }

    fn execute_saved(
        &self,
        argv: &[String],
        timeout: u64,
        directory: &Path,
        stream: bool,
    ) -> Result<ProcessResult> {
        if self.execute.is_none() && stream {
            return run_streamed(argv, None, timeout, None, directory);
        }
        let result = self.execute_complete(argv, timeout);
        save_result(&result, directory)?;
        Ok(result)
    }

    fn execute_complete(&self, argv: &[String], timeout: u64) -> ProcessResult {
        match &self.execute {
            Some(execute) => execute(argv, timeout),
            None => run(argv, None, timeout, None),
        }
    }

    pub fn record_setup(&self, commands: &[Vec<String>]) -> Result<()> {
        self.record_setup_with_replace(commands, false)
    }

    pub fn record_setup_with_replace(&self, commands: &[Vec<String>], replace: bool) -> Result<()> {
        ensure!(
            !commands.is_empty(),
            "Dependency repair requires nonempty setup commands"
        );
        command_list(&json!(commands), "setup")?;
        ensure!(
            !self.run_dir.starts_with(&self.workspace),
            "Environment artifacts must be outside the worktree"
        );
        let mut saved = if replace {
            vec![]
        } else {
            self.setup_commands()?
        };
        for command in commands {
            if !saved.contains(command) {
                saved.push(command.clone());
            }
        }
        command_list(&json!(saved), "setup")?;
        write_json(&self.env_dir.join("dependency-setup.json"), &json!(saved))
    }

    pub fn saved_setup(&self) -> Result<Vec<Vec<String>>> {
        let path = self.env_dir.join("dependency-setup.json");
        if path.exists() {
            command_list(&read_json(&path)?, "setup")
        } else {
            Ok(vec![])
        }
    }

    fn setup_commands(&self) -> Result<Vec<Vec<String>>> {
        if self.env_dir.join("dependency-setup.json").exists() {
            self.saved_setup()
        } else {
            self.spec
                .get("setup")
                .map(|value| command_list(value, "setup"))
                .unwrap_or_else(|| Ok(vec![]))
        }
    }

    pub fn inspect(&self, probe: &Value, timeout: u64) -> Result<Value> {
        self.ensure_no_pending_setup()?;
        let runner = self.spec["runner_service"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("Prepare a runner before probing test dependencies")?;
        let mut prefix = self
            .arguments(true)
            .context("Prepare a runner before probing test dependencies")?;
        prefix.extend(["exec".into(), "-T".into(), runner.into()]);
        crate::runtime::probe::inspect_runner(
            &self.workspace,
            &self.env_dir.join("probes"),
            probe,
            timeout,
            &prefix,
            runner,
        )
    }

    fn ensure_no_pending_setup(&self) -> Result<()> {
        ensure!(
            !self.env_dir.join("setup-in-progress.json").exists(),
            "An interrupted installer requires prepare_environment recovery before using the runner"
        );
        Ok(())
    }

    fn stop_pending_setup(&self) -> Result<()> {
        let pending = self.env_dir.join("setup-in-progress.json");
        if !pending.exists() {
            return Ok(());
        }
        let mut argv = self.arguments(true)?;
        argv.extend([
            "stop".into(),
            "--timeout".into(),
            "1".into(),
            self.spec["runner_service"]
                .as_str()
                .context("Missing runner service")?
                .into(),
        ]);
        // Recovery must also run after Ctrl+C or expiry of the installation budget.
        // A successful Docker stop terminates all processes inside this owned runner.
        let evidence = self
            .env_dir
            .join("setup-stop")
            .join(Uuid::new_v4().simple().to_string());
        let result = ignoring_interrupts(|| self.execute_saved(&argv, 10, &evidence, true))?;
        ensure!(
            result.returncode == 0 && !result.timed_out,
            "Cannot stop interrupted installer; prepare_environment recovery required; evidence: {}",
            evidence.display()
        );
        fs::remove_file(pending)?;
        Ok(())
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
        self.checked_until(argv, label, None)
    }

    fn checked_until(
        &self,
        argv: &[String],
        label: &str,
        deadline: Option<std::time::Instant>,
    ) -> Result<ProcessResult> {
        let timeout = if let Some(deadline) = deadline {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .context("Environment operation time budget exhausted")?;
            // Process timeouts use whole seconds; round up only the remaining fraction.
            remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
        } else {
            self.timeout
        };
        ensure!(timeout > 0, "Environment operation time budget exhausted");
        let evidence = self
            .env_dir
            .join(label)
            .join(Uuid::new_v4().simple().to_string());
        let result = self.execute_saved(
            argv,
            timeout.min(self.timeout),
            &evidence,
            label != "config" && !label.starts_with("external-"),
        )?;
        ensure!(
            result.returncode == 0,
            "Docker {label} failed (exit {}); environment retained; evidence: {}\nstdout: {}\nstderr: {}",
            result.returncode,
            evidence.display(),
            result.stdout.chars().take(4096).collect::<String>(),
            result.stderr.chars().take(4096).collect::<String>()
        );
        Ok(result)
    }

    pub fn start(&self) -> Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(self.timeout);
        ensure!(
            !absolute_path(&self.run_dir).starts_with(absolute_path(&self.workspace)),
            "Environment artifacts must be outside the worktree"
        );
        self.validate_spec()?;
        let setup = self.setup_commands()?;
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
            self.create_snapshot(runner, deadline)?;
        }
        self.stop_pending_setup()?;
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
        argv.extend(["up".into(), "-d".into()]);
        if !setup.is_empty() {
            let mut launch = argv.clone();
            launch.extend(unique.clone());
            self.checked_until(&launch, "start", Some(deadline))?;
            for (index, command) in setup.iter().enumerate() {
                let mut install = self.arguments(true)?;
                install.extend([
                    "exec".into(),
                    "-T".into(),
                    "--user".into(),
                    "0".into(),
                    runner.into(),
                ]);
                install.extend(command.clone());
                let pending = self.env_dir.join("setup-in-progress.json");
                write_json(&pending, &json!({"command":command,"index":index}))?;
                if let Err(error) =
                    self.checked_until(&install, &format!("setup-{index}"), Some(deadline))
                {
                    self.stop_pending_setup()
                        .with_context(|| format!("{error:#}"))?;
                    return Err(error);
                }
                fs::remove_file(pending)?;
            }
        }
        argv.extend([
            "--wait".into(),
            "--wait-timeout".into(),
            crate::config::unsigned_integer(&self.spec["readiness_timeout"])
                .unwrap_or(600)
                .to_string(),
        ]);
        argv.extend(unique);
        self.checked_until(&argv, "up", Some(deadline))?;
        if let Some(commands) = self.spec["readiness"].as_array() {
            for (index, command) in commands.iter().enumerate() {
                let mut argv = self.arguments(true)?;
                argv.extend(["exec".into(), "-T".into(), runner.into()]);
                argv.extend(strings(command));
                self.checked_until(&argv, &format!("readiness-{index}"), Some(deadline))?;
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
        for label in ["readiness", "setup"] {
            if let Some(commands) = self.spec.get(label) {
                command_list(commands, label)?;
            }
        }
        Ok(())
    }

    fn create_snapshot(&self, runner: &str, deadline: std::time::Instant) -> Result<()> {
        for name in strings(&self.spec["external_containers"]) {
            let result = self.checked_until(
                &[
                    "docker".into(),
                    "inspect".into(),
                    "--type".into(),
                    "container".into(),
                    name.clone(),
                ],
                &format!("external-{}", &Uuid::new_v4().simple().to_string()[..8]),
                Some(deadline),
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
        let rendered = self.checked_until(&argv, "config", Some(deadline))?;
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
        self.ensure_no_pending_setup()?;
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
        let result = self.execute_saved(&argv, timeout, &directory, true)?;
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
        Ok(self.execute_complete(&argv, 30))
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

#[cfg(test)]
mod tests {
    use crate::{runtime::compose::ComposeEnvironment, runtime::process::ProcessResult};
    use serde_json::json;
    use std::{
        fs,
        sync::{Arc, Mutex},
    };

    #[test]
    fn dependency_setup_runs_as_root_before_readiness_and_retries_without_replacing_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let captured = calls.clone();
        let installed = Arc::new(Mutex::new(false));
        let available = installed.clone();
        let runtime = ComposeEnvironment::new(
            &workspace,
            &temp.path().join("run"),
            "setup",
            json!({
                "compose":{"services":{"runner":{"image":"fixture"}}},
                "runner_service":"runner", "setup":[["install-dependency"]],
                "readiness":[["check-dependency"]]
            }),
            5,
        )
        .with_executor(Arc::new(move |argv, _| {
            captured.lock().unwrap().push(argv.to_vec());
            let mut code = 0;
            if argv.last().is_some_and(|a| a == "install-dependency") {
                assert!(argv.windows(2).any(|a| a == ["--user", "0"]));
                if !*available.lock().unwrap() {
                    code = 1;
                }
            }
            if argv.last().is_some_and(|a| a == "check-dependency") && !*available.lock().unwrap() {
                code = 1;
            }
            ProcessResult {
                argv: argv.to_vec(),
                returncode: code,
                stdout: if argv.iter().any(|a| a == "config") {
                    r#"{"services":{"runner":{"image":"fixture"}}}"#.into()
                } else {
                    String::new()
                },
                stderr: if code != 0 {
                    "package repository unavailable".into()
                } else {
                    String::new()
                },
                elapsed: 0.0,
                timed_out: false,
            }
        }));
        let failure = runtime.start().unwrap_err().to_string();
        assert!(failure.contains("setup"));
        assert!(failure.contains("package repository unavailable"));
        assert!(
            !calls
                .lock()
                .unwrap()
                .iter()
                .any(|a| a.last().is_some_and(|s| s == "check-dependency"))
        );
        let snapshot = fs::read(runtime.env_dir.join("compose.resolved.json")).unwrap();
        *installed.lock().unwrap() = true;
        runtime.start().unwrap();
        assert_eq!(
            snapshot,
            fs::read(runtime.env_dir.join("compose.resolved.json")).unwrap()
        );
        let calls = calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .filter(|a| a.iter().any(|s| s == "config"))
                .count(),
            1
        );
        let exec: Vec<_> = calls
            .iter()
            .filter(|a| a.iter().any(|s| s == "exec"))
            .map(|a| a.last().unwrap().as_str())
            .collect();
        assert_eq!(
            exec,
            [
                "install-dependency",
                "install-dependency",
                "check-dependency"
            ]
        );
        assert_eq!(fs::read_dir(&workspace).unwrap().count(), 0);
    }

    #[test]
    fn invalid_setup_is_rejected_before_docker_or_evidence_changes() {
        for setup in [
            json!("shell text"),
            json!([[]]),
            json!([["bad\u{0}argument"]]),
            json!([[""]]),
            json!([vec!["tool"; 65]]),
        ] {
            let workspace = tempfile::tempdir().unwrap();
            let run = tempfile::tempdir().unwrap();
            let mut spec = json!({"compose":{"services":{"runner":{"image":"fixture"}}},"runner_service":"runner"});
            spec["setup"] = setup;
            let runtime = ComposeEnvironment::new(workspace.path(), run.path(), "bad", spec, 5)
                .with_executor(Arc::new(|_, _| panic!("invalid setup reached Docker")));
            assert!(runtime.start().is_err());
            assert!(!runtime.env_dir.exists());
        }
    }

    #[test]
    fn repair_recipes_are_deduplicated_validated_and_loaded_by_a_new_runtime() {
        let workspace = tempfile::tempdir().unwrap();
        let run = tempfile::tempdir().unwrap();
        let runtime = ComposeEnvironment::new(workspace.path(), run.path(), "repair", json!({}), 5);
        let install = vec![vec!["install".into(), "dependency".into()]];
        runtime.record_setup(&install).unwrap();
        runtime.record_setup(&install).unwrap();
        let before = fs::read(runtime.env_dir.join("dependency-setup.json")).unwrap();
        for invalid in [
            vec![],
            vec![vec![]],
            vec![vec!["bad\u{0}argument".into()]],
            (0..33).map(|i| vec![format!("package-{i}")]).collect(),
        ] {
            assert!(runtime.record_setup(&invalid).is_err());
            assert_eq!(
                before,
                fs::read(runtime.env_dir.join("dependency-setup.json")).unwrap()
            );
        }
        let restored =
            ComposeEnvironment::new(workspace.path(), run.path(), "repair", json!({}), 5);
        assert_eq!(restored.saved_setup().unwrap(), install);
    }

    #[test]
    fn incorrect_installation_recipe_can_be_replaced_without_changing_compose() {
        let workspace = tempfile::tempdir().unwrap();
        let run = tempfile::tempdir().unwrap();
        let runtime = ComposeEnvironment::new(
            workspace.path(),
            run.path(),
            "correction",
            json!({"setup":[["install-wrong-version"]]}),
            5,
        );
        runtime
            .record_setup_with_replace(&[vec!["install-correct-version".into()]], true)
            .unwrap();
        assert_eq!(
            runtime.saved_setup().unwrap(),
            vec![vec!["install-correct-version".to_string()]]
        );
        runtime
            .record_setup(&[vec!["install-another-package".into()]])
            .unwrap();
        assert_eq!(runtime.saved_setup().unwrap().len(), 2);
    }

    #[test]
    fn failed_installer_stop_blocks_runner_use_until_preparation_recovers() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let workspace = tempfile::tempdir().unwrap();
        let run = tempfile::tempdir().unwrap();
        let recovered = Arc::new(AtomicBool::new(false));
        let ready = recovered.clone();
        let runtime = ComposeEnvironment::new(workspace.path(), run.path(), "stop-recovery", json!({
            "compose":{"services":{"runner":{"image":"fixture"}}}, "runner_service":"runner", "setup":[["install"]]
        }), 5).with_executor(Arc::new(move |argv,_| {
            let failed = !ready.load(Ordering::SeqCst) && argv.iter().any(|s| s == "exec" || s == "stop");
            ProcessResult { argv:argv.to_vec(), returncode:if failed {124} else {0}, timed_out:failed,
                stdout:if argv.iter().any(|s| s == "config") {r#"{"services":{"runner":{"image":"fixture"}}}"#.into()} else {String::new()},
                stderr:String::new(), elapsed:0.0 }
        }));
        assert!(format!("{:#}", runtime.start().unwrap_err()).contains("interrupted installer"));
        assert!(runtime.env_dir.join("setup-in-progress.json").exists());
        assert!(
            runtime
                .inspect(&json!({"kind":"tools","tools":["python"]}), 3)
                .unwrap_err()
                .to_string()
                .contains("recovery")
        );
        assert!(
            runtime
                .execute_test(&["test".into()], "test", 3)
                .unwrap_err()
                .to_string()
                .contains("recovery")
        );
        recovered.store(true, Ordering::SeqCst);
        runtime.start().unwrap();
        assert!(!runtime.env_dir.join("setup-in-progress.json").exists());
        assert!(runtime.env_dir.join("setup-stop").is_dir());
    }

    #[test]
    fn inline_compose_is_owned_outside_worktree_and_readiness_retries_reuse_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("worktree");
        fs::create_dir(&workspace).unwrap();
        let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let captured = calls.clone();
        let fail_readiness = Arc::new(Mutex::new(true));
        let fail = fail_readiness.clone();
        let runtime = ComposeEnvironment::new(
            &workspace,
            &temp.path().join("run"),
            "retry",
            json!({
                "compose":{"services":{"runner":{"image":"alpine","command":["sleep","infinity"]}}},
                "runner_service":"runner", "readiness":[["test","-f","/ready"]]
            }),
            5,
        )
        .with_executor(Arc::new(move |argv, _| {
            captured.lock().unwrap().push(argv.to_vec());
            let readiness = argv.iter().any(|s| s == "exec");
            ProcessResult {
                argv: argv.to_vec(),
                returncode: if readiness && *fail.lock().unwrap() {
                    1
                } else {
                    0
                },
                stdout: if argv.iter().any(|s| s == "config") {
                    "{\"services\":{\"runner\":{\"image\":\"alpine\"}}}".into()
                } else {
                    String::new()
                },
                stderr: String::new(),
                elapsed: 0.0,
                timed_out: false,
            }
        }));
        assert!(
            runtime
                .start()
                .unwrap_err()
                .to_string()
                .contains("readiness")
        );
        let snapshot = fs::read(runtime.env_dir.join("compose.resolved.json")).unwrap();
        assert_eq!(fs::read_dir(&workspace).unwrap().count(), 0);
        assert!(runtime.env_dir.join("compose.generated.json").is_file());
        *fail_readiness.lock().unwrap() = false;
        runtime.start().unwrap();
        assert_eq!(
            fs::read(runtime.env_dir.join("compose.resolved.json")).unwrap(),
            snapshot
        );
        assert_eq!(
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|argv| argv.iter().any(|s| s == "config"))
                .count(),
            1
        );
        fs::write(runtime.env_dir.join("compose.resolved.json"), "{}").unwrap();
        let count = calls.lock().unwrap().len();
        assert!(
            runtime
                .start()
                .unwrap_err()
                .to_string()
                .contains("integrity")
        );
        assert_eq!(calls.lock().unwrap().len(), count);
    }

    #[test]
    fn compose_rejects_ambiguous_sources_and_run_directories_inside_workspace() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("compose.yaml"), "services: {}").unwrap();
        let outside = tempfile::tempdir().unwrap();
        let runtime = ComposeEnvironment::new(
            temp.path(),
            outside.path(),
            "ambiguous",
            json!({
                "compose_files":["compose.yaml"], "compose":{"services":{}}, "runner_service":"runner"
            }),
            5,
        );
        assert!(
            runtime
                .arguments(false)
                .unwrap_err()
                .to_string()
                .contains("both")
        );
        let inside = ComposeEnvironment::new(
            temp.path(),
            &temp.path().join("run"),
            "inside",
            json!({
                "compose":{"services":{}}, "runner_service":"runner"
            }),
            5,
        );
        assert!(inside.start().unwrap_err().to_string().contains("outside"));
        assert!(!temp.path().join("run").exists());
    }

    #[test]
    fn file_compose_retry_uses_snapshot_after_source_disappears_and_rejects_changed_spec() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("compose.yaml"), "services: {}").unwrap();
        let mut runtime = ComposeEnvironment::new(
            &workspace,
            &temp.path().join("run"),
            "file",
            json!({
                "compose_files":["compose.yaml"], "runner_service":"runner"
            }),
            5,
        )
        .with_executor(Arc::new(|argv, _| ProcessResult {
            argv: argv.to_vec(),
            returncode: 0,
            stdout: if argv.contains(&"config".into()) {
                "{\"services\":{\"runner\":{\"image\":\"alpine\"}}}".into()
            } else {
                String::new()
            },
            stderr: String::new(),
            elapsed: 0.0,
            timed_out: false,
        }));
        runtime.start().unwrap();
        fs::remove_file(workspace.join("compose.yaml")).unwrap();
        runtime.spec =
            serde_json::from_str(r#"{"runner_service":"runner","compose_files":["compose.yaml"]}"#)
                .unwrap();
        runtime.start().unwrap();
        runtime.spec["runner_service"] = json!("changed");
        assert!(
            runtime
                .start()
                .unwrap_err()
                .to_string()
                .contains("immutable")
        );
    }

    #[test]
    fn encoded_compose_objects_support_structured_agent_output() {
        let temp = tempfile::tempdir().unwrap();
        let run = tempfile::tempdir().unwrap();
        let runtime = ComposeEnvironment::new(temp.path(), run.path(), "encoded", json!({
            "compose":"{\"services\":{\"runner\":{\"image\":\"alpine\"}}}", "runner_service":"runner"
        }), 5).with_executor(Arc::new(|argv,_| ProcessResult {argv:argv.to_vec(),returncode:0,
            stdout:if argv.contains(&"config".into()) {"{\"services\":{\"runner\":{\"image\":\"alpine\"}}}".into()} else {String::new()},stderr:String::new(),elapsed:0.0,timed_out:false}));
        runtime.start().unwrap();
        let generated =
            crate::state::read_json(&runtime.env_dir.join("compose.generated.json")).unwrap();
        assert_eq!(generated["services"]["runner"]["image"], "alpine");
    }

    #[test]
    fn startup_commands_share_one_operation_deadline() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let captured = calls.clone();
        let runtime=ComposeEnvironment::new(&workspace,&temp.path().join("run"),"deadline",json!({"compose":{"services":{"runner":{"image":"fixture"}}},"runner_service":"runner","readiness":[["ready"],["ready-again"]]}),2).with_executor(Arc::new(move|argv,timeout|{
            captured.lock().unwrap().push(timeout);
            if argv.iter().any(|arg|arg=="config"){std::thread::sleep(std::time::Duration::from_millis(1100));}
            ProcessResult{argv:argv.to_vec(),returncode:0,stdout:if argv.iter().any(|arg|arg=="config"){"{\"services\":{\"runner\":{\"image\":\"fixture\"}}}".into()}else{String::new()},stderr:String::new(),elapsed:0.0,timed_out:false}
        }));
        runtime.start().unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            vec![2, 1, 1, 1],
            "render, up and readiness share the initial time budget"
        );
    }

    #[test]
    fn compose_retains_and_only_cleans_owned_resources_with_intact_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("worktree");
        fs::create_dir(&workspace).unwrap();
        fs::write(
            workspace.join("compose.yaml"),
            "services: {runner: {image: 'rust:1.100.0-slim'}}\n",
        )
        .unwrap();
        let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let capture = calls.clone();
        let runtime = ComposeEnvironment::new(&workspace, &temp.path().join("run"), "test-run", json!({
            "compose_files":["compose.yaml"], "services":["runner"], "runner_service":"runner", "external_containers":["shared-source"]
        }), 30).with_executor(Arc::new(move |argv, _| {
            capture.lock().unwrap().push(argv.to_vec());
            ProcessResult {argv: argv.to_vec(), returncode:0, stdout: if argv.iter().any(|s|s=="inspect") {
                "[{\"State\":{\"Running\":true}}]".into()
            } else if argv.iter().any(|s|s=="config") {
                "{\"services\":{\"runner\":{\"image\":\"rust:1.100.0-slim\"}}}".into()
            } else {String::new()}, stderr:String::new(),elapsed:0.0,timed_out:false}
        }));
        runtime.start().unwrap();
        assert!(
            calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c.contains(&"--wait".into()))
        );
        assert!(
            !calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c.contains(&"down".into()))
        );
        let first = runtime
            .execute_test(
                &[
                    "echo".into(),
                    "{result_dir}".into(),
                    "space argument".into(),
                ],
                "test",
                30,
            )
            .unwrap();
        let second = runtime.execute_test(&["echo".into()], "test", 30).unwrap();
        assert_ne!(first["directory"], second["directory"]);
        assert_eq!(
            first["result"]["argv"].as_array().unwrap().last().unwrap(),
            "space argument"
        );
        runtime.cleanup().unwrap();
        let deletion = calls.lock().unwrap().last().unwrap().clone();
        assert!(deletion.contains(&"down".into()) && deletion.contains(&"--volumes".into()));
        assert!(!deletion.contains(&"shared-source".into()));
        fs::write(runtime.env_dir.join("compose.resolved.json"), "{}").unwrap();
        assert!(
            runtime
                .cleanup()
                .unwrap_err()
                .to_string()
                .contains("snapshot")
        );
        fs::remove_file(runtime.env_dir.join("compose.resolved.json")).unwrap();
        assert!(
            runtime
                .cleanup()
                .unwrap_err()
                .to_string()
                .contains("snapshot")
        );
    }
}
