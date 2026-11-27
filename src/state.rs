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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use serde_json::json;
    use std::fs;

    #[test]
    fn new_run_identity_uses_shanghai_wall_time_and_keeps_legacy_ids_readable() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path());
        let state = store
            .create("audit", &Config::default(), temp.path())
            .unwrap();
        let id = state["id"].as_str().unwrap();
        assert!(
            regex::Regex::new(r"^[0-9]{14}-[0-9a-f]{8}$")
                .unwrap()
                .is_match(id),
            "{id}"
        );
        let created =
            chrono::DateTime::parse_from_rfc3339(state["created_at"].as_str().unwrap()).unwrap();
        assert_eq!(created.offset().local_minus_utc(), 28_800);
        let named = chrono::NaiveDateTime::parse_from_str(&id[..14], "%Y%m%d%H%M%S").unwrap();
        assert!((created.naive_local() - named).num_seconds().abs() <= 1);
        assert_eq!(store.load(id).unwrap()["id"], id);
        let mut legacy = state.clone();
        legacy["id"] = json!("20261125T081514Z-ab12cd34");
        store.save(&legacy).unwrap();
        assert_eq!(
            store.load("20261125T081514Z-ab12cd34").unwrap()["id"],
            legacy["id"]
        );
    }

    #[test]
    fn state_rejects_traversal_and_preserves_json_format() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path().join("runs"));
        for id in ["../other", "", "-bad", "a/b", "."] {
            assert!(store.path(id).is_err());
        }
        let state = store
            .create("inspect 产品", &Config::default(), temp.path())
            .unwrap();
        assert_eq!(store.load(state["id"].as_str().unwrap()).unwrap(), state);
        assert_eq!(state["prompt"], "inspect 产品");
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            assert_eq!(
                fs::metadata(store.path(state["id"].as_str().unwrap()).unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            symlink(temp.path(), temp.path().join("runs/linked")).unwrap();
            assert!(store.path("linked").is_err());
        }
    }

    fn snapshot(state: &serde_json::Value) -> serde_json::Value {
        json!({"schema_version":1,"state":state,"artifacts":{},"execution":{"baseline":[],"tests":[]},"verdicts":[]})
    }

    #[test]
    fn post_audit_state_updates_survive_reloading_and_stale_compatibility_files() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path());
        let mut state = store
            .create("audit", &crate::config::Config::default(), temp.path())
            .unwrap();
        let id = state["id"].as_str().unwrap().to_owned();
        let directory = store.path(&id).unwrap();
        state["status"] = json!("completed");
        write_json(&directory.join("harness.json"), &snapshot(&state)).unwrap();
        state["reproductions"] =
            json!([{"id":"repro-1","status":"failed","error":"readiness failed"}]);
        store.save(&state).unwrap();
        assert_eq!(
            read_json(&directory.join("harness.json")).unwrap()["state"]["reproductions"][0]["id"],
            "repro-1"
        );
        fs::write(directory.join("run.json"), "broken projection").unwrap();
        assert_eq!(
            store.load(&id).unwrap()["reproductions"][0]["id"],
            "repro-1"
        );
    }

    #[test]
    fn unversioned_snapshot_imports_post_audit_fields_without_replacing_audit_state() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path());
        let mut state = store
            .create("audit", &crate::config::Config::default(), temp.path())
            .unwrap();
        let id = state["id"].as_str().unwrap().to_owned();
        let directory = store.path(&id).unwrap();
        state["status"] = json!("completed");
        let mut saved = snapshot(&state);
        saved.as_object_mut().unwrap().remove("schema_version");
        write_json(&directory.join("harness.json"), &saved).unwrap();
        state["status"] = json!("running"); // stale audit field in the compatibility file
        state["reproductions"] = json!([{"id":"legacy-repro"}]);
        state["environment_cleaned_at"] = json!("yesterday");
        state["environment_retained"] = json!(false);
        write_json(&directory.join("run.json"), &state).unwrap();
        let loaded = store.load(&id).unwrap();
        assert_eq!(loaded["status"], "completed");
        assert_eq!(loaded["reproductions"][0]["id"], "legacy-repro");
        assert_eq!(loaded["environment_cleaned_at"], "yesterday");
    }

    #[test]
    fn invalid_snapshots_are_rejected_at_load_with_field_context() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path());
        let state = store
            .create("audit", &crate::config::Config::default(), temp.path())
            .unwrap();
        let id = state["id"].as_str().unwrap();
        let directory = store.path(id).unwrap();
        for (field, invalid, expected) in [
            ("schema_version", json!(99), "version"),
            ("artifacts", json!([]), "artifacts"),
            (
                "state",
                json!({"id":id,"status":"unknown","stages":[],"config":crate::config::Config::default()}),
                "status",
            ),
            (
                "state",
                json!({"id":id,"status":"completed","stages":"broken","config":crate::config::Config::default()}),
                "stages",
            ),
        ] {
            let mut saved = snapshot(&state);
            saved[field] = invalid;
            write_json(&directory.join("harness.json"), &saved).unwrap();
            let error = store.load(id).unwrap_err();
            assert!(format!("{error:#}").contains(expected), "{error:#}");
        }
    }

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
