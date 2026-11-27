//! Immutable per-run executable copies. Never resolve a deleted executable by name.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const PROTOCOL: u64 = 1;

fn checksum(path: &Path) -> Result<String> {
    let mut source = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn capture(directory: &Path) -> Result<Value> {
    let runtime_path = directory.join("runtime");
    ensure!(
        !fs::symlink_metadata(&runtime_path).is_ok_and(|m| m.file_type().is_symlink()),
        "Run runtime directory cannot be a symbolic link"
    );
    fs::create_dir_all(&runtime_path)?;
    let runtime = directory.join("runtime").canonicalize()?;
    let destination = if runtime.join("aaqe").exists() {
        let version = runtime.join(uuid::Uuid::new_v4().simple().to_string());
        fs::create_dir(&version)?;
        version.join("aaqe")
    } else {
        runtime.join("aaqe")
    };
    let temporary = runtime.join(format!(".capture-{}", uuid::Uuid::new_v4().simple()));
    let result = (|| -> Result<Value> {
        let selected = super::tmux::executable()?;
        let actual = std::env::current_exe()?;
        // Open the live inode, including when readlink reports " (deleted)".
        let source = if cfg!(target_os = "linux") && selected == actual {
            Path::new("/proc/self/exe")
        } else {
            selected.as_path()
        };
        let mut source = File::open(source).context("Cannot read running AAQE executor")?;
        let mut target = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        std::io::copy(&mut source, &mut target)?;
        target.flush()?;
        target.sync_all()?;
        use std::os::unix::fs::PermissionsExt;
        target.set_permissions(fs::Permissions::from_mode(0o555))?;
        let digest = checksum(&temporary)?;
        // Destination is new: preserve all prior executor copies and live inodes.
        fs::hard_link(&temporary, &destination)?;
        fs::remove_file(&temporary)?;
        File::open(destination.parent().unwrap())?.sync_all()?;
        Ok(
            json!({"path":destination,"version":env!("CARGO_PKG_VERSION"),"sha256":digest,"protocol":PROTOCOL,"captured_at":crate::state::now()}),
        )
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub fn resolve(directory: &Path) -> Result<PathBuf> {
    let saved = crate::state::read_json(&directory.join("harness.json"))?;
    validate(directory, &saved["state"]["executor"])
}

pub fn validate(directory: &Path, metadata: &Value) -> Result<PathBuf> {
    ensure!(
        metadata.is_object(),
        "Run has no pinned executor; resume with --takeover"
    );
    ensure!(
        metadata["protocol"] == PROTOCOL,
        "Unsupported executor protocol; use a compatible AAQE release"
    );
    let path = PathBuf::from(
        metadata["path"]
            .as_str()
            .context("Run has no pinned executor; resume with --takeover")?,
    );
    let root = directory.join("runtime").canonicalize()?;
    ensure!(
        path.is_absolute() && path.starts_with(&root),
        "Executor is outside run runtime directory"
    );
    let relative = path
        .strip_prefix(&root)?
        .to_str()
        .context("Invalid executor path")?;
    let safe = crate::guards::patch::safe_path(&root, relative)?;
    ensure!(
        safe.is_file(),
        "Pinned executor is missing; resume with --takeover"
    );
    ensure!(
        checksum(&path)? == metadata["sha256"].as_str().unwrap_or(""),
        "Pinned executor checksum mismatch; resume with --takeover"
    );
    Ok(path)
}

/// Stage fixtures outside an audit use the Cargo binary; actual Runs always pin.
pub(crate) fn for_directory(directory: &Path) -> Result<PathBuf> {
    for parent in directory.ancestors() {
        if parent.join("harness.json").is_file() {
            let saved = crate::state::read_json(&parent.join("harness.json"))?;
            if !saved["state"]["executor"].is_null() || saved["state"]["id"].is_string() {
                return resolve(parent);
            }
        }
    }
    super::tmux::executable()
}
