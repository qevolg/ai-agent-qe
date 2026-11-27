use crate::{
    config::Config,
    guards::patch::PatchGate,
    runtime::{
        git::git,
        process::{run, run_env},
    },
    state::{read_json, write_json},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn project(remote: &str) -> Result<(String, String)> {
    let remote = remote.trim_end_matches('/').trim_end_matches(".git");
    let (host, path) = if let Some(url) = remote
        .strip_prefix("https://")
        .or_else(|| remote.strip_prefix("http://"))
    {
        url.split_once('/')
            .context("GitLab project URL has no project path")?
    } else if let Some(url) = remote.strip_prefix("ssh://") {
        let url = url.split_once('@').map_or(url, |(_, rest)| rest);
        url.split_once('/')
            .context("GitLab SSH URL has no project path")?
    } else {
        remote
            .split_once('@')
            .context("Use a GitLab remote or publication.project URL")?
            .1
            .split_once(':')
            .context("Invalid GitLab SSH remote")?
    };
    ensure!(
        !host.is_empty()
            && !host.contains('@')
            && host != "github.com"
            && path.contains('/')
            && !path.contains(['?', '#', '\\']),
        "A GitLab project URL is required"
    );
    Ok((host.into(), path.into()))
}

fn api(
    root: &Path,
    config: &Config,
    host: &str,
    endpoint: &str,
    body: Option<&Path>,
) -> Result<Value> {
    api_method(root, config, host, endpoint, body, "POST")
}

fn api_method(
    root: &Path,
    config: &Config,
    host: &str,
    endpoint: &str,
    body: Option<&Path>,
    method: &str,
) -> Result<Value> {
    let mut command = config.publication.glab_command.clone();
    ensure!(
        !command.is_empty(),
        "publication.glab_command cannot be empty"
    );
    command.extend([
        "api".into(),
        "--hostname".into(),
        host.into(),
        endpoint.into(),
    ]);
    if let Some(body) = body {
        command.extend([
            "--method".into(),
            method.into(),
            "--input".into(),
            body.display().to_string(),
        ]);
    }
    let result = run(&command, Some(root), 180, None);
    ensure!(
        result.returncode == 0,
        "GitLab API request failed; verify glab authentication and project permissions: {}",
        result.stderr.trim()
    );
    serde_json::from_str(&result.stdout).context("Invalid GitLab API response")
}

pub fn publish(
    directory: &Path,
    state: &mut Value,
    artifacts: &Value,
    execution: &Value,
    config: &Config,
) -> Result<()> {
    if directory.join("publication.json").exists() {
        state["publication"] = read_json(&directory.join("publication.json"))?;
    }
    if !state["publication"].is_object() {
        state["publication"] = json!({});
    }
    let result = publish_inner(directory, state, artifacts, execution, config);
    if let Err(error) = &result {
        state["publication"]["status"] = json!("failed");
        state["publication"]["error"] = json!(format!("{error:#}"));
    }
    write_json(&directory.join("publication.json"), &state["publication"])?;
    result
}

fn publish_inner(
    directory: &Path,
    state: &mut Value,
    artifacts: &Value,
    execution: &Value,
    config: &Config,
) -> Result<()> {
    ensure!(
        state["audit_finalized"] == true && state["status"] != "interrupted",
        "Finalize the audit before publishing tests"
    );
    ensure!(
        super::policy::denial(directory).is_none() && state["recovery_error"].is_null(),
        "Cannot publish after policy denial or failed patch recovery"
    );
    let patch = artifacts["test-developer"]["patch"]
        .as_str()
        .filter(|patch| !patch.trim().is_empty())
        .context("No accepted tests to publish")?;
    let workspace = state["workspace"].clone();
    let root = Path::new(
        workspace["path"]
            .as_str()
            .context("Missing audit worktree")?,
    );
    let base = workspace["commit"].as_str().context("Missing audit base")?;
    let gate = PatchGate::new(root, Some(base))?.with_patch(patch);
    let mut paths = gate.verify_worktree()?;
    let remote = git(root, &["remote", "get-url", &config.remote])?;
    let (host, project_path) = project(config.publication.project.as_deref().unwrap_or(&remote))?;
    let branch = format!(
        "aaqe-tests/{}",
        state["id"].as_str().context("Missing run identity")?
    );
    git(root, &["check-ref-format", &format!("refs/heads/{branch}")])?;
    let index_path = directory.join("publication-index");
    if index_path.exists() {
        fs::remove_file(&index_path)?;
    }
    let environment = [("GIT_INDEX_FILE".into(), index_path.display().to_string())];
    let indexed = |arguments: &[&str], input: Option<&str>| -> Result<String> {
        let command = std::iter::once("git")
            .chain(arguments.iter().copied())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let result = run_env(&command, Some(root), 180, input, &environment);
        ensure!(
            result.returncode == 0,
            "Cannot construct protected test commit: {}",
            result.stderr.trim()
        );
        Ok(result.stdout.trim().into())
    };
    let tree_result = (|| -> Result<String> {
        indexed(&["read-tree", base], None)?;
        indexed(
            &["apply", "--cached", "--whitespace=error", "-"],
            Some(patch),
        )?;
        let expected_tree = indexed(&["write-tree"], None)?;
        let accepted_paths = indexed(&["diff", "--cached", "--name-only", "-z", base], None)?;
        for path in accepted_paths.split('\0').filter(|path| !path.is_empty()) {
            if !paths.iter().any(|existing| existing == path) {
                paths.push(path.to_owned());
            }
        }
        indexed(&["read-tree", base], None)?;
        let mut arguments = vec!["--literal-pathspecs", "add", "--force", "--"];
        arguments.extend(paths.iter().map(String::as_str));
        ensure!(!paths.is_empty(), "Accepted test worktree has no changes");
        indexed(&arguments, None)?;
        ensure!(
            indexed(&["write-tree"], None)? == expected_tree,
            "Audit worktree differs from accepted test patch"
        );
        Ok(expected_tree)
    })();
    let _ = fs::remove_file(index_path);
    let tree = tree_result?;
    let saved_ref = git(
        root,
        &["rev-parse", "--verify", &format!("refs/heads/{branch}")],
    )
    .ok();
    let previous = state["publication"].clone();
    let mut history = previous["history"].as_array().cloned().unwrap_or_default();
    let saved_commit = state["publication"]["commit"]
        .as_str()
        .or(saved_ref.as_deref());
    let parent = if let Some(commit) = saved_commit {
        if let Some(reference) = &saved_ref {
            ensure!(
                reference == commit,
                "Publication branch differs from saved commit"
            );
        }
        git(root, &["merge-base", "--is-ancestor", base, commit])?;
        let previous_tree = git(root, &["show", "-s", "--format=%T", commit])?;
        if previous_tree != tree {
            ensure!(
                state["resume_round"].as_u64().unwrap_or(0) > 0,
                "Saved publication commit differs from accepted patch"
            );
            let mut entry = previous.clone();
            entry.as_object_mut().unwrap().remove("history");
            if entry["commit"].is_null() {
                entry["commit"] = json!(commit);
            }
            history.push(entry);
            commit
        } else {
            base
        }
    } else {
        base
    };
    let commit = if let Some(commit) = saved_commit.filter(|_| parent == base) {
        ensure!(
            git(root, &["show", "-s", "--format=%T", commit])? == tree,
            "Saved publication commit differs from accepted patch"
        );
        commit.to_owned()
    } else {
        git(
            root,
            &[
                "commit-tree",
                &tree,
                "-p",
                parent,
                "-m",
                &format!(
                    "test: add AAQE regressions for {}",
                    state["id"].as_str().unwrap()
                ),
            ],
        )?
    };
    let draft =
        !crate::guards::coverage::approved(execution, artifacts) || state["status"] != "completed";
    state["publication"] = json!({"status":"committed","commit":commit,"tree":tree,"history":history,"branch":branch,"target_branch":config.branch,"draft":draft,"project":format!("https://{host}/{project_path}")});
    write_json(&directory.join("publication.json"), &state["publication"])?;
    git(
        root,
        &["update-ref", &format!("refs/heads/{branch}"), &commit],
    )?;
    git(
        root,
        &[
            "push",
            &config.remote,
            &format!("{commit}:refs/heads/{branch}"),
        ],
    )?;
    let remote_sha = git(
        root,
        &["ls-remote", &config.remote, &format!("refs/heads/{branch}")],
    )?;
    ensure!(
        remote_sha.split_whitespace().next() == Some(commit.as_str()),
        "Remote test branch SHA differs from committed tests"
    );
    state["publication"]["status"] = json!("pushed");
    write_json(&directory.join("publication.json"), &state["publication"])?;
    let project_endpoint = format!("projects/{}", encode(&project_path));
    let metadata = api(root, config, &host, &project_endpoint, None)?;
    let project_id = metadata["id"]
        .as_u64()
        .context("GitLab did not return a project identity")?;
    let endpoint = format!("{project_endpoint}/merge_requests");
    let existing = api(
        root,
        config,
        &host,
        &format!(
            "{endpoint}?state=opened&source_branch={}&target_branch={}",
            encode(&branch),
            encode(&config.branch)
        ),
        None,
    )?;
    let existing = existing.as_array().context("Invalid GitLab MR list")?;
    ensure!(
        existing.len() <= 1,
        "Multiple open merge requests for audit branch"
    );
    let title = format!(
        "{}test: AAQE regressions {}",
        if draft { "Draft: " } else { "" },
        state["id"].as_str().unwrap()
    );
    let description = format!(
        "AAQE run: {}\n\nAudited base: {base}\n\nTest commit: {commit}\n\nModule line coverage (minimum 80%): {}\n\nAudit status: {}\n\n{}\n\nOnly accepted test changes are included. Product assertion failures remain evidence. Detailed logs and reports are retained by the audit operator.",
        state["id"],
        execution["coverage"],
        state["status"],
        state["finish_reason"].as_str().unwrap_or("")
    );
    let body = directory.join("publication-request.json");
    let mr = if let Some(mr) = existing.first() {
        let iid = mr["iid"].as_u64().context("GitLab MR has no IID")?;
        ensure!(
            mr["source_project_id"] == project_id
                && mr["target_project_id"] == project_id
                && mr["source_branch"] == branch
                && mr["target_branch"] == config.branch
                && mr["sha"] == commit,
            "Existing MR differs from the published branch"
        );
        write_json(&body, &json!({"title":title,"description":description}))?;
        api_method(
            root,
            config,
            &host,
            &format!("{endpoint}/{iid}"),
            Some(&body),
            "PUT",
        )?
    } else {
        write_json(
            &body,
            &json!({"source_branch":branch,"target_branch":config.branch,"title":title,"description":description,"remove_source_branch":false}),
        )?;
        api(root, config, &host, &endpoint, Some(&body))?
    };
    let url = mr["web_url"]
        .as_str()
        .context("GitLab did not return an MR URL")?;
    ensure!(
        mr["source_project_id"] == project_id
            && mr["target_project_id"] == project_id
            && mr["source_branch"] == branch
            && mr["target_branch"] == config.branch
            && mr["sha"] == commit,
        "GitLab MR does not contain the published test commit in the expected project and branches"
    );
    ensure!(
        url.starts_with(&format!("https://{host}/{project_path}/-/merge_requests/")),
        "GitLab returned an unexpected MR URL"
    );
    ensure!(
        !draft
            || mr["draft"] == true
            || mr["work_in_progress"] == true
            || mr["title"]
                .as_str()
                .is_some_and(|title| title.starts_with("Draft:")),
        "Coverage incomplete but existing MR is not Draft"
    );
    state["publication"]["status"] = json!("published");
    state["publication"]["mr_url"] = json!(url);
    state["publication"]["iid"] = mr["iid"].clone();
    Ok(())
}
