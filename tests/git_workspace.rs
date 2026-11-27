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
