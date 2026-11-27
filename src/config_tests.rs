use super::*;
use serde_json::json;
use std::fs;

#[test]
fn config_is_optional_and_resolves_paths_without_reading_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path();
    let default = Config::load(directory, None).unwrap();
    assert_eq!(default.remote, "origin");
    assert_eq!(default.branch, "main");
    assert_eq!(default.max_stage_calls, 20);
    assert_eq!(default.max_stage_attempts, 4);
    assert_eq!(default.run_timeout, 14400);
    assert!(default.state_dir.is_absolute());
    fs::write(
        directory.join("aaqe.yaml"),
        "loop: {max_stage_calls: 8, max_stage_attempts: 2, timeout: 3600}\nstate_dir: runs\ngit:\n  branch: develop\nenvironment:\n  env_file: private.env\n",
    )
    .unwrap();
    fs::write(directory.join("private.env"), "PASSWORD=do-not-publish\n").unwrap();
    let loaded = Config::load(directory, None).unwrap();
    assert_eq!(loaded.branch, "develop");
    assert_eq!(loaded.max_stage_calls, 8);
    assert_eq!(loaded.max_stage_attempts, 2);
    assert_eq!(loaded.run_timeout, 3600);
    assert_eq!(loaded.state_dir, directory.join("runs"));
    assert_eq!(
        loaded.environment["env_file"],
        directory.join("private.env").to_str().unwrap()
    );
    let snapshot = serde_json::to_value(&loaded).unwrap();
    assert!(!snapshot.to_string().contains("do-not-publish"));
    assert_eq!(
        serde_json::from_value::<Config>(snapshot).unwrap().branch,
        "develop"
    );
}

#[test]
fn invalid_config_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    for content in [
        "unknown: true",
        "git: []",
        "environment: {services: nope}",
        "timeouts: {test: 0}",
        "git: {remote: --upload-pack=bad}",
        "codex: {command: []}",
        "loop: {max_stage_calls: 0}",
        "loop: {max_stage_attempts: -1}",
        "loop: {timeout: 0}",
    ] {
        fs::write(temp.path().join("aaqe.yaml"), content).unwrap();
        assert!(Config::load(temp.path(), None).is_err(), "{content}");
    }
    assert!(Config::load(temp.path(), Some(&temp.path().join("missing.yaml"))).is_err());
}

#[test]
fn integral_float_config_timeouts_preserve_json_schema_integer_semantics() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("aaqe.yaml"),
        "timeouts: {agent: 3.0, test: 4.0, docker: 5.0}\n",
    )
    .unwrap();
    let config = crate::config::Config::load(temp.path(), None).unwrap();
    assert_eq!(
        (
            config.agent_timeout,
            config.test_timeout,
            config.docker_timeout
        ),
        (3, 4, 5)
    );
    assert_eq!(crate::config::unsigned_integer(&json!(7.0)), Some(7));
    assert_eq!(crate::config::unsigned_integer(&json!(7.1)), None);
    let saved: crate::config::Config = serde_json::from_value(
        json!({"agent_timeout":3.0,"test_timeout":4.0,"docker_timeout":5.0}),
    )
    .unwrap();
    assert_eq!(
        (
            saved.agent_timeout,
            saved.test_timeout,
            saved.docker_timeout
        ),
        (3, 4, 5)
    );
}

#[test]
fn yaml_merge_keys_keep_inherited_settings_and_explicit_overrides() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("aaqe.yaml"), "environment:\n  <<: &runner_defaults\n    runner_workdir: /workspace\n    services: [runner]\n    runner_service: inherited\n  runner_service: runner\n").unwrap();
    let config = crate::config::Config::load(temp.path(), None).unwrap();
    assert_eq!(
        config.environment,
        json!({"runner_workdir":"/workspace", "services":["runner"], "runner_service":"runner"})
    );
}
