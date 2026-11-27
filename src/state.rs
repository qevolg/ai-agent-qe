use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, FixedOffset, Utc};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

mod snapshot;
pub use snapshot::HarnessSnapshot;

#[cfg(test)]
mod tests {
    use super::shanghai_time;
    use chrono::{DateTime, Utc};

    #[test]
    fn shanghai_run_time_rolls_into_the_next_day_and_year() {
        for (utc, name, timestamp) in [
            (
                "2026-11-25T00:15:14Z",
                "20261125081514",
                "2026-11-25T08:15:14+08:00",
            ),
            (
                "2026-12-31T16:00:00Z",
                "20270101000000",
                "2027-01-01T00:00:00+08:00",
            ),
        ] {
            let instant: DateTime<Utc> = utc.parse().unwrap();
            let local = shanghai_time(instant);
            assert_eq!(local.format("%Y%m%d%H%M%S").to_string(), name);
            assert_eq!(local.to_rfc3339(), timestamp);
        }
    }
}

/// Persistent inode: unlinking an advisory lock can allow two independent owners.
pub struct RunLock(fs::File);

impl RunLock {
    pub fn acquire(directory: &Path) -> Result<Self> {
        Self::open(directory, libc::LOCK_EX | libc::LOCK_NB)
    }

    /// Audit finalization waits for an in-flight MCP response to finish saving.
    pub(crate) fn wait(directory: &Path) -> Result<Self> {
        Self::open(directory, libc::LOCK_EX)
    }

    fn open(directory: &Path, operation: i32) -> Result<Self> {
        use std::os::fd::AsRawFd;
        fs::create_dir_all(directory)?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("operation.lock"))?;
        ensure!(
            unsafe { libc::flock(file.as_raw_fd(), operation) } == 0,
            "Run is busy; another operation owns {}",
            directory.display()
        );
        Ok(Self(file))
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

use crate::config::Config;

pub fn now() -> String {
    shanghai_time(Utc::now()).to_rfc3339_opts(chrono::SecondsFormat::Micros, false)
}

// Asia/Shanghai uses UTC+08:00 for present-day audit runs, independent of host TZ.
fn shanghai_time(instant: DateTime<Utc>) -> DateTime<FixedOffset> {
    instant.with_timezone(&FixedOffset::east_opt(8 * 60 * 60).unwrap())
}

pub fn expand_user(path: &Path) -> PathBuf {
    if let Ok(relative) = path.strip_prefix("~") {
        return PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into()))
            .join(relative);
    }
    path.to_path_buf()
}

pub fn absolute_path(path: &Path) -> PathBuf {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            _ => {
                result.push(component.as_os_str());
                if let Ok(resolved) = fs::canonicalize(&result) {
                    result = resolved;
                }
            }
        }
    }
    result
}

pub fn write_json(path: &Path, document: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_file_name(format!(
        "{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        Uuid::new_v4().simple()
    ));
    let mut data = serde_json::to_vec_pretty(document)?;
    data.push(b'\n');
    let result = (|| -> Result<()> {
        fs::write(&temporary, data)?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("Cannot write artifact {}", path.display()))
}

pub fn read_json(path: &Path) -> Result<Value> {
    let data =
        fs::read(path).with_context(|| format!("Cannot read artifact {}", path.display()))?;
    serde_json::from_slice(&data)
        .with_context(|| format!("Cannot read artifact {}", path.display()))
}

#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: absolute_path(&expand_user(root.as_ref())),
        }
    }

    pub fn path(&self, run_id: &str) -> Result<PathBuf> {
        if !Regex::new(r"^[A-Za-z0-9][A-Za-z0-9_-]{0,99}$")?.is_match(run_id) {
            bail!("Invalid run ID");
        }
        let path = self.root.join(run_id);
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            bail!("Run directories cannot be symbolic links");
        }
        Ok(path)
    }

    pub fn create(&self, prompt: &str, config: &Config, caller: &Path) -> Result<Value> {
        let created_at = shanghai_time(Utc::now());
        let run_id = format!(
            "{}-{}",
            created_at.format("%Y%m%d%H%M%S"),
            &Uuid::new_v4().simple().to_string()[..8]
        );
        let directory = self.path(&run_id)?;
        fs::create_dir_all(&self.root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().mode(0o700).create(&directory)?;
        }
        #[cfg(not(unix))]
        fs::create_dir(&directory)?;
        let state = json!({
            "id":run_id,"created_at":created_at.to_rfc3339_opts(chrono::SecondsFormat::Micros, false),"prompt":prompt,"caller":absolute_path(caller),
            "config":config,"status":"initializing","stage":0,"stages":[],"error":null,
            "workspace":null,"environment":null
        });
        self.save(&state)?;
        Ok(state)
    }

    pub fn load(&self, run_id: &str) -> Result<Value> {
        let directory = self.path(run_id)?;
        if directory.join("harness.json").exists() {
            return Ok(HarnessSnapshot::load(&directory)?.state);
        }
        let state = read_json(&directory.join("run.json"))?;
        ensure!(
            state["id"].as_str() == Some(run_id),
            "Saved run identity differs from run directory"
        );
        Ok(state)
    }

    pub fn save(&self, state: &Value) -> Result<()> {
        let id = state["id"].as_str().context("Invalid run ID")?;
        let directory = self.path(id)?;
        if directory.join("harness.json").exists() {
            let mut snapshot = HarnessSnapshot::load(&directory)?;
            snapshot.state = state.clone();
            snapshot.save(&directory)
        } else {
            write_json(&directory.join("run.json"), state)
        }
    }
}
