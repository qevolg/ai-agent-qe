mod support;

use aaqe::{
    config::Config,
    runtime::{
        git::{create_workspace, git},
        publication::publish,
    },
};
use serde_json::json;
use std::{fs, path::Path};

#[test]
fn publication_creates_a_ready_mr_and_recovers_a_lost_journal() {
    let temp = tempfile::tempdir().unwrap();
    let repository = support::repository(temp.path(), false);
    let run = temp.path().join("run");
    fs::create_dir_all(&run).unwrap();
    let mut config = Config::default();
    let workspace = create_workspace(&repository, &run, &config, "create-test").unwrap();
    let root = Path::new(workspace["path"].as_str().unwrap());
    let patch = "--- /dev/null\n+++ b/tests/regression.rs\n@@ -0,0 +1 @@\n+#[test] fn regression() { assert!(false); }\n--- /dev/null\n+++ b/tests/fixture.json\n@@ -0,0 +1 @@\n+{\"value\":1}\n";
    git(
        root,
        &[
            "config",
            "core.excludesfile",
            temp.path().join("ignored").to_str().unwrap(),
        ],
    )
    .unwrap();
    fs::write(temp.path().join("ignored"), "tests/*.json\n").unwrap();
    aaqe::guards::patch::PatchGate::new(root, &config.test_paths, workspace["commit"].as_str())
        .unwrap()
        .apply(patch)
        .unwrap();
    let glab = temp.path().join("glab-double");
    let capture = temp.path().join("request.json");
    support::executable(
        &glab,
        json!({"mode":"gitlab","capture":capture,"mr":{"web_url":"https://gitlab.example/group/product/-/merge_requests/13","iid":13,"source_branch":"aaqe-tests/create-test","target_branch":"main","draft":false}}),
    );
    config.publication.project = Some("https://gitlab.example/group/product".into());
    config.publication.glab_command = vec![glab.display().to_string()];
    let mut state = json!({"id":"create-test","workspace":workspace,"status":"completed","audit_finalized":true});
    let artifacts = json!({"test-validator":{"coverage_assessment":{"scope_valid":true,"measurement_valid":true,"meaningful_assertions":true,"rationale":"Reviewed"}},"test-developer":{"patch":patch,"tests":[{"id":"regression"}]}});
    let execution = json!({"coverage":{"status":"passed","percent":80}});
    publish(&run, &mut state, &artifacts, &execution, &config).unwrap();
    let commit = state["publication"]["commit"].clone();
    assert_eq!(
        git(
            root,
            &[
                "show",
                &format!("{}:tests/fixture.json", commit.as_str().unwrap())
            ]
        )
        .unwrap(),
        "{\"value\":1}"
    );
    let request = aaqe::state::read_json(&capture).unwrap();
    assert_eq!(request["source_branch"], "aaqe-tests/create-test");
    assert_eq!(request["target_branch"], "main");
    assert!(!request["title"].as_str().unwrap().starts_with("Draft:"));
    fs::remove_file(run.join("publication.json")).unwrap();
    state["publication"] = serde_json::Value::Null;
    std::thread::sleep(std::time::Duration::from_millis(1100));
    publish(&run, &mut state, &artifacts, &execution, &config).unwrap();
    assert_eq!(state["publication"]["commit"], commit);
    assert_eq!(state["publication"]["status"], "published");
    assert_eq!(
        git(root, &["rev-parse", "HEAD"]).unwrap(),
        workspace["commit"]
    );
}

#[test]
fn publication_pushes_exact_patch_and_reuses_the_remote_mr() {
    let temp = tempfile::tempdir().unwrap();
    let repository = support::repository(temp.path(), false);
    let run = temp.path().join("run");
    fs::create_dir_all(&run).unwrap();
    let mut config = Config::default();
    let workspace = create_workspace(&repository, &run, &config, "publication-test").unwrap();
    let root = Path::new(workspace["path"].as_str().unwrap());
    let patch = "--- /dev/null\n+++ b/tests/regression.rs\n@@ -0,0 +1 @@\n+#[test] fn regression() { assert!(false); }\n";
    aaqe::guards::patch::PatchGate::new(root, &config.test_paths, workspace["commit"].as_str())
        .unwrap()
        .apply(patch)
        .unwrap();
    config.publication.project = Some("https://gitlab.example/group/product".into());
    config.publication.glab_command = support::command(json!({"stdout":"[]"}));
    let mut state = json!({"id":"publication-test","workspace":workspace,"status":"blocked","audit_finalized":true});
    let artifacts = json!({"test-validator":{"coverage_assessment":{"scope_valid":true,"measurement_valid":true,"meaningful_assertions":true,"rationale":"Reviewed"}},"test-developer":{"patch":patch,"tests":[{"id":"regression"}]}});
    let execution = json!({"coverage":{"status":"below_threshold","percent":79}});
    assert!(publish(&run, &mut state, &artifacts, &execution, &config).is_err());
    let commit = state["publication"]["commit"].as_str().unwrap().to_owned();
    assert_eq!(
        git(root, &["rev-parse", "HEAD"]).unwrap(),
        workspace["commit"]
    );
    assert_eq!(
        git(
            &repository,
            &[
                "ls-remote",
                "origin",
                "refs/heads/aaqe-tests/publication-test"
            ]
        )
        .unwrap()
        .split_whitespace()
        .next(),
        Some(commit.as_str())
    );
    assert_eq!(
        git(root, &["show", &format!("{commit}:app.rs")]).unwrap(),
        "pub fn value() -> i32 { 1 }"
    );
    assert!(
        git(root, &["show", &format!("{commit}:tests/regression.rs")])
            .unwrap()
            .contains("assert!(false)")
    );
    assert_eq!(state["publication"]["draft"], true);
    let glab = temp.path().join("glab-double");
    support::executable(
        &glab,
        json!({"mode":"gitlab","capture":temp.path().join("request.json"),"existing":true,"mr":{"web_url":"https://gitlab.example/group/product/-/merge_requests/12","iid":12,"source_branch":"aaqe-tests/publication-test","target_branch":"main","draft":true}}),
    );
    config.publication.glab_command = vec![glab.display().to_string()];
    publish(&run, &mut state, &artifacts, &execution, &config).unwrap();
    assert_eq!(state["publication"]["status"], "published");
    assert_eq!(state["publication"]["commit"], commit);
    assert_eq!(
        state["publication"]["mr_url"],
        "https://gitlab.example/group/product/-/merge_requests/12"
    );
    fs::write(root.join("tests/unrelated.rs"), "unrelated").unwrap();
    assert!(publish(&run, &mut state, &artifacts, &execution, &config).is_err());
}
