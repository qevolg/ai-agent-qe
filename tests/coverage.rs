use aaqe::guards::coverage::read_coverage;
use serde_json::json;
use std::fs;

#[test]
fn delivery_requires_both_threshold_and_current_independent_approval() {
    let mut execution = json!({"coverage":{"status":"passed"}});
    let mut artifacts = json!({"test-validator":{"coverage_assessment":{"scope_valid":true,"measurement_valid":true,"meaningful_assertions":true,"rationale":"Verified"}}});
    assert!(aaqe::guards::coverage::approved(&execution, &artifacts));
    for field in ["scope_valid", "measurement_valid", "meaningful_assertions"] {
        artifacts["test-validator"]["coverage_assessment"][field] = json!(false);
        assert!(!aaqe::guards::coverage::approved(&execution, &artifacts));
        artifacts["test-validator"]["coverage_assessment"][field] = json!(true);
    }
    execution["coverage"]["status"] = json!("below_threshold");
    assert!(!aaqe::guards::coverage::approved(&execution, &artifacts));
    execution["coverage"]["status"] = json!("passed");
    artifacts["test-validator"] = serde_json::Value::Null;
    assert!(!aaqe::guards::coverage::approved(&execution, &artifacts));
}

#[test]
fn coverage_requires_eighty_percent_and_original_measurement() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let evidence = temp.path().join("evidence");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&evidence).unwrap();
    fs::write(workspace.join("app.rs"), "source\n".repeat(100)).unwrap();
    fs::write(evidence.join("raw.txt"), "coverage tool output").unwrap();
    let specification = json!({"report":"coverage.json","files":["app.rs"]});
    for (covered, status) in [(79, "below_threshold"), (80, "passed"), (100, "passed")] {
        fs::write(evidence.join("coverage.json"), json!({"files":[{"path":"app.rs","covered_lines":covered,"executable_lines":100}],"raw_reports":["raw.txt"]}).to_string()).unwrap();
        let result = read_coverage(&workspace, &evidence, &specification).unwrap();
        assert_eq!(result["status"], status);
        assert_eq!(result["covered_lines"], covered);
        assert_eq!(result["executable_lines"], 100);
    }
    fs::remove_file(evidence.join("raw.txt")).unwrap();
    assert!(read_coverage(&workspace, &evidence, &specification).is_err());
}

#[test]
fn coverage_rejects_incomplete_impossible_duplicate_and_unsafe_reports() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("app.rs"), "source\n".repeat(10)).unwrap();
    fs::write(temp.path().join("raw.txt"), "measurement").unwrap();
    let specification = json!({"report":"coverage.json","files":["app.rs"]});
    for files in [
        json!([]),
        json!([{"path":"app.rs","covered_lines":11,"executable_lines":10}]),
        json!([{"path":"app.rs","covered_lines":10,"executable_lines":11}]),
        json!([{"path":"app.rs","covered_lines":0,"executable_lines":0}]),
        json!([{"path":"app.rs","covered_lines":1,"executable_lines":1},{"path":"app.rs","covered_lines":1,"executable_lines":1}]),
        json!([{"path":"../app.rs","covered_lines":1,"executable_lines":1}]),
    ] {
        fs::write(
            temp.path().join("coverage.json"),
            json!({"files":files,"raw_reports":["raw.txt"]}).to_string(),
        )
        .unwrap();
        assert!(read_coverage(temp.path(), temp.path(), &specification).is_err());
    }
}
