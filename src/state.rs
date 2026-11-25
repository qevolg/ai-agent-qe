use anyhow::{Context, Result, bail};
use chrono::Utc;
use regex::Regex;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

use crate::config::Config;

pub fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, false)
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
        let run_id = format!(
            "{}-{}",
            Utc::now().format("%Y%m%dT%H%M%SZ"),
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
            "id":run_id,"created_at":now(),"prompt":prompt,"caller":absolute_path(caller),
            "config":config,"status":"initializing","stage":0,"stages":[],"error":null,
            "workspace":null,"environment":null
        });
        self.save(&state)?;
        Ok(state)
    }

    pub fn load(&self, run_id: &str) -> Result<Value> {
        read_json(&self.path(run_id)?.join("run.json"))
    }

    pub fn save(&self, state: &Value) -> Result<()> {
        let id = state["id"].as_str().context("Invalid run ID")?;
        write_json(&self.path(id)?.join("run.json"), state)
    }
}
