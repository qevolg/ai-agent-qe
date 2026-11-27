use sha2::Digest;
mod support;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn installer_checks_checksum_and_preserves_previous_binary_on_failure() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let release = temp.path().join("releases/download/v0.1.0");
    let destination = temp.path().join("bin with spaces");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&release).unwrap();
    fs::write(source.join("aaqe"), "#!/bin/sh\necho 'aaqe 0.1.0'\n").unwrap();
    fs::set_permissions(source.join("aaqe"), fs::Permissions::from_mode(0o755)).unwrap();
    let archive = release.join("aaqe-x86_64-unknown-linux-musl.tar.gz");
    assert!(
        Command::new("tar")
            .args(["-czf"])
            .arg(&archive)
            .arg("-C")
            .arg(&source)
            .arg("aaqe")
            .status()
            .unwrap()
            .success()
    );
    let checksum = Command::new("sha256sum").arg(&archive).output().unwrap();
    fs::write(archive.with_extension("gz.sha256"), checksum.stdout).unwrap();
    let install = || {
        Command::new("bash")
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh"))
            .args(["--version", "0.1.0", "--bin-dir"])
            .arg(&destination)
            .env(
                "AAQE_RELEASE_BASE_URL",
                format!("file://{}", temp.path().join("releases").display()),
            )
            .output()
            .unwrap()
    };
    // An already running executable must keep its original inode during install.
    fs::create_dir_all(&destination).unwrap();
    fs::copy(support::worker(), destination.join("aaqe")).unwrap();
    let old_digest = format!(
        "{:x}",
        sha2::Sha256::digest(fs::read(destination.join("aaqe")).unwrap())
    );
    let run = temp.path().join("active");
    fs::create_dir(&run).unwrap();
    let mut active = Command::new(destination.join("aaqe"))
        .arg("executor-snapshot")
        .arg(&run)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !run.join("ready").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = install();
    fs::write(run.join("continue"), "").unwrap();
    assert!(active.wait().unwrap().success());
    assert_eq!(
        aaqe::state::read_json(&run.join("captured.json")).unwrap()["sha256"],
        old_digest
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let previous = fs::read(destination.join("aaqe")).unwrap();
    fs::write(
        archive.with_extension("gz.sha256"),
        format!("{}\n", "0".repeat(64)),
    )
    .unwrap();
    assert!(!install().status.success());
    assert_eq!(fs::read(destination.join("aaqe")).unwrap(), previous);
    fs::remove_file(archive.with_extension("gz.sha256")).unwrap();
    assert!(!install().status.success());
    assert_eq!(fs::read(destination.join("aaqe")).unwrap(), previous);
}
