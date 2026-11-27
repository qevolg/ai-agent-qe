mod support;
use aaqe::runtime::process;
use serde_json::json;
use std::fs;

#[test]
fn stage_logs_keep_full_output_on_disk_with_bounded_head_and_tail_in_process_records() {
    let temp = tempfile::tempdir().unwrap();
    let command = support::command(json!({"large":true}));
    let result = process::run_logged(&command, temp.path(), 10, "", temp.path()).unwrap();
    assert_eq!(result.returncode, 0);
    assert!(result.stdout.len() < 70000 && result.stderr.len() < 70000);
    assert!(result.stdout.starts_with("BEGIN\n") && result.stdout.ends_with("\nEND"));
    assert!(result.stderr.starts_with("ERROR-BEGIN\n") && result.stderr.ends_with("\nERROR-END"));
    assert!(result.stdout.contains(process::OUTPUT_TRUNCATED.trim()));
    assert_eq!(
        fs::metadata(temp.path().join("stdout.log")).unwrap().len(),
        2_000_010
    );
    assert_eq!(
        fs::metadata(temp.path().join("stderr.log")).unwrap().len(),
        2_000_022
    );
    let saved = aaqe::state::read_json(&temp.path().join("process.json")).unwrap();
    assert_eq!(saved["stdout"], result.stdout);
}

#[test]
fn streamed_binary_output_stays_bounded_after_lossy_utf8_conversion() {
    let temp = tempfile::tempdir().unwrap();
    let command = support::command(json!({"binary":true}));
    let result = process::run_logged(&command, temp.path(), 10, "", temp.path()).unwrap();
    assert_eq!(result.returncode, 0);
    assert!(result.stdout.len() < 70000);
    assert_eq!(
        fs::read(temp.path().join("stdout.log")).unwrap(),
        vec![255; 60000]
    );
}

#[test]
fn failure_to_persist_a_stream_is_reported_as_failed_execution() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    symlink("/dev/full", temp.path().join("stdout.log")).unwrap();
    let result = process::run_logged(
        &support::command(json!({"stdout":"evidence"})),
        temp.path(),
        5,
        "",
        temp.path(),
    )
    .unwrap();
    assert_ne!(
        result.returncode, 0,
        "A process without persisted evidence must not appear successful"
    );
    assert!(result.stderr.contains("log"), "{}", result.stderr);
}

#[test]
fn process_captures_stdin_and_missing_executables() {
    let result = process::run(
        &support::command(json!({"stdin":true,"stderr":"diagnostic","exit":7})),
        None,
        10,
        Some("产品"),
    );
    assert_eq!(result.returncode, 7);
    assert_eq!(result.stdout, "产品");
    assert_eq!(result.stderr, "diagnostic");
    let result = process::run(&["aaqe-this-command-does-not-exist".into()], None, 10, None);
    assert_eq!(result.returncode, 127);
    assert!(!result.stderr.is_empty());
}

#[test]
fn process_timeout_kills_descendants() {
    let result = process::run(
        &support::command(json!({"child":true,"delay_ms":30000})),
        None,
        1,
        None,
    );
    assert_eq!(result.returncode, 124);
    assert!(result.timed_out);
    assert!(result.elapsed < 5.0);
}

#[cfg(unix)]
#[test]
fn interrupted_processes_can_finish_cleanup_without_clearing_status() {
    if std::env::var_os("AAQE_CORE_INTERRUPT_CHILD").is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "interrupted_processes_can_finish_cleanup_without_clearing_status",
                "--nocapture",
            ])
            .env("AAQE_CORE_INTERRUPT_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    process::install_signals().unwrap();
    unsafe {
        libc::raise(libc::SIGINT);
    }
    assert!(process::interrupted());
    let cancelled = process::run(&support::command(json!({"delay_ms":30000})), None, 10, None);
    assert_eq!(cancelled.returncode, 130);
    let cleanup =
        process::ignoring_interrupts(|| process::run(&support::success(), None, 10, None));
    assert_eq!(cleanup.returncode, 0);
    assert!(process::interrupted());
    process::reset_interrupt();
}
