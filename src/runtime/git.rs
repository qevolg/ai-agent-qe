use crate::{config::Config, runtime::process, state::absolute_path};
use anyhow::{Context, Result, bail, ensure};
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
    let config = resolve_target(caller, config)?;
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
    Ok(
        json!({"path":path,"repository":repository,"commit":commit,"branch":branch,"remote":config.remote,"target_branch":config.branch}),
    )
}

/// Resolve once at audit startup; retain the result for later publication.
pub fn resolve_target(caller: &Path, config: &Config) -> Result<Config> {
    let mut resolved = config.clone();
    let remotes = git(caller, &["remote"])?;
    let remotes: Vec<_> = remotes.lines().collect();
    if resolved.remote.is_empty() {
        let upstream = git(caller, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .ok()
            .and_then(|branch| {
                git(
                    caller,
                    &["config", "--get", &format!("branch.{branch}.remote")],
                )
                .ok()
            })
            .filter(|remote| remotes.contains(&remote.as_str()));
        resolved.remote = if let Some(remote) = upstream {
            remote
        } else if remotes.contains(&"origin") {
            "origin".into()
        } else if remotes.len() == 1 {
            remotes[0].into()
        } else {
            bail!("Cannot choose a Git remote; set git.remote in aaqe.yaml");
        };
    }
    ensure!(
        !resolved.remote.starts_with('-') && remotes.contains(&resolved.remote.as_str()),
        "Unknown Git remote; set git.remote in aaqe.yaml"
    );
    if resolved.branch.is_empty() {
        let head = git(caller, &["ls-remote", "--symref", &resolved.remote, "HEAD"])?;
        resolved.branch = head
            .lines()
            .find_map(|line| {
                line.strip_prefix("ref: refs/heads/")
                    .and_then(|rest| rest.strip_suffix("\tHEAD"))
            })
            .context("Remote has no default branch; set git.branch in aaqe.yaml")?
            .into();
    }
    git(
        caller,
        &[
            "check-ref-format",
            &format!("refs/heads/{}", resolved.branch),
        ],
    )?;
    Ok(resolved)
}
