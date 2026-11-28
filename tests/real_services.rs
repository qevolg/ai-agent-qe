use aaqe::guards::{contracts, patch::PatchGate};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn service_contracts_reject_missing_checks_and_service_substitution() {
    let mut artifacts = json!({
        "code-explorer":{"real_services":["influxdb-1.8"]},
        "bug-hunter":{"candidates":[{"id":"bug"}]},
        "test-designer":{"plans":[{"test_id":"query","candidate_id":"bug","blocked_reason":null,"real_services":["influxdb-1.8"]}]}
    });
    contracts::validate_links("test-designer", &artifacts["test-designer"], &artifacts).unwrap();
    let mut downgraded = artifacts["test-designer"].clone();
    downgraded["plans"][0]["real_services"] = json!([]);
    assert!(contracts::validate_links("test-designer", &downgraded, &artifacts).is_err());
    let mut developer = json!({"summary":"test","patch":"diff","tests":[{
        "id":"query","candidate_id":"bug","command":["query"],"reproduction_commands":[]
    }],"limitations":[]});
    assert!(
        contracts::validate("test-developer", &developer).is_err(),
        "missing service_checks was accepted"
    );
    developer["tests"][0]["service_checks"] = json!([]);
    assert!(
        contracts::validate_links("test-developer", &developer, &artifacts).is_err(),
        "required real service was dropped"
    );
    developer["tests"][0]["service_checks"] = json!([{"service":"other","command":["check"]}]);
    assert!(contracts::validate_links("test-developer", &developer, &artifacts).is_err());
    developer["tests"][0]["service_checks"] =
        json!([{"service":"influxdb-1.8","command":["check"]}]);
    contracts::validate("test-developer", &developer).unwrap();
    contracts::validate_links("test-developer", &developer, &artifacts).unwrap();
    let check = developer["tests"][0]["service_checks"][0].clone();
    developer["tests"][0]["service_checks"]
        .as_array_mut()
        .unwrap()
        .push(check);
    assert!(contracts::validate_links("test-developer", &developer, &artifacts).is_err());
    artifacts["test-designer"]["plans"][0]["real_services"] = json!([]);
    developer["tests"][0]["service_checks"] = json!([]);
    contracts::validate_links("test-developer", &developer, &artifacts).unwrap();
}

#[test]
fn patch_gate_rejects_mock_clients_without_changing_the_worktree() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "AAQE Test"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    fs::write(root.join("README.md"), "Real services only\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "base"]);
    let gate = PatchGate::new(root, &["tests/*".into()], None).unwrap();
    fs::create_dir(root.join("tests")).unwrap();
    for (name, source) in [
        (
            "ClientTest.java",
            "import static org.mockito.Mockito.*;\nclass ClientTest {}\n",
        ),
        ("test_client.py", "from unittest.mock import patch\n"),
        ("client.test.ts", "vi.spyOn(client, 'query');\n"),
        ("client.rs", "use mockall::automock;\n"),
        (
            "client.go",
            "package tests\nimport \"github.com/DATA-DOG/go-sqlmock\"\n",
        ),
    ] {
        fs::create_dir_all(root.join("tests")).unwrap();
        let file = root.join("tests").join(name);
        fs::write(&file, source).unwrap();
        git(root, &["add", "."]);
        let patch = git(root, &["diff", "--cached"]);
        git(root, &["reset", "--hard", "HEAD"]);
        let error = gate.apply(&patch).expect_err("mock patch was accepted");
        assert!(error.to_string().contains("mock"), "{error}");
        assert!(!file.exists());
        assert!(git(root, &["status", "--porcelain"]).is_empty());
    }
    let patch = "diff --git a/tests/real.py b/tests/real.py\nnew file mode 100644\n--- /dev/null\n+++ b/tests/real.py\n@@ -0,0 +1,2 @@\n+# Do not mock the real service.\n+assert 2 + 2 == 4\n";
    gate.apply(patch).unwrap();
}

#[test]
fn verdicts_require_service_evidence_and_independent_no_mock_review() {
    let mut verdict = json!({"candidate_id":"bug","test_ids":["query"],"status":"confirmed","rationale":"assertion failed",
        "assessment":{"executed":true,"assertion_failed":true,"infrastructure_failure":false},
        "evidence":["/run/target/execution.json"]});
    let mut execution = json!({"baseline":[{"result":{"returncode":0}}],"tests":[{
        "test_id":"query","directory":"/run/target","evidence":{"complete":true},"result":{"returncode":1},"required_services":["influxdb-1.8"],
        "service_checks":[{"service":"influxdb-1.8","directory":"/run/check","result":{"returncode":0,"timed_out":false},
            "docker":{"project":"aaqe-test","service":"influxdb-1.8","containers":[{"project":"aaqe-test","service":"influxdb-1.8","running":true,"id":"container","image_id":"sha256:image","image":"influxdb:1.8.10"}]}}]
    }]});
    let status = |verdict: &Value, execution: &Value| {
        aaqe::audit::workflow::grounded_verdicts(&json!({"verdicts":[verdict]}), execution)[0]["status"].clone()
    };
    assert_eq!(
        status(&verdict, &execution),
        "blocked",
        "missing no-mock review was accepted"
    );
    verdict["assessment"]["no_mocks"] = json!(true);
    verdict["assessment"]["real_services_verified"] = json!(true);
    assert_eq!(status(&verdict, &execution), "confirmed");
    let check = execution["tests"][0]["service_checks"][0].clone();
    execution["tests"][0]["service_checks"][0]["docker"] = Value::Null;
    assert_eq!(
        status(&verdict, &execution),
        "blocked",
        "endpoint check without Docker identity was accepted"
    );
    execution["tests"][0]["service_checks"][0] = check;
    execution["tests"][0]["service_checks"][0]["result"]["returncode"] = json!(1);
    assert_eq!(status(&verdict, &execution), "blocked");
    execution["tests"][0]["service_checks"] = Value::Null;
    assert_eq!(
        status(&verdict, &execution),
        "blocked",
        "historical execution lacked service checks"
    );
    execution["tests"][0]["service_checks"] = json!([]);
    execution["tests"][0]["required_services"] = json!([]);
    verdict["assessment"]["no_mocks"] = json!(false);
    assert_eq!(status(&verdict, &execution), "invalid_test");
    verdict["status"] = json!("not_reproduced");
    execution["tests"][0]["result"]["returncode"] = json!(0);
    assert_eq!(status(&verdict, &execution), "invalid_test");
}

#[test]
fn completion_requires_every_target_and_current_independent_policy_review() {
    let approved = |execution: &Value, artifacts: &Value| {
        aaqe::guards::real_services::approved(
            execution,
            artifacts,
            &artifacts["test-validator"]["verdicts"],
        )
    };
    let artifacts = json!({
        "test-developer":{"tests":[{"id":"calculation","service_checks":[]}]},
        "test-validator":{"verdicts":[{"test_ids":["calculation"],"status":"not_reproduced","assessment":{
            "executed":true,"infrastructure_failure":false,"no_mocks":true,"real_services_verified":true
        }}]}
    });
    let execution = json!({"coverage":{"status":"passed"},"tests":[{
        "test_id":"calculation","target_executed":true,"evidence":{"complete":true},"required_services":[],"service_checks":[]
    }]});
    assert!(approved(&execution, &artifacts));
    for field in ["no_mocks", "real_services_verified", "executed"] {
        let mut rejected = artifacts.clone();
        rejected["test-validator"]["verdicts"][0]["assessment"][field] = json!(false);
        assert!(!approved(&execution, &rejected), "accepted false {field}");
    }
    let mut incomplete = execution.clone();
    incomplete["tests"][0]["target_executed"] = json!(false);
    assert!(!approved(&incomplete, &artifacts));
    let mut dropped = artifacts.clone();
    dropped["test-developer"]["tests"][0]["service_checks"] =
        json!([{"service":"influxdb","command":["check"]}]);
    assert!(
        !approved(&execution, &dropped),
        "execution omitted a required service"
    );
    let mut partial = artifacts.clone();
    partial["test-developer"]["tests"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"second","service_checks":[]}));
    assert!(
        !approved(&execution, &partial),
        "selected rerun completed an unexecuted target"
    );
}

#[test]
#[ignore = "requires Docker with real InfluxDB and TDengine images"]
fn real_docker_target_services_and_stopped_service_blocking() {
    use aaqe::{audit::workflow::collect_test, runtime::compose::ComposeEnvironment};
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let run = temp.path().join("run");
    // These are real database servers. No model process or runtime is replaced.
    let influx_check = r#"import urllib.request
with urllib.request.urlopen('http://influxdb:8086/ping', timeout=5) as response:
    assert response.status == 204
    version = response.headers['X-Influxdb-Version']
    assert version.startswith('1.8.'), version
    print('InfluxDB', version)
"#;
    let td_check = r#"import urllib.request, json
request = urllib.request.Request('http://tdengine:6041/rest/sql', data=b'select server_version()', headers={'Authorization':'Basic cm9vdDp0YW9zZGF0YQ=='})
with urllib.request.urlopen(request, timeout=5) as response:
    result = json.load(response)
    assert result['code'] == 0, result
    assert result['data'][0][0].startswith('3.'), result
    print('TDengine', result['data'][0][0])
"#;
    let readiness = format!(
        r#"import time
deadline = time.monotonic() + 90
while True:
    try:
        exec({influx_check:?})
        exec({td_check:?})
        break
    except Exception:
        if time.monotonic() >= deadline:
            raise
        time.sleep(1)
"#
    );
    let environment = json!({
        "compose":{"services":{
            "runner":{"image":"python:3.12-slim","command":["sleep","infinity"]},
            "influxdb":{"image":"influxdb:1.8.10"},
            "tdengine":{"image":"tdengine/tsdb-ee:3.4.3.12","hostname":"tdengine","environment":{"TAOS_FQDN":"tdengine"}}
        }},
        "runner_service":"runner","services":["runner","influxdb","tdengine"],
        "readiness":[["python3","-c",readiness]],"readiness_timeout":120
    });
    let runtime = ComposeEnvironment::new(
        &workspace,
        &run,
        &uuid::Uuid::new_v4().simple().to_string(),
        environment,
        180,
    );
    struct Cleanup<'a>(&'a ComposeEnvironment);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = self.0.cleanup();
        }
    }
    let _cleanup = Cleanup(&runtime);
    runtime.start().unwrap();
    let target = r#"import urllib.request, urllib.parse, json, pathlib
def influx(sql):
    url = 'http://influxdb:8086/query?' + urllib.parse.urlencode({'q':sql, 'db':'aaqe_real'})
    with urllib.request.urlopen(urllib.request.Request(url, data=b''), timeout=5) as response:
        result = json.load(response)
        assert not result.get('error'), result
        assert not any(part.get('error') for part in result['results']), result
        return result
influx('CREATE DATABASE aaqe_real')
request = urllib.request.Request('http://influxdb:8086/write?db=aaqe_real&precision=ns', data=b'real,series=source value=42i 1767225600000000001')
with urllib.request.urlopen(request, timeout=5) as response:
    assert response.status == 204
point = influx('SELECT value FROM real')['results'][0]['series'][0]['values'][0]
assert point == ['2026-01-01T00:00:00.000000001Z', 42], point
def td(sql):
    request = urllib.request.Request('http://tdengine:6041/rest/sql', data=sql.encode(), headers={'Authorization':'Basic cm9vdDp0YW9zZGF0YQ=='})
    with urllib.request.urlopen(request, timeout=5) as response:
        result = json.load(response)
        assert result['code'] == 0, result
        return result
td("CREATE DATABASE IF NOT EXISTS aaqe_real KEEP 36500 PRECISION 'ns'")
td('CREATE TABLE IF NOT EXISTS aaqe_real.points (ts TIMESTAMP, value INT)')
td('INSERT INTO aaqe_real.points VALUES (1767225600000000001, 42)')
result = td('SELECT COUNT(*), SUM(value) FROM aaqe_real.points')['data']
assert result == [[1, 42]], result
pathlib.Path('/aaqe-results/target-ran').write_text('real InfluxDB and TDengine read/write passed')
print('Real InfluxDB and TDengine read/write assertions passed')
"#;
    let test = json!({"id":"real-databases","candidate_id":"service-policy","command":["python3","-c",target],
        "service_checks":[{"service":"influxdb","command":["python3","-c",influx_check]},
            {"service":"tdengine","command":["python3","-c",td_check]}]});
    let record = collect_test(&runtime, &test, 60).unwrap();
    assert_eq!(record["result"]["returncode"], 0, "{record}");
    assert_eq!(record["target_executed"], true);
    assert_eq!(record["evidence"]["complete"], true);
    assert!(aaqe::guards::real_services::checks_verified(&record));
    assert!(run.join("executions/target-ran").is_file());
    for check in record["service_checks"].as_array().unwrap() {
        assert!(
            Path::new(check["directory"].as_str().unwrap())
                .join("execution.json")
                .is_file()
        );
        assert!(check["result"]["stdout"].as_str().unwrap().contains(
            if check["service"] == "influxdb" {
                "InfluxDB 1.8."
            } else {
                "TDengine 3."
            }
        ));
    }
    fs::remove_file(run.join("executions/target-ran")).unwrap();
    let container = record["service_checks"][0]["docker"]["containers"][0]["id"]
        .as_str()
        .unwrap();
    assert!(
        Command::new("docker")
            .args(["stop", "--time", "2", container])
            .status()
            .unwrap()
            .success()
    );
    let blocked = collect_test(&runtime, &test, 30).unwrap();
    assert_eq!(blocked["target_executed"], false);
    assert_eq!(blocked["evidence"]["complete"], false);
    assert_eq!(blocked["evidence"]["error"], "real_service_unavailable");
    assert!(!aaqe::guards::real_services::checks_verified(&blocked));
    let service_receipt = aaqe::state::read_json(
        &Path::new(blocked["service_checks"][0]["directory"].as_str().unwrap())
            .join("execution.json"),
    )
    .unwrap();
    assert!(
        service_receipt.get("target_executed").is_none(),
        "blocked target overwrote the original service-check receipt"
    );
    assert!(
        !run.join("executions/target-ran").exists(),
        "target ran after the real service stopped"
    );
    let mut legacy = test.clone();
    legacy.as_object_mut().unwrap().remove("service_checks");
    assert!(collect_test(&runtime, &legacy, 10).is_err());
    assert!(
        runtime
            .execute_service_check(
                &["python3".into(), "--version".into()],
                "runner",
                "runner-is-not-target",
                10
            )
            .is_err()
    );
    assert!(
        runtime
            .execute_service_check(
                &["python3".into(), "--version".into()],
                "absent",
                "missing-target",
                10
            )
            .is_err()
    );
    eprintln!(
        "Validated real Docker InfluxDB and TDengine, protocol/version evidence, data read/write, and stopped-service blocking"
    );
}
