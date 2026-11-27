use crate::{config::Config, runtime::process, state::absolute_path};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn git(directory: &Path, args: &[&str]) -> Result<String> {
    let mut command = vec![
        "git".into(),
        "-C".into(),
        directory.to_string_lossy().into_owned(),
    ];
    command.extend(args.iter().map(|s| (*s).to_owned()));
    let result = process::run(&command, None, 180, None);
    if result.returncode != 0 {
        bail!(
            "Git {} failed: {}",
            args.first().copied().unwrap_or(""),
            result.stderr.trim()
        );
    }
    Ok(result.stdout.trim().to_owned())
}

pub fn create_workspace(
    caller: &Path,
    run_dir: &Path,
    config: &Config,
    run_id: &str,
) -> Result<Value> {
    let repository = absolute_path(&PathBuf::from(git(
        caller,
        &["rev-parse", "--show-toplevel"],
    )?));
    let remote_ref = format!("refs/heads/{}", config.branch);
    git(&repository, &["check-ref-format", &remote_ref])?;
    git(
        &repository,
        &["fetch", "--no-tags", &config.remote, &remote_ref],
    )?;
    let commit = git(&repository, &["rev-parse", "FETCH_HEAD^{commit}"])?;
    let path = absolute_path(run_dir).join("worktree");
    let branch = format!("aaqe/{run_id}");
    git(
        &repository,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            &path.to_string_lossy(),
            &commit,
        ],
    )?;
    Ok(json!({"path":path,"repository":repository,"commit":commit,"branch":branch}))
}
