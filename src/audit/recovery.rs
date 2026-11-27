//! CLI recovery uses saved configuration and the original source revision.
use super::{harness::AuditHarness, interfaces::TestRuntime};
use crate::{
    agents::codex::Codex,
    config::Config,
    runtime::{compose::ComposeEnvironment, executor, tmux},
    state::{HarnessSnapshot, RunLock, now},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

/// The detached watcher survives a closed tmux session and delegates to the
/// active executor. Session identity and round checks reject delayed exits.
pub fn coordinator_exit(directory: &Path, owner: &str, round: u64) -> Result<()> {
    let _lock = RunLock::wait(directory)?;
    let saved = HarnessSnapshot::load(directory)?;
    let coordinator = directory.join("stages/coordinator");
    let invocation = crate::state::read_json(&coordinator.join("invocation.json"))?;
    if invocation["owner"] != owner || round == 0 || saved.state["resume_round"] != round {
        return Ok(());
    }
    ensure!(
        std::env::current_exe()?.canonicalize()? == executor::resolve(directory)?.canonicalize()?,
        "Executor changed before coordinator finalization; retry"
    );
    let config: Config = serde_json::from_value(saved.state["config"].clone())?;
    let agent = Codex::new(&config);
    let factory = |workspace: &Path,
                   directory: &Path,
                   id: &str,
                   spec: Value,
                   timeout: u64|
     -> Box<dyn TestRuntime> {
        Box::new(ComposeEnvironment::new(
            workspace, directory, id, spec, timeout,
        ))
    };
    let progress = |message: &str| eprintln!("{message}");
    let mut harness = AuditHarness::open(directory, &agent, &factory, &progress)?;
    if saved.state["status"] != "running" {
        return harness.finalize_after_exit();
    }
    let result = crate::state::read_json(&coordinator.join("process.json")).unwrap_or(Value::Null);
    harness.coordinator_exited(result["timed_out"] == true)
}

pub fn resume(
    directory: &Path,
    from_stage: Option<&str>,
    takeover: bool,
    progress: &dyn Fn(&str),
) -> Result<Value> {
    ensure!(
        !fs::symlink_metadata(directory)?.file_type().is_symlink(),
        "Run directories cannot be symbolic links"
    );
    let directory = directory.canonicalize()?;
    let lock = RunLock::acquire(&directory)?;
    let mut saved = HarnessSnapshot::load(&directory)?;
    ensure!(
        crate::runtime::policy::denial(&directory).is_none(),
        "Audit terminated after a safety policy denial"
    );
    let live = tmux::live_coordinator(&directory)?;
    ensure!(
        live.is_none() || saved.state["mcp_relay_protocol"] == 1,
        "Legacy coordinator cannot adopt recovery in its read-only sandbox. Preserve its history, close the old coordinator TUI, then retry this resume command to start a replacement. The Run was not modified."
    );
    if saved.state["status"] == "running" {
        ensure!(
            live.is_none(),
            "Run is already active; use the existing coordinator"
        );
        saved.state["status"] = json!("interrupted");
        saved.state["error"] = json!("Coordinator exited before finalization");
        saved.save(&directory)?;
    }
    // Normal resume remains on the pinned release. An explicit takeover is the
    // only way to switch a modern Run; legacy Runs acquire their first snapshot.
    if !takeover && !saved.state["executor"].is_null() {
        let pinned = executor::validate(&directory, &saved.state["executor"])?;
        if std::env::current_exe()?.canonicalize()? != pinned.canonicalize()? {
            drop(lock);
            let mut command = Command::new(pinned);
            command.args(["resume", "--run-dir"]).arg(&directory);
            if let Some(stage) = from_stage {
                command.args(["--from-stage", stage]);
            }
            let status = command.status()?;
            ensure!(
                matches!(status.code(), Some(0 | 1)),
                "Pinned executor could not resume this Run"
            );
            return Ok(HarnessSnapshot::load(&directory)?.state);
        }
    }
    let config: Config = serde_json::from_value(saved.state["config"].clone())?;
    let agent = Codex::new(&config);
    let factory = |workspace: &Path,
                   directory: &Path,
                   id: &str,
                   spec: Value,
                   timeout: u64|
     -> Box<dyn TestRuntime> {
        Box::new(ComposeEnvironment::new(
            workspace, directory, id, spec, timeout,
        ))
    };
    let mut harness = AuditHarness::open(&directory, &agent, &factory, progress)?;
    harness.preflight_resume(from_stage)?;
    saved = HarnessSnapshot::load(&directory)?;
    if takeover || saved.state["executor"].is_null() {
        if !saved.state["executor"].is_null() {
            ensure!(
                saved.state["executor"]["protocol"] == executor::PROTOCOL,
                "Incompatible executor protocol"
            );
        }
        let metadata = executor::capture(&directory)?;
        if !saved.state["executor_history"].is_array() {
            saved.state["executor_history"] = json!([]);
        }
        let previous = saved.state["executor"].clone();
        saved.state["executor_history"]
            .as_array_mut()
            .unwrap()
            .push(json!({"at":now(),"previous":previous,"replacement":metadata}));
        saved.state["executor"] = metadata;
        saved.save(&directory)?;
    }
    if live.is_none() {
        saved.state["mcp_relay_protocol"] = json!(1);
        saved.save(&directory)?;
    }
    harness.reload()?;
    let stage = match harness.resume(from_stage) {
        Ok(stage) => stage,
        Err(error) => {
            harness.reload()?;
            if harness.status()["state"]["status"] == "running" {
                harness.finish("blocked", &format!("Resume preparation failed: {error:#}"))?;
            }
            return Err(error);
        }
    };
    harness.reload()?;
    let round = harness.status()["state"]["resume_round"].as_u64().unwrap();
    let outcome = (|| -> Result<Value> {
        let archive = directory.join("resumes").join(format!("{round:04}"));
        let instruction = archive.join("instruction.md");
        fs::write(
            &instruction,
            format!(
                "请继续当前审计，从 {stage} 开始。恢复轮次 {round} 已启动；先读取 status，保留既有补丁和历史。若 resume_requires_execution 为 true，先 execute_tests。完成时调用 finish 更新报告。\n"
            ),
        )?;
        progress(&format!(
            "Run {}: resumed from {stage} (round {round})",
            saved.state["id"].as_str().unwrap()
        ));
        if let Some(session) = live {
            drop(lock);
            tmux::notify_coordinator(&session, &instruction).with_context(|| {
                format!(
                    "Cannot deliver resume instruction; read {}",
                    instruction.display()
                )
            })?;
            progress(&format!("Codex session: tmux attach-session -t {session}"));
            progress(&format!("Resume instruction: {}", instruction.display()));
            return Ok(harness.status()["state"].clone());
        }
        tmux::retire_dead_coordinator(&directory)?;
        let coordinator = directory.join("stages/coordinator");
        if coordinator.exists() {
            fs::rename(&coordinator, archive.join("coordinator"))?;
        }
        drop(lock);
        let outcome = agent.coordinate(&mut harness);
        let _lock = RunLock::wait(&directory)?;
        harness.reload()?;
        if harness.status()["state"]["resume_round"] != round {
            return Ok(harness.status()["state"].clone());
        }
        if harness.status()["state"]["status"] == "running" {
            let reason = outcome
                .err()
                .map(|e| format!("{e:#}"))
                .unwrap_or_else(|| "Coordinator exited without calling finish".into());
            harness
                .finish("blocked", &reason)
                .context("Resumed coordinator did not finish")?;
        }
        harness.finalize()?;
        Ok(harness.status()["state"].clone())
    })();
    if let Err(error) = &outcome
        && let Ok(_lock) = RunLock::acquire(&directory)
    {
        harness.reload()?;
        if harness.status()["state"]["status"] == "running"
            && harness.status()["state"]["resume_round"] == round
        {
            harness.finish("blocked", &format!("Resume failed: {error:#}"))?;
        }
    }
    outcome
}
