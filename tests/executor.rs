mod support;
use aaqe::{
    runtime::executor,
    state::{read_json, write_json},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    process::Command,
    thread,
    time::{Duration, Instant},
};

#[test]
fn pinned_executor_survives_replacement_and_tampering_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let run = temp.path();
    let metadata = executor::capture(run).unwrap();
    write_json(
        &run.join("harness.json"),
        &json!({"state":{"executor":metadata}}),
    )
    .unwrap();
    let pinned = executor::resolve(run).unwrap();
    assert!(
        Command::new(&pinned)
            .arg("--version")
            .output()
            .unwrap()
            .status
            .success()
    );
    let next = executor::capture(run).unwrap();
    assert_ne!(metadata["path"], next["path"]);
    assert_eq!(metadata["sha256"], next["sha256"]);
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&pinned, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(&pinned, "corrupt").unwrap();
    assert!(
        executor::resolve(run)
            .unwrap_err()
            .to_string()
            .contains("checksum")
    );
}

#[test]
fn snapshot_reads_running_inode_after_installation_replaces_it() {
    let temp = tempfile::tempdir().unwrap();
    let installed = temp.path().join("aaqe");
    fs::copy(support::worker(), &installed).unwrap();
    let digest = format!("{:x}", Sha256::digest(fs::read(&installed).unwrap()));
    let run = temp.path().join("run");
    fs::create_dir(&run).unwrap();
    let mut child = Command::new(&installed)
        .args(["executor-snapshot", run.to_str().unwrap()])
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !run.join("ready").exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let replacement = temp.path().join("replacement");
    fs::write(&replacement, "new installation").unwrap();
    fs::rename(replacement, &installed).unwrap();
    fs::write(run.join("continue"), "").unwrap();
    assert!(child.wait().unwrap().success());
    let metadata = read_json(&run.join("captured.json")).unwrap();
    assert_eq!(metadata["sha256"], digest);
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(fs::read(metadata["path"].as_str().unwrap()).unwrap())
        ),
        digest
    );
}
