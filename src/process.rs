use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    cell::Cell,
    fs::{self, File},
    io::{Read, Write},
    os::{fd::AsRawFd, unix::process::CommandExt},
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::state::write_json;

static INTERRUPTED: OnceLock<Arc<AtomicBool>> = OnceLock::new();
static SIGNALS: OnceLock<std::result::Result<(), String>> = OnceLock::new();
thread_local! {
    static IGNORE_INTERRUPT: Cell<bool> = const { Cell::new(false) };
}

fn interrupt_flag() -> &'static Arc<AtomicBool> {
    INTERRUPTED.get_or_init(|| Arc::new(AtomicBool::new(false)))
}

pub fn install_signals() -> Result<()> {
    let result = SIGNALS.get_or_init(|| {
        for signal in [
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
        ] {
            if let Err(error) = signal_hook::flag::register(signal, Arc::clone(interrupt_flag())) {
                return Err(error.to_string());
            }
        }
        Ok(())
    });
    result.clone().map_err(anyhow::Error::msg)
}

pub fn interrupted() -> bool {
    interrupt_flag().load(Ordering::SeqCst)
}
pub fn reset_interrupt() {
    interrupt_flag().store(false, Ordering::SeqCst);
}

pub fn request_interrupt() {
    interrupt_flag().store(true, Ordering::SeqCst);
}

pub fn ignoring_interrupts<T>(function: impl FnOnce() -> T) -> T {
    struct MaskGuard<'a> {
        mask: &'a Cell<bool>,
        previous: bool,
    }
    impl Drop for MaskGuard<'_> {
        fn drop(&mut self) {
            self.mask.set(self.previous);
        }
    }
    IGNORE_INTERRUPT.with(|mask| {
        let _guard = MaskGuard {
            mask,
            previous: mask.replace(true),
        };
        function()
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessResult {
    pub argv: Vec<String>,
    pub returncode: i32,
    pub stdout: String,
    pub stderr: String,
    pub elapsed: f64,
    #[serde(default)]
    pub timed_out: bool,
}

pub fn run(
    argv: &[String],
    cwd: Option<&Path>,
    timeout: u64,
    input: Option<&str>,
) -> ProcessResult {
    run_inner(argv, cwd, timeout, input, &[], true, None)
}

pub fn run_logged(
    argv: &[String],
    cwd: &Path,
    timeout: u64,
    input: &str,
    directory: &Path,
) -> Result<ProcessResult> {
    let logs = (
        File::create(directory.join("stdout.log"))?,
        File::create(directory.join("stderr.log"))?,
    );
    let result = run_inner(argv, Some(cwd), timeout, Some(input), &[], true, Some(logs));
    save_result(&result, directory)?;
    File::create(directory.join("finished"))?;
    Ok(result)
}

pub fn run_stage_worker(directory: &Path) -> Result<ProcessResult> {
    let mut child = Command::new(crate::tmux::executable()?)
        .arg("__stage-worker")
        .arg(directory)
        .arg(std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(File::create(directory.join("worker.log"))?)
        .process_group(0)
        .spawn()?;
    loop {
        if interrupted() {
            File::create(directory.join("cancel"))?;
        }
        if let Some(status) = child.try_wait()? {
            anyhow::ensure!(
                status.success(),
                "Stage supervisor failed: {status}; see {}",
                directory.display()
            );
            return Ok(serde_json::from_value(crate::state::read_json(
                &directory.join("process.json"),
            )?)?);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

pub fn stage_worker(directory: &Path, parent: u32) -> Result<()> {
    let _lock = WorkerLock::new(&directory.join("worker-running"))?;
    let invocation = crate::state::read_json(&directory.join("invocation.json"))?;
    let argv: Vec<String> = serde_json::from_value(invocation["argv"].clone())?;
    let cwd = invocation["cwd"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing stage cwd"))?;
    let timeout = invocation["timeout"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Missing stage timeout"))?;
    let input = fs::read_to_string(directory.join("prompt.md"))?;
    let cancel = directory.join("cancel");
    thread::spawn(move || {
        loop {
            if cancel.exists() || unsafe { libc::getppid() } != parent as i32 {
                request_interrupt();
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
    });
    run_logged(&argv, Path::new(cwd), timeout, &input, directory)?;
    Ok(())
}

pub(crate) struct WorkerLock {
    path: std::path::PathBuf,
    _file: File,
}

impl WorkerLock {
    pub(crate) fn new(path: &Path) -> Result<Self> {
        let file = File::create(path)?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self {
            path: path.to_owned(),
            _file: file,
        })
    }
}

impl Drop for WorkerLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(crate) fn wait_for_worker(path: &Path) {
    if let Ok(file) = File::open(path) {
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                break;
            }
            if std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

pub fn run_env(
    argv: &[String],
    cwd: Option<&Path>,
    timeout: u64,
    input: Option<&str>,
    env: &[(String, String)],
) -> ProcessResult {
    run_inner(argv, cwd, timeout, input, env, true, None)
}

pub fn run_uninterrupted(
    argv: &[String],
    cwd: Option<&Path>,
    timeout: u64,
    input: Option<&str>,
) -> ProcessResult {
    run_inner(argv, cwd, timeout, input, &[], false, None)
}

fn read_pipe(
    mut pipe: impl Read + Send + 'static,
    sender: mpsc::Sender<Vec<u8>>,
    mut log: Option<File>,
) {
    thread::spawn(move || {
        let mut data = Vec::new();
        let mut buffer = [0u8; 65536];
        loop {
            let count = match pipe.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };
            data.extend_from_slice(&buffer[..count]);
            if let Some(log) = &mut log {
                let _ = log.write_all(&buffer[..count]);
                let _ = log.flush();
            }
        }
        let _ = sender.send(data);
    });
}

fn run_inner(
    argv: &[String],
    cwd: Option<&Path>,
    timeout: u64,
    input: Option<&str>,
    env: &[(String, String)],
    allow_interrupt: bool,
    logs: Option<(File, File)>,
) -> ProcessResult {
    let started = Instant::now();
    let mut result = ProcessResult {
        argv: argv.to_vec(),
        returncode: 127,
        stdout: String::new(),
        stderr: String::new(),
        elapsed: 0.0,
        timed_out: false,
    };
    let Some(program) = argv.first() else {
        result.stderr = "Empty command".into();
        return result;
    };
    let mut command = Command::new(program);
    command
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(env.iter().map(|(k, v)| (k, v)));
    if let Some(directory) = cwd {
        command.current_dir(directory);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            result.stderr = error.to_string();
            result.elapsed = started.elapsed().as_secs_f64();
            return result;
        }
    };
    let process_id = child.id();
    let mut source = child.stdin.take().expect("piped stdin");
    let input = input.unwrap_or_default().as_bytes().to_vec();
    let writer = thread::spawn(move || {
        let _ = source.write_all(&input);
    });
    let (stdout_send, stdout_recv) = mpsc::channel();
    let (stderr_send, stderr_recv) = mpsc::channel();
    let (stdout_log, stderr_log) = match logs {
        Some((stdout, stderr)) => (Some(stdout), Some(stderr)),
        None => (None, None),
    };
    read_pipe(
        child.stdout.take().expect("piped stdout"),
        stdout_send,
        stdout_log,
    );
    read_pipe(
        child.stderr.take().expect("piped stderr"),
        stderr_send,
        stderr_log,
    );
    let mut stdout = None;
    let mut stderr = None;
    let mut status = None;
    let mut killed = false;
    let mut cancelled = false;
    loop {
        if stdout.is_none() {
            stdout = stdout_recv.try_recv().ok();
        }
        if stderr.is_none() {
            stderr = stderr_recv.try_recv().ok();
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(value) => status = value,
                Err(error) => {
                    result.stderr = error.to_string();
                    break;
                }
            }
        }
        if status.is_some() && stdout.is_some() && stderr.is_some() {
            break;
        }
        if !killed {
            cancelled = allow_interrupt && !IGNORE_INTERRUPT.with(Cell::get) && interrupted();
            result.timed_out = !cancelled && started.elapsed() >= Duration::from_secs(timeout);
            if cancelled || result.timed_out {
                #[cfg(unix)]
                unsafe {
                    libc::killpg(process_id as i32, libc::SIGKILL);
                }
                #[cfg(not(unix))]
                let _ = child.kill();
                killed = true;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    let status = status.or_else(|| child.wait().ok());
    let _ = writer.join();
    result.stdout = String::from_utf8_lossy(&stdout.unwrap_or_default()).into_owned();
    result
        .stderr
        .push_str(&String::from_utf8_lossy(&stderr.unwrap_or_default()));
    result.returncode = if result.timed_out {
        124
    } else if cancelled {
        130
    } else {
        status
            .map(|status| {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    status
                        .code()
                        .unwrap_or_else(|| -status.signal().unwrap_or(1))
                }
                #[cfg(not(unix))]
                status.code().unwrap_or(1)
            })
            .unwrap_or(127)
    };
    result.elapsed = started.elapsed().as_secs_f64();
    result
}

pub fn save_result(result: &ProcessResult, directory: &Path) -> Result<()> {
    fs::create_dir_all(directory)?;
    fs::write(directory.join("stdout.log"), &result.stdout)?;
    fs::write(directory.join("stderr.log"), &result.stderr)?;
    write_json(
        &directory.join("process.json"),
        &serde_json::to_value(result)?,
    )
}
