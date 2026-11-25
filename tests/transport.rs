#![cfg(unix)]

use aaqe::{
    codex::Codex,
    config::Config,
    state::{read_json, write_json},
    tmux::{cleanup_tmux, run_tmux},
};
use serde_json::json;
use std::{
    fs,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

struct Sessions(TempDir);
impl Sessions {
    fn new() -> Self {
        Self(TempDir::new().unwrap())
    }
    fn path(&self) -> &Path {
        self.0.path()
    }
}
impl Drop for Sessions {
    fn drop(&mut self) {
        fn clean(path: &Path) {
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        clean(&path);
                    } else if path.file_name().is_some_and(|s| s == "tmux.json")
                        && let Ok(v) = read_json(&path)
                        && let Some(s) = v["session"].as_str()
                    {
                        let _ = Command::new("tmux")
                            .args(["kill-session", "-t", s])
                            .output();
                    }
                }
            }
        }
        clean(self.path());
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}
fn ready(path: &Path) {
    let limit = Instant::now() + Duration::from_secs(8);
    while !path.exists() && Instant::now() < limit {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(path.exists(), "not ready: {}", path.display());
}

#[test]
fn tmux_streams_logs_and_preserves_arguments() {
    let dir = Sessions::new();
    let workspace = dir.path().join("workspace 'quotes $dollar");
    fs::create_dir(&workspace).unwrap();
    let stage = dir.path().join("stages/bug-hunter");
    fs::create_dir_all(&stage).unwrap();
    let prompt = stage.join("prompt.md");
    fs::write(&prompt, "stage input\n").unwrap();
    let argument = "; touch UNEXPECTED $(echo unsafe)";
    thread::scope(|scope| {
        let handle = scope.spawn(|| run_tmux(&strings(&["sh", "-c", "cat; printf '%s\\n' \"$1\"; printf 'live diagnostic\\n' >&2; touch ready; while [ ! -f release ]; do sleep .02; done", "sh", argument]), &workspace, &stage, &prompt, 10, &|_| {}));
        ready(&workspace.join("ready"));
        let stdout = fs::read_to_string(stage.join("stdout.log")).unwrap();
        assert_eq!(stdout, format!("stage input\n{argument}\n"));
        assert_eq!(
            fs::read_to_string(stage.join("stderr.log")).unwrap(),
            "live diagnostic\n"
        );
        assert!(!handle.is_finished());
        fs::write(workspace.join("release"), "").unwrap();
        assert_eq!(handle.join().unwrap().unwrap().returncode, 0);
    });
    assert!(!workspace.join("UNEXPECTED").exists());
    let session = read_json(&stage.join("tmux.json")).unwrap();
    let pane = Command::new("tmux")
        .args([
            "capture-pane",
            "-p",
            "-S",
            "-",
            "-t",
            session["session"].as_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&pane.stdout).contains("live diagnostic"));
    cleanup_tmux(dir.path()).unwrap();
    cleanup_tmux(dir.path()).unwrap();
}

#[test]
fn tmux_preserves_exit_and_spawn_errors() {
    let dir = Sessions::new();
    for (name, command, code) in [
        (
            "failed",
            strings(&["sh", "-c", "echo diagnostic >&2; exit 23"]),
            23,
        ),
        ("missing", strings(&["aaqe-nonexistent-executable"]), 127),
    ] {
        let stage = dir.path().join("stages").join(name);
        fs::create_dir_all(&stage).unwrap();
        let prompt = stage.join("prompt.md");
        fs::write(&prompt, "").unwrap();
        let result = run_tmux(&command, dir.path(), &stage, &prompt, 5, &|_| {}).unwrap();
        assert_eq!(result.returncode, code);
        assert!(!result.timed_out);
        assert!(!result.stderr.is_empty());
    }
}

#[test]
fn timeout_kills_child_group_and_keeps_evidence() {
    let dir = Sessions::new();
    let stage = dir.path().join("stages/bug-hunter");
    fs::create_dir_all(&stage).unwrap();
    let prompt = stage.join("prompt.md");
    fs::write(&prompt, "").unwrap();
    let result = run_tmux(
        &strings(&[
            "sh",
            "-c",
            "sleep 30 & echo $! > child.pid; echo before-timeout; wait",
        ]),
        dir.path(),
        &stage,
        &prompt,
        1,
        &|_| {},
    )
    .unwrap();
    assert_eq!(result.returncode, 124);
    assert!(result.timed_out);
    assert_eq!(result.stdout, "before-timeout\n");
    let pid = fs::read_to_string(dir.path().join("child.pid")).unwrap();
    if let Ok(stat) = fs::read_to_string(format!("/proc/{}/stat", pid.trim())) {
        assert_eq!(stat.split_whitespace().nth(2), Some("Z"));
    }
    assert_eq!(
        read_json(&stage.join("process.json")).unwrap()["timed_out"],
        true
    );
}

#[test]
fn codex_bundles_skills_and_validates_output() {
    let dir = Sessions::new();
    let script = dir.path().join("codex.sh");
    fs::write(&script, "#!/bin/sh\ncat > received.txt\noutput=\nwhile [ $# -gt 0 ]; do\nif [ \"$1\" = --output-last-message ]; then shift; output=$1; fi\nshift\ndone\nprintf '%s' '{\"summary\":\"done\",\"candidates\":[],\"excluded_areas\":[]}' > \"$output\"\necho visible-output\necho visible-diagnostic >&2\n").unwrap();
    let config = Config {
        codex_command: vec!["sh".into(), script.display().to_string()],
        ..Config::default()
    };
    let stage = dir.path().join("stages/bug-hunter");
    let artifact = Codex::new(&config)
        .invoke(
            "bug-hunter",
            dir.path(),
            &stage,
            &json!({"prompt":"check the module"}),
            &|message| {
                assert!(message.contains("tmux attach-session"));
            },
        )
        .unwrap();
    assert_eq!(artifact["summary"], "done");
    let prompt = fs::read_to_string(dir.path().join("received.txt")).unwrap();
    assert!(
        prompt.contains("check the module")
            && prompt.contains("Inputs")
            && prompt.contains("Outputs")
    );
    let invocation = read_json(&stage.join("invocation.json")).unwrap();
    assert!(
        invocation["argv"]
            .as_array()
            .unwrap()
            .contains(&json!("--ephemeral"))
    );
    assert!(
        invocation["argv"]
            .as_array()
            .unwrap()
            .contains(&json!("read-only"))
    );
    assert!(stage.join("SKILL.md").is_file());
}

#[test]
fn cleanup_refuses_foreign_ownership_and_target_patterns() {
    let dir = Sessions::new();
    let stage = dir.path().join("owner/stages/bug-hunter");
    fs::create_dir_all(&stage).unwrap();
    let prompt = stage.join("prompt.md");
    fs::write(&prompt, "").unwrap();
    run_tmux(&strings(&["true"]), dir.path(), &stage, &prompt, 5, &|_| {}).unwrap();
    let metadata = read_json(&stage.join("tmux.json")).unwrap();
    write_json(
        &dir.path().join("other/stages/bug-hunter/tmux.json"),
        &metadata,
    )
    .unwrap();
    assert!(
        cleanup_tmux(&dir.path().join("other"))
            .unwrap_err()
            .to_string()
            .contains("ownership")
    );
    write_json(
        &dir.path().join("invalid/stages/bug-hunter/tmux.json"),
        &json!({"session":"aaqe-"}),
    )
    .unwrap();
    assert!(
        cleanup_tmux(&dir.path().join("invalid"))
            .unwrap_err()
            .to_string()
            .contains("Invalid tmux session")
    );
}

#[test]
fn deleting_live_session_stops_its_process() {
    let dir = Sessions::new();
    let stage = dir.path().join("stages/bug-hunter");
    fs::create_dir_all(&stage).unwrap();
    let prompt = stage.join("prompt.md");
    fs::write(&prompt, "").unwrap();
    thread::scope(|scope| {
        let handle = scope.spawn(|| {
            run_tmux(
                &strings(&["sh", "-c", "echo $$ > codex.pid; sleep 30 & wait"]),
                dir.path(),
                &stage,
                &prompt,
                10,
                &|_| {},
            )
        });
        ready(&dir.path().join("codex.pid"));
        let session = read_json(&stage.join("tmux.json")).unwrap();
        assert!(
            Command::new("tmux")
                .args(["kill-session", "-t", session["session"].as_str().unwrap()])
                .status()
                .unwrap()
                .success()
        );
        let _ = handle.join().unwrap();
    });
    let pid = fs::read_to_string(dir.path().join("codex.pid")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let path = format!("/proc/{}", pid.trim());
    while Path::new(&path).exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !Path::new(&path).exists(),
        "deleting tmux left Codex running"
    );
}

#[test]
fn callback_panic_cancels_just_started_worker() {
    let dir = Sessions::new();
    let stage = dir.path().join("stages/bug-hunter");
    fs::create_dir_all(&stage).unwrap();
    let prompt = stage.join("prompt.md");
    fs::write(&prompt, "").unwrap();
    let result = std::panic::catch_unwind(|| {
        run_tmux(
            &strings(&["sh", "-c", "sleep .5; touch leaked"]),
            dir.path(),
            &stage,
            &prompt,
            5,
            &|_| panic!("cancel progress"),
        )
    });
    assert!(result.is_err());
    thread::sleep(Duration::from_millis(700));
    assert!(!dir.path().join("leaked").exists());
}

#[test]
fn codex_rejects_invalid_structured_output() {
    let dir = Sessions::new();
    let script = dir.path().join("invalid.sh");
    fs::write(&script, "#!/bin/sh\nwhile [ $# -gt 0 ]; do\nif [ \"$1\" = --output-last-message ]; then shift; output=$1; fi\nshift\ndone\nprintf '{}' > \"$output\"\n").unwrap();
    let config = Config {
        codex_command: vec!["sh".into(), script.display().to_string()],
        ..Config::default()
    };
    let error = Codex::new(&config)
        .invoke(
            "bug-hunter",
            dir.path(),
            &dir.path().join("stages/bug-hunter"),
            &json!({}),
            &|_| {},
        )
        .unwrap_err();
    assert!(error.to_string().contains("contract"));
}

#[test]
fn worker_environment_does_not_inherit_stale_tmux_server_variables() {
    let dir = Sessions::new();
    let stage = dir.path().join("stages/bug-hunter");
    fs::create_dir_all(&stage).unwrap();
    let prompt = stage.join("prompt.md");
    fs::write(&prompt, "").unwrap();
    let seed = format!("aaqe-test-seed-{}", uuid::Uuid::new_v4().simple());
    assert!(
        Command::new("tmux")
            .args(["new-session", "-d", "-s", &seed, "sleep 30"])
            .status()
            .unwrap()
            .success()
    );
    let variable = format!("AAQE_STALE_{}", uuid::Uuid::new_v4().simple());
    assert!(
        Command::new("tmux")
            .args(["set-environment", "-g", &variable, "stale server value"])
            .status()
            .unwrap()
            .success()
    );
    let command = format!("printf '%s\\n' \"${{{variable}:-unset}}\"");
    let result = run_tmux(
        &strings(&["sh", "-c", &command]),
        dir.path(),
        &stage,
        &prompt,
        5,
        &|_| {},
    );
    let _ = Command::new("tmux")
        .args(["set-environment", "-gu", &variable])
        .output();
    let _ = Command::new("tmux")
        .args(["kill-session", "-t", &seed])
        .output();
    let result = result.unwrap();
    assert_eq!(result.returncode, 0);
    assert_eq!(result.stdout, "unset\n");
}

#[test]
fn interrupting_audit_preserves_report_and_session_and_stops_codex() {
    let dir = Sessions::new();
    let repository = dir.path().join("repository");
    let remote = dir.path().join("remote.git");
    fs::create_dir(&repository).unwrap();
    let git = |path: &Path, argv: &[&str]| aaqe::gitops::git(path, argv).unwrap();
    git(dir.path(), &["init", "--bare", remote.to_str().unwrap()]);
    git(&repository, &["init", "-b", "main"]);
    git(&repository, &["config", "user.name", "Test User"]);
    git(
        &repository,
        &["config", "user.email", "test@example.invalid"],
    );
    fs::write(repository.join("README.md"), "A test product.\n").unwrap();
    git(&repository, &["add", "."]);
    git(&repository, &["commit", "-m", "initial"]);
    git(
        &repository,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&repository, &["push", "-u", "origin", "main"]);
    let script = dir.path().join("codex.sh");
    let pid_path = dir.path().join("codex.pid");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$AAQE_TEST_CURRENT\" >&2\necho $$ > \"$1\"\nsleep 30 & wait\n",
    )
    .unwrap();
    let state_root = dir.path().join("runs");
    let config = dir.path().join("config.yaml");
    fs::write(&config, serde_json::to_string(&json!({"state_dir":state_root,"codex":{"command":["sh",script,pid_path]},"timeouts":{"agent":20}})).unwrap()).unwrap();
    let mut parent = Command::new(env!("CARGO_BIN_EXE_aaqe"))
        .current_dir(&repository)
        .env(
            "AAQE_TEST_CURRENT",
            "current caller 'quotes $literal $(echo unsafe)",
        )
        .args([
            "--config",
            config.to_str().unwrap(),
            "run",
            "check this module",
        ])
        .stdout(fs::File::create(dir.path().join("parent.stdout")).unwrap())
        .stderr(fs::File::create(dir.path().join("parent.stderr")).unwrap())
        .spawn()
        .unwrap();
    ready(&pid_path);
    unsafe {
        libc::kill(parent.id() as i32, libc::SIGINT);
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    let status = loop {
        if let Some(status) = parent.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = parent.kill();
            let _ = parent.wait();
            panic!("interrupted AAQE failed to stop");
        }
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        status.code(),
        Some(1),
        "{}",
        fs::read_to_string(dir.path().join("parent.stdout")).unwrap()
    );
    let run = fs::read_dir(&state_root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        read_json(&run.join("run.json")).unwrap()["status"],
        "interrupted"
    );
    assert_eq!(
        read_json(&run.join("report.json")).unwrap()["status"],
        "interrupted"
    );
    assert!(run.join("report.md").is_file());
    let stage = run.join("stages/coordinator");
    let result = read_json(&stage.join("process.json")).unwrap();
    assert_eq!(result["returncode"], 130);
    assert_eq!(result["timed_out"], false);
    assert_eq!(
        result["stderr"],
        "current caller 'quotes $literal $(echo unsafe)\n"
    );
    let pid = fs::read_to_string(pid_path).unwrap();
    assert!(!Path::new(&format!("/proc/{}", pid.trim())).exists());
    let session = read_json(&stage.join("tmux.json")).unwrap();
    assert!(
        Command::new("tmux")
            .args(["has-session", "-t", session["session"].as_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    cleanup_tmux(&run).unwrap();
    assert!(stage.join("stdout.log").is_file());
}

#[test]
fn stuck_tmux_control_times_out_and_kills_its_descendants_after_interrupt() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Sessions::new();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let fake_tmux = bin.join("tmux");
    fs::write(&fake_tmux, "#!/bin/sh\necho $$ > \"$AAQE_CONTROL_PID\"\n/bin/sleep 30 &\necho $! > \"$AAQE_CONTROL_CHILD_PID\"\nwait\n").unwrap();
    fs::set_permissions(&fake_tmux, fs::Permissions::from_mode(0o755)).unwrap();
    let runs = dir.path().join("runs");
    let run = runs.join("bounded-control");
    write_json(
        &run.join("run.json"),
        &json!({"id":"bounded-control","status":"completed"}),
    )
    .unwrap();
    write_json(
        &run.join("stages/bug-hunter/tmux.json"),
        &json!({"session":"aaqe-bug-hunter-0123456789ab"}),
    )
    .unwrap();
    let pid = dir.path().join("control.pid");
    let child_pid = dir.path().join("control-child.pid");
    let started = Instant::now();
    let mut command = Command::new(env!("CARGO_BIN_EXE_aaqe"))
        .current_dir(dir.path())
        .args([
            "--state-dir",
            runs.to_str().unwrap(),
            "cleanup",
            "bounded-control",
        ])
        .env("PATH", &bin)
        .env("AAQE_CONTROL_PID", &pid)
        .env("AAQE_CONTROL_CHILD_PID", &child_pid)
        .stdout(fs::File::create(dir.path().join("control.stdout")).unwrap())
        .stderr(fs::File::create(dir.path().join("control.stderr")).unwrap())
        .spawn()
        .unwrap();
    ready(&child_pid);
    unsafe {
        libc::kill(command.id() as i32, libc::SIGINT);
    }
    let deadline = Instant::now() + Duration::from_secs(7);
    let status = loop {
        if let Some(status) = command.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = command.kill();
            let _ = command.wait();
            panic!("tmux control exceeded its timeout");
        }
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(2));
    assert!(started.elapsed() < Duration::from_secs(7));
    assert!(
        fs::read_to_string(dir.path().join("control.stdout"))
            .unwrap()
            .contains("timed out after 5s")
    );
    for file in [pid, child_pid] {
        let pid = fs::read_to_string(file).unwrap();
        if let Ok(stat) = fs::read_to_string(format!("/proc/{}/stat", pid.trim())) {
            assert_eq!(
                stat.split_whitespace().nth(2),
                Some("Z"),
                "control process survived timeout"
            );
        }
    }
}

#[test]
fn cleanup_discovers_nested_attempts_and_refuses_directory_symlinks() {
    let dir = Sessions::new();
    let stage = dir.path().join("stages/test-developer/attempts/0002");
    fs::create_dir_all(&stage).unwrap();
    let prompt = stage.join("prompt.md");
    fs::write(&prompt, "").unwrap();
    run_tmux(&strings(&["true"]), dir.path(), &stage, &prompt, 5, &|_| {}).unwrap();
    let session = read_json(&stage.join("tmux.json")).unwrap()["session"]
        .as_str()
        .unwrap()
        .to_owned();
    cleanup_tmux(dir.path()).unwrap();
    assert!(
        !Command::new("tmux")
            .args(["has-session", "-t", &session])
            .output()
            .unwrap()
            .status
            .success()
    );
    let foreign = Sessions::new();
    std::os::unix::fs::symlink(foreign.path(), dir.path().join("stages/foreign")).unwrap();
    assert!(
        cleanup_tmux(dir.path())
            .unwrap_err()
            .to_string()
            .contains("Symbolic")
    );
    fs::remove_file(dir.path().join("stages/foreign")).unwrap();
}
