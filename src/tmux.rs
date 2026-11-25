use crate::{
    process::{self, ProcessResult},
    state::{read_json, write_json},
};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::json;
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    os::unix::process::{CommandExt, ExitStatusExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_owned()).collect()
}
fn quoted(argument: &str) -> String {
    shell_escape::unix::escape(argument.into()).into_owned()
}
fn shell_join(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|arg| quoted(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

// Session control must still work after the process-wide interrupt flag is set.
fn tmux(arguments: &[&str], timeout: u64) -> ProcessResult {
    let mut argv = vec!["tmux".to_owned()];
    argv.extend(arguments.iter().map(|argument| (*argument).to_owned()));
    let mut result = process::run_uninterrupted(&argv, None, timeout, None);
    if result.timed_out {
        result
            .stderr
            .push_str(&format!("tmux command timed out after {timeout}s"));
    } else if result.returncode == 127 {
        result.stderr = format!(
            "Codex requires tmux; install tmux and run AAQE on Linux or WSL2: {}",
            result.stderr
        );
    }
    result
}

fn executable() -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    // Integration tests run from target/{profile}/deps, whereas the worker is
    // the ordinary installed executable built alongside the test harness.
    if executable
        .parent()
        .is_some_and(|p| p.file_name().is_some_and(|n| n == "deps"))
    {
        let binary = executable.parent().unwrap().parent().unwrap().join("aaqe");
        if binary.is_file() {
            return Ok(binary);
        }
    }
    Ok(executable)
}

struct WorkerGuard<'a> {
    session: &'a str,
    directory: &'a Path,
    completed: bool,
}
impl Drop for WorkerGuard<'_> {
    fn drop(&mut self) {
        if !self.completed {
            stop_worker(self.session, self.directory);
        }
    }
}

pub fn run_tmux(
    argv: &[String],
    cwd: &Path,
    directory: &Path,
    input_path: &Path,
    timeout: u64,
    progress: &dyn Fn(&str),
) -> Result<ProcessResult> {
    fs::create_dir_all(directory)?;
    let directory = directory.canonicalize()?;
    let cwd = cwd.canonicalize()?;
    let input_path = input_path.canonicalize()?;
    let stage = directory
        .file_name()
        .context("Stage directory has no name")?
        .to_string_lossy();
    let session = format!(
        "aaqe-{}-{}",
        stage,
        &Uuid::new_v4().simple().to_string()[..12]
    );
    let attach = format!("tmux attach-session -t {}", quoted(&session));
    write_json(
        &directory.join("tmux.json"),
        &json!({"session":session,"attach_command":attach}),
    )?;
    write_json(
        &directory.join("invocation.json"),
        &json!({"argv":argv,"cwd":cwd,"input_path":input_path,"timeout":timeout}),
    )?;
    let _ = fs::remove_file(directory.join("finished"));
    let mut environment = strings(&["env", "-i"]);
    environment.extend(
        std::env::vars()
            .filter(|(name, _)| name != "TMUX" && name != "TMUX_PANE")
            .map(|(name, value)| format!("{name}={value}")),
    );
    let command = format!(
        "exec {} TMUX=\"$TMUX\" TMUX_PANE=\"$TMUX_PANE\" {}",
        shell_join(&environment),
        shell_join(&[
            executable()?.display().to_string(),
            "__tmux-worker".to_owned(),
            directory.display().to_string()
        ])
    );
    let launched = tmux(
        &[
            "new-session",
            "-d",
            "-s",
            &session,
            "-c",
            &cwd.display().to_string(),
            &command,
            ";",
            "set-option",
            "-t",
            &session,
            "@aaqe_directory",
            &directory.display().to_string(),
        ],
        20,
    );
    let mut guard = WorkerGuard {
        session: &session,
        directory: &directory,
        completed: false,
    };
    if launched.returncode != 0 {
        bail!(
            "Cannot start Codex tmux session: {}",
            launched.stderr.trim()
        );
    }
    progress(&format!("Codex session: {attach}"));
    let deadline = Instant::now() + Duration::from_secs(timeout.saturating_add(20));
    while !directory.join("finished").exists() {
        if process::interrupted() {
            bail!("Interrupted by user");
        }
        if tmux(&["has-session", "-t", &format!("={session}")], 5).returncode != 0 {
            bail!(
                "Codex tmux session disappeared; see {}",
                directory.display()
            );
        }
        if Instant::now() >= deadline {
            bail!(
                "Codex tmux worker did not finish before timeout; see {}",
                directory.display()
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
    let result = serde_json::from_value(read_json(&directory.join("process.json"))?)?;
    guard.completed = true;
    Ok(result)
}

pub fn cleanup_tmux(directory: &Path) -> Result<()> {
    let directory = directory.canonicalize()?;
    let stages = directory.join("stages");
    if !stages.exists() {
        return Ok(());
    }
    let pattern = Regex::new(r"^aaqe-[A-Za-z0-9_-]+-[0-9a-f]{12}$")?;
    let mut metadata_paths = fs::read_dir(&stages)?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .map(|e| e.path().join("tmux.json"))
        .filter(|p| p.exists())
        .collect::<Vec<_>>();
    metadata_paths.sort();
    let mut sessions = Vec::new();
    for metadata in metadata_paths {
        if !metadata.canonicalize()?.starts_with(&directory) {
            bail!("Invalid tmux metadata path: {}", metadata.display());
        }
        let value = read_json(&metadata)?;
        let session = value["session"]
            .as_str()
            .filter(|s| pattern.is_match(s))
            .with_context(|| format!("Invalid tmux session in {}", metadata.display()))?;
        sessions.push((
            session.to_owned(),
            metadata.parent().unwrap().canonicalize()?,
        ));
    }
    if sessions.is_empty() {
        return Ok(());
    }
    let listed = tmux(&["list-sessions", "-F", "#{session_name}"], 5);
    if listed.returncode != 0 {
        let error = &listed.stderr;
        if listed.returncode == 1
            && (error.contains("no server running") || error.contains("No such file or directory"))
        {
            return Ok(());
        }
        bail!("Cannot list tmux sessions: {}", error.trim());
    }
    let listing = &listed.stdout;
    let existing: HashSet<_> = listing.lines().collect();
    let mut owned = Vec::new();
    for (session, stage) in sessions {
        if !existing.contains(session.as_str()) {
            continue;
        }
        let target = format!("={session}");
        let owner = tmux(
            &["show-options", "-qv", "-t", &session, "@aaqe_directory"],
            5,
        );
        if owner.returncode != 0 || owner.stdout.trim_end_matches('\n') != stage.to_string_lossy() {
            bail!("Tmux session ownership does not match stage directory: {session}");
        }
        owned.push(target);
    }
    for target in owned {
        let deleted = tmux(&["kill-session", "-t", &target], 5);
        if deleted.returncode != 0 {
            bail!(
                "Cannot clean tmux session {target}: {}",
                deleted.stderr.trim()
            );
        }
    }
    Ok(())
}

fn stop_worker(session: &str, directory: &Path) {
    let pane = tmux(&["display-message", "-p", "-t", session, "#{pane_pid}"], 5);
    if pane.returncode != 0 {
        return;
    }
    if let Ok(pid) = pane.stdout.trim().parse::<i32>() {
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !directory.join("finished").exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    if !directory.join("finished").exists() {
        let _ = tmux(&["kill-session", "-t", &format!("={session}")], 5);
    }
}

fn pump(
    mut source: impl Read + Send + 'static,
    mut log: File,
    stderr: bool,
    finished: mpsc::Sender<()>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0u8; 65536];
        loop {
            let count = match source.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let _ = log.write_all(&buffer[..count]);
            let _ = log.flush();
            if stderr {
                let _ = std::io::stderr().write_all(&buffer[..count]);
                let _ = std::io::stderr().flush();
            } else {
                let _ = std::io::stdout().write_all(&buffer[..count]);
                let _ = std::io::stdout().flush();
            }
        }
        let _ = finished.send(());
    })
}

pub fn worker(directory: &Path) -> Result<()> {
    let invocation = read_json(&directory.join("invocation.json"))?;
    let argv: Vec<String> = serde_json::from_value(invocation["argv"].clone())?;
    let cwd = invocation["cwd"].as_str().context("Missing worker cwd")?;
    let input = invocation["input_path"]
        .as_str()
        .context("Missing worker input path")?;
    let timeout = invocation["timeout"]
        .as_u64()
        .context("Missing worker timeout")?;
    let started = Instant::now();
    let _ = tmux(&["set-option", "-w", "remain-on-exit", "on"], 5);
    let interrupted = Arc::new(AtomicBool::new(false));
    let mut registrations = Vec::new();
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        registrations.push(signal_hook::flag::register(signal, interrupted.clone())?);
    }
    let stdout_path = directory.join("stdout.log");
    let stderr_path = directory.join("stderr.log");
    let stdout_log = File::create(&stdout_path)?;
    let mut stderr_log = File::create(&stderr_path)?;
    let mut command = Command::new(argv.first().context("Empty worker command")?);
    command
        .args(&argv[1..])
        .current_dir(cwd)
        .stdin(File::open(input)?)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut timed_out = false;
    let returncode = match command.spawn() {
        Err(error) => {
            let message = format!("{}: {error}\n", argv[0]);
            stderr_log.write_all(message.as_bytes())?;
            let _ = std::io::stderr().write_all(message.as_bytes());
            127
        }
        Ok(mut child) => {
            let (sender, receiver) = mpsc::channel();
            let stdout_thread = pump(
                child.stdout.take().unwrap(),
                stdout_log,
                false,
                sender.clone(),
            );
            let stderr_thread = pump(child.stderr.take().unwrap(), stderr_log, true, sender);
            let mut streams = 0;
            let mut status = None;
            let mut killed = false;
            loop {
                if !timed_out && started.elapsed() >= Duration::from_secs(timeout) {
                    timed_out = true;
                }
                if !killed && (timed_out || interrupted.load(Ordering::Relaxed)) {
                    unsafe {
                        libc::killpg(child.id() as i32, libc::SIGKILL);
                    }
                    killed = true;
                }
                while receiver.try_recv().is_ok() {
                    streams += 1;
                }
                if status.is_none() {
                    status = child.try_wait()?;
                }
                if status.is_some() && streams == 2 {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            let status = status.unwrap();
            status
                .code()
                .unwrap_or_else(|| -status.signal().unwrap_or(1))
        }
    };
    for registration in registrations {
        signal_hook::low_level::unregister(registration);
    }
    let result = ProcessResult {
        argv,
        returncode: if timed_out {
            124
        } else if interrupted.load(Ordering::Relaxed) {
            130
        } else {
            returncode
        },
        stdout: String::from_utf8_lossy(&fs::read(stdout_path)?).into_owned(),
        stderr: String::from_utf8_lossy(&fs::read(stderr_path)?).into_owned(),
        elapsed: started.elapsed().as_secs_f64(),
        timed_out,
    };
    write_json(
        &directory.join("process.json"),
        &serde_json::to_value(&result)?,
    )?;
    let _ = writeln!(
        std::io::stdout(),
        "\nAAQE Codex finished (exit {}); logs: {}",
        result.returncode,
        directory.display()
    );
    File::create(directory.join("finished"))?;
    Ok(())
}
