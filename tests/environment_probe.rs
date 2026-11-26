use aaqe::environment_probe::{inspect, schema};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn ssh_and_go_tool_inventory_use_their_supported_version_commands() {
    if std::env::var_os("AAQE_TOOL_VERSION_CHILD").is_none() {
        let temp = tempfile::tempdir().unwrap();
        for (tool, flag) in [("ssh", "-V"), ("go", "version")] {
            let executable = temp.path().join(tool);
            fs::write(&executable, format!("#!/bin/sh\n[ \"$#\" = 1 ] && [ \"$1\" = '{flag}' ] || exit 2\nprintf '%s\\n' '{tool} installed version'\n")).unwrap();
            fs::set_permissions(executable, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ssh_and_go_tool_inventory_use_their_supported_version_commands",
                "--nocapture",
            ])
            .env("AAQE_TOOL_VERSION_CHILD", "1")
            .env("PATH", temp.path())
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let workspace = tempfile::tempdir().unwrap();
    let evidence = tempfile::tempdir().unwrap();
    let value = inspect(
        workspace.path(),
        evidence.path(),
        &json!({"kind":"tools","tools":["ssh", "go"]}),
        3,
    )
    .unwrap();
    assert_eq!(value["success"], true);
    assert_eq!(value["results"][0]["stdout"], "ssh installed version\n");
    assert_eq!(value["results"][1]["stdout"], "go installed version\n");
}

#[test]
fn bounded_ssh_capture_drains_both_streams_and_redacts_clipped_password_prefixes() {
    if std::env::var_os("AAQE_CAPTURE_CHILD").is_none() {
        let temp = tempfile::tempdir().unwrap();
        let ssh = temp.path().join("ssh");
        fs::write(&ssh, "#!/usr/bin/python3\nimport os, sys\npayload = b'x' * (65536 - 8) + os.environ['AAQE_TEST_PASSWORD'].encode() + b'y' * (2 * 1024 * 1024)\nsys.stdout.buffer.write(payload)\nsys.stderr.buffer.write(payload)\n").unwrap();
        fs::set_permissions(ssh, fs::Permissions::from_mode(0o700)).unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "bounded_ssh_capture_drains_both_streams_and_redacts_clipped_password_prefixes",
                "--nocapture",
            ])
            .env("AAQE_CAPTURE_CHILD", "1")
            .env("AAQE_TEST_PASSWORD", "sensitive-password-boundary-987")
            .env("PATH", temp.path())
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let workspace = tempfile::tempdir().unwrap();
    let evidence = tempfile::tempdir().unwrap();
    let value = inspect(
        workspace.path(),
        evidence.path(),
        &json!({"kind":"ssh_inventory","host":"example.test","password_env":"AAQE_TEST_PASSWORD"}),
        3,
    )
    .unwrap();
    assert_eq!(value["success"], true);
    assert_eq!(value["capture_limit_bytes"], 65536);
    for stream in ["stdout", "stderr"] {
        let output = value["results"][0][stream].as_str().unwrap();
        assert!(output.len() <= 65536 + 32);
        assert!(output.contains("[output truncated]"));
        assert!(!output.contains("sensitiv"));
        let path = std::path::Path::new(value["directory"].as_str().unwrap())
            .join("process-0")
            .join(format!("{stream}.log"));
        assert!(!fs::read_to_string(path).unwrap().contains("sensitiv"));
    }
}

#[test]
fn endpoint_failure_is_recorded_as_unhealthy_with_real_http_status() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0u8; 2048];
        assert!(connection.read(&mut request).unwrap() > 0);
        connection.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    let workspace = tempfile::tempdir().unwrap();
    let evidence = tempfile::tempdir().unwrap();
    let value = inspect(
        workspace.path(),
        evidence.path(),
        &json!({"kind":"endpoint","url":format!("http://{address}/health")}),
        3,
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(value["success"], false);
    assert!(
        value["results"][0]["stdout"]
            .as_str()
            .unwrap()
            .contains("503")
    );
}

#[test]
fn probes_reject_arbitrary_commands_bad_remote_targets_and_invalid_timeouts() {
    let workspace = tempfile::tempdir().unwrap();
    let evidence = tempfile::tempdir().unwrap();
    for probe in [
        json!({"kind":"tools","tools":["sh"]}),
        json!({"kind":"tools","tools":["/tmp/custom"]}),
        json!({"kind":"docker","command":["rm","-f","shared"]}),
        json!({"kind":"endpoint","url":"file:///etc/passwd"}),
        json!({"kind":"endpoint","url":"https://user:secret@example.com"}),
        json!({"kind":"ssh_inventory","host":"-oProxyCommand=touch /tmp/pwn"}),
        json!({"kind":"ssh_inventory","host":"localhost","command":"rm -rf /"}),
        json!({"kind":"ssh_inventory","host":"localhost","port":0}),
    ] {
        assert!(
            inspect(workspace.path(), evidence.path(), &probe, 3).is_err(),
            "accepted {probe}"
        );
    }
    assert!(
        inspect(
            workspace.path(),
            evidence.path(),
            &json!({"kind":"tools","tools":["git"]}),
            0
        )
        .is_err()
    );
    assert_eq!(fs::read_dir(evidence.path()).unwrap().count(), 0);
    assert!(schema().is_object());
}

#[test]
fn local_tool_probe_records_real_process_evidence_without_worktree_writes() {
    let workspace = tempfile::tempdir().unwrap();
    let evidence = tempfile::tempdir().unwrap();
    let value = inspect(
        workspace.path(),
        evidence.path(),
        &json!({"kind":"tools","tools":["git"]}),
        3,
    )
    .unwrap();
    assert_eq!(value["success"], true);
    assert!(
        value["results"][0]["stdout"]
            .as_str()
            .unwrap()
            .contains("git version")
    );
    assert!(
        std::path::Path::new(value["directory"].as_str().unwrap())
            .join("probe.json")
            .is_file()
    );
    assert_eq!(fs::read_dir(workspace.path()).unwrap().count(), 0);
}

#[test]
fn ssh_password_echoes_are_redacted_from_all_saved_evidence() {
    if std::env::var_os("AAQE_PROBE_CHILD").is_none() {
        let temp = tempfile::tempdir().unwrap();
        let ssh = temp.path().join("ssh");
        fs::write(&ssh, "#!/bin/sh\n\"$SSH_ASKPASS\"\nprintf '%s' \"$AAQE_SSH_PASSWORD\" >&2\nprintf '\\n%s\\n' \"$*\"\n").unwrap();
        fs::set_permissions(&ssh, fs::Permissions::from_mode(0o700)).unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ssh_password_echoes_are_redacted_from_all_saved_evidence",
                "--nocapture",
            ])
            .env("AAQE_PROBE_CHILD", "1")
            .env("AAQE_TEST_PASSWORD", "private-test-password-987")
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    temp.path().display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let workspace = tempfile::tempdir().unwrap();
    let evidence = tempfile::tempdir().unwrap();
    let value = inspect(workspace.path(), evidence.path(), &json!({"kind":"ssh_inventory","host":"example.test","user":"tester","password_env":"AAQE_TEST_PASSWORD"}), 3).unwrap();
    let serialized = serde_json::to_string(&value).unwrap();
    assert!(!serialized.contains("private-test-password-987"));
    assert!(serialized.contains("[REDACTED]"));
    assert!(
        value["results"][0]["stdout"]
            .as_str()
            .unwrap()
            .contains("-F /dev/null")
    );
    let directory = std::path::Path::new(value["directory"].as_str().unwrap());
    fn check(path: &std::path::Path) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                check(&path)
            } else {
                assert!(
                    !fs::read_to_string(path)
                        .unwrap()
                        .contains("private-test-password-987")
                );
            }
        }
    }
    check(directory);
    assert!(
        value["results"][0]["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|part| part.as_str().unwrap().contains("docker ps"))
    );
}

#[test]
fn multiple_probe_commands_share_one_operation_deadline() {
    if std::env::var_os("AAQE_PROBE_DEADLINE_CHILD").is_none() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["git", "go", "ssh"] {
            let file = temp.path().join(name);
            fs::write(&file, "#!/bin/sh\n/bin/sleep 1.1\n").unwrap();
            fs::set_permissions(file, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "multiple_probe_commands_share_one_operation_deadline",
                "--nocapture",
            ])
            .env("AAQE_PROBE_DEADLINE_CHILD", "1")
            .env("PATH", temp.path())
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let workspace = tempfile::tempdir().unwrap();
    let evidence = tempfile::tempdir().unwrap();
    let start = std::time::Instant::now();
    let result = inspect(
        workspace.path(),
        evidence.path(),
        &json!({"kind":"tools","tools":["git","go","ssh"]}),
        2,
    );
    assert!(
        start.elapsed() < std::time::Duration::from_millis(3000),
        "probe commands multiplied the initial operation timeout"
    );
    assert!(result.is_err() || result.unwrap()["success"] == false);
}
