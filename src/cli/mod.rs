use crate::{
    audit::workflow::{reproduce, run_audit},
    config::Config,
    reporting::refresh_report,
    runtime::compose::ComposeEnvironment,
    runtime::process::{install_signals, run},
    runtime::tmux::{cleanup_tmux, worker},
    state::{RunLock, Store, now},
};
use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use serde_json::json;
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Parser)]
#[command(
    name = "aaqe",
    version,
    about = "Describe a product or module to inspect; AAQE audits it in an isolated worktree."
)]
pub struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    action: Option<Action>,
}
#[derive(Subcommand)]
enum Action {
    /// Run an audit from a natural-language prompt
    Run { prompt: Option<String> },
    /// Check local executables without launching an audit
    Doctor,
    /// Print a saved report
    Report { run_id: String },
    /// Retry publication of the accepted test patch to GitLab
    Publish { run_id: String },
    /// Rerun saved tests in the retained environment
    Reproduce {
        run_id: String,
        #[arg(long = "case")]
        case_id: Option<String>,
    },
    /// Explicitly remove this run's owned Docker environment and tmux sessions
    Cleanup { run_id: String },
    /// Inspect or enter a retained test environment
    Env {
        #[command(subcommand)]
        operation: EnvironmentAction,
    },
    #[command(name = "__tmux-worker", hide = true)]
    TmuxWorker { directory: PathBuf },
    #[command(name = "__harness-mcp", hide = true)]
    HarnessMcp { directory: PathBuf },
    #[command(name = "__stage-worker", hide = true)]
    StageWorker { directory: PathBuf, parent: u32 },
    #[command(name = "__terminal-log", hide = true)]
    TerminalLog { path: PathBuf },
}
#[derive(Subcommand)]
enum EnvironmentAction {
    Status {
        run_id: String,
    },
    Shell {
        run_id: String,
        #[arg(long)]
        service: Option<String>,
    },
}

pub fn main() -> i32 {
    if std::env::args_os().next().is_some_and(|path| {
        Path::new(&path)
            .file_name()
            .is_some_and(|name| name == "aaqe-ssh-askpass")
    }) {
        return match std::env::var("AAQE_SSH_PASSWORD") {
            Ok(password) => i32::from(writeln!(io::stdout(), "{password}").is_err()),
            Err(_) => 1,
        };
    }
    let cli = Cli::parse();
    let mcp = matches!(cli.action, Some(Action::HarnessMcp { .. }));
    match execute(cli) {
        Ok(code) => code,
        Err(error) => {
            if mcp {
                eprintln!("aaqe: {error:#}");
            } else {
                println!("aaqe: {error:#}");
            }
            2
        }
    }
}

fn execute(cli: Cli) -> Result<i32> {
    if let Some(Action::TerminalLog { path }) = &cli.action {
        crate::runtime::tmux::terminal_log(path)?;
        return Ok(0);
    }
    if let Some(Action::TmuxWorker { directory }) = &cli.action {
        worker(directory)?;
        return Ok(0);
    }
    install_signals()?;
    if let Some(Action::StageWorker { directory, parent }) = &cli.action {
        crate::runtime::process::stage_worker(directory, *parent)?;
        return Ok(0);
    }
    if let Some(Action::HarnessMcp { directory }) = &cli.action {
        crate::agents::mcp::serve(directory)?;
        return Ok(0);
    }
    let caller = std::env::current_dir()?;
    let config = Config::load(&caller, cli.config.as_deref())?;
    if cli.action.is_none() || matches!(cli.action, Some(Action::Run { .. })) {
        let prompt = match cli.action {
            Some(Action::Run {
                prompt: Some(prompt),
            }) => prompt,
            _ => {
                print!("Describe the product/module and what to check:\n> ");
                io::stdout().flush()?;
                let mut line = String::new();
                ensure!(
                    io::stdin().read_line(&mut line)? > 0,
                    "EOF when reading a line"
                );
                line.trim_end_matches(['\r', '\n']).to_owned()
            }
        };
        ensure!(!prompt.trim().is_empty(), "An audit prompt is required");
        let state = run_audit(&prompt, &caller, &config, &|message| println!("{message}"))?;
        println!(
            "Run {}: {}",
            state["id"].as_str().unwrap_or(""),
            state["status"].as_str().unwrap_or("failed")
        );
        if let Some(error) = state["error"].as_str() {
            println!("{error}");
        }
        return Ok(if state["status"] == "completed" { 0 } else { 1 });
    }
    let action = cli.action.unwrap();
    if matches!(action, Action::Doctor) {
        let mut codex = config.codex_command.clone();
        codex.push("--version".into());
        let commands: Vec<Vec<String>> = vec![
            vec!["git".into(), "--version".into()],
            codex,
            vec!["tmux".into(), "-V".into()],
            vec![
                "docker".into(),
                "version".into(),
                "--format".into(),
                "{{.Server.Version}}".into(),
            ],
            vec!["docker".into(), "compose".into(), "version".into()],
        ];
        let mut failed = false;
        for command in commands {
            let result = run(&command, None, 20, None);
            failed |= result.returncode != 0;
            println!(
                "{} {}: {}",
                if result.returncode == 0 { "OK" } else { "FAIL" },
                command[0],
                if result.stdout.is_empty() {
                    result.stderr.trim()
                } else {
                    result.stdout.trim()
                }
            );
        }
        return Ok(i32::from(failed));
    }
    let run_id = match &action {
        Action::Report { run_id }
        | Action::Publish { run_id }
        | Action::Reproduce { run_id, .. }
        | Action::Cleanup { run_id } => run_id,
        Action::Env { operation } => match operation {
            EnvironmentAction::Status { run_id } | EnvironmentAction::Shell { run_id, .. } => {
                run_id
            }
        },
        _ => unreachable!(),
    };
    let store = Store::new(&config.state_dir);
    let _lock = if matches!(action, Action::Cleanup { .. } | Action::Report { .. }) {
        Some(RunLock::acquire(&store.path(run_id)?)?)
    } else {
        None
    };
    let mut state = store.load(run_id)?;
    let directory = store.path(state["id"].as_str().context("Missing run identity")?)?;
    match action {
        Action::Publish { .. } => {
            let _lock = RunLock::acquire(&directory)?;
            let mut saved = crate::state::HarnessSnapshot::load(&directory)?;
            let config: Config = serde_json::from_value(saved.state["config"].clone())?;
            let result = crate::runtime::publication::publish(
                &directory,
                &mut saved.state,
                &saved.artifacts,
                &saved.execution,
                &config,
            );
            saved.save(&directory)?;
            refresh_report(&directory, &saved.state)?;
            result?;
            println!(
                "GitLab MR: {}",
                saved.state["publication"]["mr_url"]
                    .as_str()
                    .context("Missing GitLab MR URL")?
            );
            return Ok(0);
        }
        Action::Report { .. } => {
            if directory.join("harness.json").exists() || !directory.join("report.md").is_file() {
                refresh_report(&directory, &state)?;
            }
            println!("{}", std::fs::read_to_string(directory.join("report.md"))?);
            return Ok(0);
        }
        Action::Reproduce { case_id, .. } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&reproduce(&store, &mut state, case_id.as_deref())?)?
            );
            return Ok(0);
        }
        Action::Cleanup { .. } => {
            ensure!(
                state["status"] != "initializing" && state["status"] != "running",
                "Cannot clean resources while the audit is running"
            );
            ensure!(
                state["audit_finalized"] != false,
                "Cannot clean resources while audit finalization is pending"
            );
            cleanup_tmux(&directory)?;
            if state["tmux_cleaned_at"].is_null() {
                state["tmux_cleaned_at"] = json!(now());
            }
            store.save(&state)?;
            if state["environment_retained"] != true || !state["environment_cleaned_at"].is_null() {
                refresh_report(&directory, &state)?;
                println!("Owned tmux sessions cleaned; reports, branch and worktree retained.");
                return Ok(0);
            }
        }
        _ => {}
    }
    let root = state["workspace"]["path"]
        .as_str()
        .context("This run has no Docker test environment")?;
    ensure!(
        state["environment"].is_object(),
        "This run has no Docker test environment"
    );
    let saved_config: Config = serde_json::from_value(state["config"].clone())?;
    let runtime = ComposeEnvironment::new(
        Path::new(root),
        &directory,
        state["id"].as_str().unwrap(),
        state["environment"].clone(),
        saved_config.docker_timeout,
    );
    match action {
        Action::Cleanup { .. } => {
            let cleanup = runtime.cleanup();
            if cleanup.is_ok() {
                state["environment_cleaned_at"] = json!(now());
                state["environment_retained"] = json!(false);
            }
            store.save(&state)?;
            let report = refresh_report(&directory, &state);
            cleanup?;
            report.context("Resources cleaned and state saved, but report refresh failed")?;
            println!(
                "Owned Docker resources and tmux sessions cleaned; reports, branch and worktree retained."
            );
            Ok(0)
        }
        Action::Env {
            operation: EnvironmentAction::Status { .. },
        } => {
            let result = runtime.status()?;
            println!(
                "{}",
                if result.stdout.is_empty() {
                    result.stderr
                } else {
                    result.stdout
                }
            );
            Ok(i32::from(result.returncode != 0))
        }
        Action::Env {
            operation: EnvironmentAction::Shell { service, .. },
        } => {
            let mut argv = runtime.arguments(true)?;
            argv.extend([
                "exec".into(),
                service.unwrap_or_else(|| {
                    state["environment"]["runner_service"]
                        .as_str()
                        .unwrap_or("")
                        .into()
                }),
                "sh".into(),
            ]);
            let status = Command::new(&argv[0]).args(&argv[1..]).status()?;
            Ok(status.code().unwrap_or(1))
        }
        _ => bail!("Unsupported command"),
    }
}
