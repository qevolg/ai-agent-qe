use super::*;
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
    state["reproductions"] = json!([{"id":"repro-1","status":"failed","error":"readiness failed"}]);
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
