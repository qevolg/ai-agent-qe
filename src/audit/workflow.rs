use crate::{
    agents::codex::Codex,
    config::Config,
    guards::patch::PatchGate,
    reporting::refresh_report,
    runtime::compose::{ComposeEnvironment, strings},
    state::{HarnessSnapshot, RunLock, Store, now, read_json, write_json},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path};

use super::interfaces::{AuditAgent, RuntimeFactory, TestRuntime};

pub fn collect_test(runtime: &dyn TestRuntime, test: &Value, timeout: u64) -> Result<Value> {
    let mut record = runtime.execute_test(
        &strings(&test["command"]),
        test["id"].as_str().context("Missing test identity")?,
        timeout,
    )?;
    record["candidate_id"] = test["candidate_id"].clone();
    let code = record["result"]["returncode"].as_i64();
    let complete = code
        .is_some_and(|code| code >= 0 && ![124, 125, 126, 127, 137, 143].contains(&code))
        && record["result"]["timed_out"] != true;
    record["evidence"] = json!({
        "complete": complete,
        "error": if complete { None } else { Some("execution_interrupted_or_start_failed") }
    });
    write_json(
        &Path::new(record["directory"].as_str().unwrap()).join("execution.json"),
        &record,
    )?;
    Ok(record)
}

pub fn grounded_verdicts(artifact: &Value, execution: &Value) -> Value {
    let empty = Vec::new();
    let records = execution["tests"].as_array().unwrap_or(&empty);
    let baseline = execution["baseline"].as_array().unwrap_or(&empty);

    let mut verdicts = Vec::new();
    for original in artifact["verdicts"].as_array().unwrap_or(&empty) {
        let mut verdict = original.clone();
        let ids = strings(&verdict["test_ids"]);
        let supporting: Vec<&Value> = ids
            .iter()
            .filter_map(|id| records.iter().find(|r| r["test_id"].as_str() == Some(id)))
            .collect();
        let complete = !ids.is_empty()
            && supporting.len() == ids.len()
            && supporting.iter().all(|record| {
                record["evidence"]["complete"] == true
                    || (record["evidence"].is_null() && record["junit"]["complete"] == true)
            });
        let healthy = ids.iter().all(|id| {
            let applicable: Vec<_> = baseline
                .iter()
                .filter(|r| r["for_test_id"].is_null() || r["for_test_id"].as_str() == Some(id))
                .collect();
            !applicable.is_empty()
                && applicable
                    .iter()
                    .all(|r| r["result"]["returncode"] == 0 && r["result"]["timed_out"] != true)
        });
        let legacy_failed = supporting.iter().any(|r| {
            r["junit"]["cases"]
                .as_array()
                .is_some_and(|cases| cases.iter().any(|c| c["status"] == "failed"))
        });
        let allowed: HashSet<String> = supporting
            .iter()
            .filter_map(|r| r["directory"].as_str())
            .map(|dir| {
                Path::new(dir)
                    .join("execution.json")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        if verdict["status"] == "confirmed" || verdict["status"] == "not_reproduced" {
            let evidence = strings(&verdict["evidence"]);
            let assessment = &verdict["assessment"];
            let legacy = supporting.iter().all(|record| record["evidence"].is_null());
            let assertion_failed =
                assessment["assertion_failed"] == true || (legacy && legacy_failed);
            let assessed = (assessment["executed"] == true
                && assessment["infrastructure_failure"] == false)
                || (legacy && assessment.is_null());
            let valid = complete
                && assessed
                && healthy
                && !evidence.is_empty()
                && evidence.iter().all(|p| allowed.contains(p))
                && (if verdict["status"] == "confirmed" {
                    assertion_failed
                        && supporting.iter().any(|record| {
                            record["result"]["returncode"]
                                .as_i64()
                                .is_some_and(|code| code != 0)
                                || legacy
                        })
                } else {
                    !assertion_failed
                        && supporting
                            .iter()
                            .all(|record| record["result"]["returncode"] == 0 || legacy)
                });
            if !valid {
                verdict["agent_status"] = verdict["status"].clone();
                verdict["status"] = json!("blocked");
                verdict["rationale"] = json!(format!(
                    "Execution gate: incomplete, inconsistent or absent target/baseline evidence. Agent assessment: {}",
                    verdict["rationale"].as_str().unwrap_or("")
                ));
            }
        }
        verdict["execution_records"] = json!(
            supporting
                .iter()
                .filter_map(|r| r["directory"].as_str())
                .map(|p| Path::new(p)
                    .join("execution.json")
                    .to_string_lossy()
                    .into_owned())
                .collect::<Vec<_>>()
        );
        verdicts.push(verdict);
    }
    json!(verdicts)
}

pub fn run_audit(
    prompt: &str,
    caller: &Path,
    config: &Config,
    progress: &dyn Fn(&str),
) -> Result<Value> {
    let agent = Codex::new(config);
    run_audit_with(
        prompt,
        caller,
        config,
        &agent,
        &|workspace, directory, id, spec, timeout| {
            Box::new(ComposeEnvironment::new(
                workspace, directory, id, spec, timeout,
            ))
        },
        progress,
    )
}

pub fn run_audit_with(
    prompt: &str,
    caller: &Path,
    config: &Config,
    agent: &dyn AuditAgent,
    factory: &RuntimeFactory<'_>,
    progress: &dyn Fn(&str),
) -> Result<Value> {
    crate::audit::harness::audit(prompt, caller, config, agent, factory, progress)
}

pub fn reproduce(store: &Store, state: &mut Value, case_id: Option<&str>) -> Result<Value> {
    let directory = store.path(state["id"].as_str().context("Missing run identity")?)?;
    let _lock = RunLock::acquire(&directory)?;
    *state = store.load(state["id"].as_str().unwrap())?;
    ensure!(
        state["status"] != "initializing" && state["status"] != "running",
        "Cannot reproduce while the audit is running"
    );
    ensure!(
        state["audit_finalized"] != false,
        "Cannot reproduce while audit finalization is pending"
    );
    ensure!(
        state["environment_cleaned_at"].is_null(),
        "This run's environment has already been cleaned"
    );
    ensure!(
        state["environment"].is_object(),
        "This run has no test environment"
    );
    let config: Config = serde_json::from_value(state["config"].clone())?;
    let artifact = if directory.join("harness.json").exists() {
        HarnessSnapshot::load(&directory)?
            .artifacts
            .get("test-developer")
            .cloned()
            .context("This run has no accepted test-developer artifact")?
    } else {
        read_json(&directory.join("artifacts/test-developer.json"))?
    };
    let tests: Vec<Value> = artifact["tests"]
        .as_array()
        .context("Missing saved tests")?
        .iter()
        .filter(|test| case_id.is_none_or(|id| test["id"].as_str() == Some(id)))
        .cloned()
        .collect();
    ensure!(!tests.is_empty(), "Unknown test ID");
    let root_path = std::path::PathBuf::from(
        state["workspace"]["path"]
            .as_str()
            .context("Missing saved worktree")?,
    );
    let root = root_path.as_path();
    let gate = PatchGate::new(root, state["workspace"]["commit"].as_str())?.with_patch(
        artifact["patch"]
            .as_str()
            .context("Missing accepted patch")?,
    );
    gate.verify_worktree()?;
    let runtime = ComposeEnvironment::new(
        root,
        &directory,
        state["id"].as_str().unwrap(),
        state["environment"].clone(),
        config.docker_timeout,
    );
    if !state["reproductions"].is_array() {
        state["reproductions"] = json!([]);
    }
    for previous in state["reproductions"].as_array_mut().unwrap() {
        if previous["status"] == "running" {
            previous["status"] = json!("interrupted");
            previous["finished_at"] = json!(now());
            previous["error"] = json!("Previous reproduction exited before recording completion");
        }
    }
    let entry = state["reproductions"].as_array().unwrap().len();
    let attempt = uuid::Uuid::new_v4().simple().to_string();
    let evidence = directory.join("reproductions").join(&attempt);
    state["reproductions"].as_array_mut().unwrap().push(json!({"id":attempt,"at":now(),"status":"running","phase":"environment_start","directory":evidence,"test_ids":tests.iter().map(|test|test["id"].clone()).collect::<Vec<_>>(),"preparations":[],"records":[]}));
    let save_attempt = |state: &Value| -> Result<()> {
        write_json(
            &evidence.join("reproduction.json"),
            &state["reproductions"][entry],
        )?;
        store.save(state)
    };
    save_attempt(state)?;
    let outcome = (|| -> Result<()> {
        runtime.start()?;
        for test in tests {
            state["reproductions"][entry]["test_id"] = test["id"].clone();
            state["reproductions"][entry]["phase"] = json!("preparation");
            save_attempt(state)?;
            for (index, command) in test["reproduction_commands"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
            {
                let preparation = runtime.execute_test(
                    &strings(command),
                    &format!("{}-prepare-{}", test["id"].as_str().unwrap(), index + 1),
                    config.test_timeout,
                )?;
                state["reproductions"][entry]["preparations"]
                    .as_array_mut()
                    .unwrap()
                    .push(preparation.clone());
                save_attempt(state)?;
                ensure!(
                    preparation["result"]["returncode"] == 0,
                    "Reproduction preparation failed; original evidence retained"
                );
            }
            state["reproductions"][entry]["phase"] = json!("test_execution");
            save_attempt(state)?;
            let record = collect_test(&runtime, &test, config.test_timeout)?;
            state["reproductions"][entry]["records"]
                .as_array_mut()
                .unwrap()
                .push(record);
            save_attempt(state)?;
            gate.verify_worktree()?;
        }
        if artifact["coverage"].is_object() {
            state["reproductions"][entry]["phase"] = json!("coverage_measurement");
            save_attempt(state)?;
            let specification = &artifact["coverage"];
            let record = runtime.execute_test(
                &strings(&specification["command"]),
                &format!("coverage-replay-{attempt}"),
                config.test_timeout,
            )?;
            state["reproductions"][entry]["coverage"] = json!({"status":"blocked","record":record});
            save_attempt(state)?;
            let measurement = (|| -> Result<Value> {
                let code = record["result"]["returncode"]
                    .as_i64()
                    .context("Coverage process has no exit status")?;
                ensure!(
                    record["result"]["timed_out"] != true && (0..=1).contains(&code),
                    "Coverage measurement failed or timed out"
                );
                let mut measurement = crate::guards::coverage::read_coverage(
                    root,
                    Path::new(
                        record["directory"]
                            .as_str()
                            .context("Missing coverage directory")?,
                    ),
                    specification,
                )?;
                gate.verify_worktree()?;
                measurement["record"] = record;
                Ok(measurement)
            })();
            match measurement {
                Ok(measurement) => state["reproductions"][entry]["coverage"] = measurement,
                Err(error) => {
                    state["reproductions"][entry]["coverage"]["error"] =
                        json!(format!("{error:#}"));
                    return Err(error);
                }
            }
            save_attempt(state)?;
        }
        Ok(())
    })();
    state["reproductions"][entry]["status"] = json!(if crate::runtime::process::interrupted() {
        "interrupted"
    } else if outcome.is_ok() {
        "completed"
    } else {
        "failed"
    });
    state["reproductions"][entry]["finished_at"] = json!(now());
    if let Err(error) = &outcome {
        state["reproductions"][entry]["error"] = json!(format!("{error:#}"));
    }
    save_attempt(state)?;
    let report = refresh_report(&directory, state);
    outcome?;
    report?;
    Ok(state["reproductions"][entry]["records"].clone())
}

#[cfg(test)]
mod tests {
    use super::{collect_test, grounded_verdicts};
    use crate::audit::interfaces::TestRuntime;
    use anyhow::Result;
    use serde_json::json;

    #[test]
    fn execution_evidence_does_not_require_a_framework_report() {
        struct Runtime(std::path::PathBuf);

        impl TestRuntime for Runtime {
            fn start(&self) -> Result<()> {
                Ok(())
            }

            fn logs(&self) -> Result<()> {
                Ok(())
            }

            fn execute_test(&self, _: &[String], id: &str, _: u64) -> Result<serde_json::Value> {
                Ok(
                    json!({"test_id":id,"directory":self.0,"result":{"returncode":1,"timed_out":false,"stdout":"regression executed: assertion failed","stderr":""}}),
                )
            }
        }

        let directory = tempfile::tempdir().unwrap();
        let record = collect_test(
            &Runtime(directory.path().into()),
            &json!({"id":"regression","candidate_id":"bug","command":["test"]}),
            10,
        )
        .unwrap();
        assert_eq!(record["evidence"]["complete"], true);
        assert!(record.get("junit").is_none());
        let artifact = json!({"verdicts":[{"candidate_id":"bug","test_ids":["regression"],"status":"confirmed","rationale":"assertion failed","assessment":{"executed":true,"assertion_failed":true,"infrastructure_failure":false},"evidence":[directory.path().join("execution.json")]}]});
        let execution = json!({"baseline":[{"result":{"returncode":0}}],"tests":[record]});
        assert_eq!(
            grounded_verdicts(&artifact, &execution)[0]["status"],
            "confirmed"
        );
        let mut invalid = artifact.clone();
        invalid["verdicts"][0]["assessment"]["infrastructure_failure"] = json!(true);
        assert_eq!(
            grounded_verdicts(&invalid, &execution)[0]["status"],
            "blocked"
        );
        for assessment in [
            serde_json::Value::Null,
            json!({"executed":false,"assertion_failed":true,"infrastructure_failure":false}),
            json!({"executed":true,"assertion_failed":false,"infrastructure_failure":false}),
        ] {
            invalid["verdicts"][0]["assessment"] = assessment;
            assert_eq!(
                grounded_verdicts(&invalid, &execution)[0]["status"],
                "blocked"
            );
        }
    }

    #[test]
    fn execution_gate_requires_real_matching_failure_and_healthy_baseline() {
        let artifact = json!({"verdicts":[{"candidate_id":"bug", "test_ids":["test"], "status":"confirmed", "rationale":"failed assertion", "evidence":["/run/attempt/execution.json"]}]});
        let execution = json!({"baseline":[{"result":{"returncode":0}}],"tests":[{"test_id":"test", "directory":"/run/attempt", "junit":{"complete":true,"cases":[{"status":"failed"}]}}]});
        assert_eq!(
            grounded_verdicts(&artifact, &execution)[0]["status"],
            "confirmed"
        );
        let mut missing = execution.clone();
        missing["tests"][0]["junit"]["complete"] = json!(false);
        assert_eq!(
            grounded_verdicts(&artifact, &missing)[0]["status"],
            "blocked"
        );
        let mut fabricated = artifact.clone();
        fabricated["verdicts"][0]["evidence"] = json!(["made-up.json"]);
        assert_eq!(
            grounded_verdicts(&fabricated, &execution)[0]["status"],
            "blocked"
        );
        let mut baseline = execution.clone();
        baseline["baseline"] = json!([]);
        assert_eq!(
            grounded_verdicts(&artifact, &baseline)[0]["status"],
            "blocked"
        );
    }
}
