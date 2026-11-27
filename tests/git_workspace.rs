use aaqe::{config::Config, runtime::git as gitops};
use std::{fs, path::Path};

fn git(directory: &Path, arguments: &[&str]) -> String {
    gitops::git(directory, arguments).unwrap()
}

#[test]
fn workspace_uses_fetched_main_and_preserves_dirty_caller() {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    fs::create_dir(&seed).unwrap();
    git(temp.path(), &["init", "--bare", remote.to_str().unwrap()]);
    git(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git(&seed, &["init", "-b", "main"]);
    git(&seed, &["config", "user.name", "Test User"]);
    git(&seed, &["config", "user.email", "test@example.invalid"]);
    fs::write(seed.join("app.rs"), "original\n").unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-m", "initial"]);
    git(
        &seed,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&seed, &["push", "-u", "origin", "main"]);
    git(&seed, &["checkout", "-b", "local-work"]);
    fs::write(seed.join("app.rs"), "local uncommitted work\n").unwrap();
    let run = temp.path().join("run");
    let workspace = gitops::create_workspace(&seed, &run, &Config::default(), "run-001").unwrap();
    let path = Path::new(workspace["path"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(path.join("app.rs")).unwrap(),
        "original\n"
    );
    assert_eq!(
        fs::read_to_string(seed.join("app.rs")).unwrap(),
        "local uncommitted work\n"
    );
    assert_eq!(git(&seed, &["branch", "--show-current"]), "local-work");
    assert_eq!(
        workspace["commit"],
        git(&seed, &["rev-parse", "origin/main"])
    );
    assert_eq!(git(path, &["branch", "--show-current"]), "aaqe/run-001");
    let missing = Config {
        branch: "missing".into(),
        ..Config::default()
    };
    assert!(gitops::create_workspace(&seed, &run.join("missing"), &missing, "run-002").is_err());
}

#[test]
fn workspace_discovers_upstream_remote_default_branch_and_keeps_explicit_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let root = temp.path().join("repo");
    fs::create_dir(&root).unwrap();
    git(temp.path(), &["init", "--bare", remote.to_str().unwrap()]);
    git(&remote, &["symbolic-ref", "HEAD", "refs/heads/develop"]);
    git(&root, &["init", "-b", "develop"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    fs::write(root.join("app.rs"), "remote default\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "base"]);
    git(
        &root,
        &["remote", "add", "upstream", remote.to_str().unwrap()],
    );
    git(&root, &["push", "-u", "upstream", "develop"]);
    git(&root, &["checkout", "-b", "feature"]);
    git(&root, &["config", "branch.feature.remote", "upstream"]);
    git(&root, &["remote", "add", "origin", "missing-remote"]);
    let config = Config::load(&root, None).unwrap();
    let workspace =
        gitops::create_workspace(&root, &temp.path().join("run"), &config, "auto").unwrap();
    assert_eq!(workspace["remote"], "upstream");
    assert_eq!(workspace["target_branch"], "develop");
    assert_eq!(
        git(
            Path::new(workspace["path"].as_str().unwrap()),
            &["show", "HEAD:app.rs"]
        ),
        "remote default"
    );

    fs::write(root.join("app.rs"), "explicit branch\n").unwrap();
    git(&root, &["commit", "-am", "feature"]);
    git(&root, &["push", "upstream", "feature"]);
    let config = Config {
        remote: "upstream".into(),
        branch: "feature".into(),
        ..config
    };
    let workspace =
        gitops::create_workspace(&root, &temp.path().join("override"), &config, "override")
            .unwrap();
    assert_eq!(workspace["target_branch"], "feature");
    assert_eq!(
        git(
            Path::new(workspace["path"].as_str().unwrap()),
            &["show", "HEAD:app.rs"]
        ),
        "explicit branch"
    );
}

#[test]
fn ambiguous_remotes_and_missing_default_head_require_configuration() {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-b", "main"]);
    for name in ["one", "two"] {
        git(temp.path(), &["remote", "add", name, "."]);
    }
    let config = Config::load(temp.path(), None).unwrap();
    let error =
        gitops::create_workspace(temp.path(), &temp.path().join("run"), &config, "ambiguous")
            .unwrap_err();
    assert!(format!("{error:#}").contains("git.remote"), "{error:#}");
    git(temp.path(), &["remote", "remove", "two"]);
    let error = gitops::create_workspace(temp.path(), &temp.path().join("run"), &config, "no-head")
        .unwrap_err();
    assert!(format!("{error:#}").contains("git.branch"), "{error:#}");
}
