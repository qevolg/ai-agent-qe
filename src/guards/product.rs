//! Relationships and evidence for documented product scenarios.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

pub fn items<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    value[key]
        .as_array()
        .with_context(|| format!("Missing product array: {key}"))
}

fn unique<'a>(values: &'a [Value], key: &str) -> Result<HashMap<&'a str, &'a Value>> {
    let mut result = HashMap::new();
    for value in values {
        let id = value[key].as_str().context("Missing product identity")?;
        ensure!(
            result.insert(id, value).is_none(),
            "Duplicate product identity: {id}"
        );
    }
    Ok(result)
}

pub fn version(scenario: &Value) -> String {
    let mut scenario = scenario.clone();
    scenario.sort_all_objects();
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&scenario).expect("JSON value"))
    )
}

pub fn validate_services(explorer: &Value, prompt: &str) -> Result<()> {
    let services = items(explorer, "services")?;
    unique(services, "id")?;
    let runner = explorer["environment"]["runner_service"].as_str();
    let mut docker_names = HashSet::new();
    for service in services {
        match service["source"].as_str() {
            Some("external") => {
                let endpoint = service["endpoint"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .context("External service requires its explicit endpoint")?;
                ensure!(
                    explicit_endpoint(prompt, endpoint),
                    "External service endpoint was not explicitly provided in the run prompt: {endpoint}"
                );
                ensure!(
                    service["compose_service"].is_null(),
                    "External service must not have a Docker replacement"
                );
            }
            Some("docker") => {
                let name = service["compose_service"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .context("Unspecified services require a real Docker Compose target")?;
                ensure!(
                    Some(name) != runner && docker_names.insert(name),
                    "Docker targets must be unique and separate from the runner"
                );
                ensure!(
                    service["endpoint"].is_null(),
                    "Docker endpoint is discovered from its real Compose service"
                );
            }
            _ => anyhow::bail!("Unknown real-service source"),
        }
        crate::runtime::compose::command_list(&json!([service["check"]]), "real-service check")?;
    }
    Ok(())
}

fn explicit_endpoint(prompt: &str, endpoint: &str) -> bool {
    let address_char = |c: char| c.is_ascii_alphanumeric() || ":/?#@%._-=&+~!$*;[]".contains(c);
    prompt.match_indices(endpoint).any(|(start, _)| {
        let before = prompt[..start].chars().next_back();
        let after = prompt[start + endpoint.len()..].chars().next();
        // A prose colon may introduce an address; an address continuation must
        // not turn a declared host, port or path into an accepted prefix.
        before.is_none_or(|c| c == ':' || !address_char(c))
            && after.is_none_or(|c| !address_char(c))
    })
}

pub fn validate_links(stage: &str, output: &Value, artifacts: &Value) -> Result<()> {
    match stage {
        "product-explorer" => {
            unique(items(output, "services")?, "id")?;
            unique(items(output, "entry_points")?, "id")?;
            unique(items(output, "references")?, "id")?;
        }
        "scenario-designer" => {
            let scenarios = unique(items(output, "scenarios")?, "id")?;
            let services = unique(items(&artifacts["product-explorer"], "services")?, "id")?;
            let entries = items(&artifacts["product-explorer"], "entry_points")?;
            for scenario in scenarios.values() {
                unique(items(scenario, "steps")?, "id")?;
                let entry = scenario["entry_point"]
                    .as_str()
                    .context("Missing scenario entry point")?;
                ensure!(
                    entry == "cross_entry"
                        || entries
                            .iter()
                            .any(|e| e["kind"] == entry && e["applicable"] == true),
                    "Scenario uses an unknown or inapplicable entry point"
                );
                let required = items(scenario, "services")?;
                ensure!(
                    !required.is_empty(),
                    "Product scenarios require a real target service"
                );
                let mut seen = HashSet::new();
                for service in required {
                    let id = service.as_str().context("Invalid scenario service")?;
                    ensure!(
                        services.contains_key(id) && seen.insert(id),
                        "Unknown or duplicate scenario service: {id}"
                    );
                }
            }
            for coverage in items(output, "coverage")? {
                for id in items(coverage, "scenario_ids")? {
                    ensure!(
                        id.as_str().is_some_and(|id| scenarios.contains_key(id)),
                        "Coverage references an unknown scenario"
                    );
                }
            }
        }
        "product-executor" | "issue-reproducer" => {
            let scenarios = unique(items(&artifacts["scenario-designer"], "scenarios")?, "id")?;
            unique(items(output, "operations")?, "id")?;
            for operation in items(output, "operations")? {
                let scenario = scenarios
                    .get(operation["scenario_id"].as_str().unwrap_or(""))
                    .context("Operation references an undocumented scenario")?;
                ensure!(
                    scenario["blocked_reason"].is_null() || scenario["blocked_reason"] == "",
                    "Cannot execute a blocked scenario"
                );
                ensure!(
                    items(scenario, "steps")?
                        .iter()
                        .any(|step| step["id"] == operation["step_id"]),
                    "Operation references an undocumented step"
                );
                crate::runtime::compose::command_list(
                    &json!([operation["command"]]),
                    "product operation",
                )?;
            }
            if stage == "issue-reproducer" {
                unique(items(output, "issues")?, "id")?;
                for issue in items(output, "issues")? {
                    ensure!(
                        !items(issue, "scenario_ids")?.is_empty(),
                        "Issue requires a tested scenario"
                    );
                    for id in items(issue, "scenario_ids")? {
                        ensure!(
                            id.as_str().is_some_and(|id| scenarios.contains_key(id)),
                            "Issue references an unknown scenario"
                        );
                    }
                }
                for followup in items(output, "followups")? {
                    ensure!(
                        followup["scenario_id"]
                            .as_str()
                            .is_some_and(|id| scenarios.contains_key(id)),
                        "Followup references an unknown source scenario"
                    );
                }
            }
        }
        "result-reviewer" => {
            let results = unique(items(output, "scenario_results")?, "scenario_id")?;
            let scenarios = unique(items(&artifacts["scenario-designer"], "scenarios")?, "id")?;
            ensure!(
                results.keys().copied().collect::<HashSet<_>>()
                    == scenarios.keys().copied().collect::<HashSet<_>>(),
                "Independent review must cover every current scenario"
            );
            let results = unique(items(output, "issue_results")?, "issue_id")?;
            let issues = unique(items(&artifacts["issue-reproducer"], "issues")?, "id")?;
            ensure!(
                results.keys().copied().collect::<HashSet<_>>()
                    == issues.keys().copied().collect::<HashSet<_>>(),
                "Independent review must cover every current issue"
            );
        }
        _ => anyhow::bail!("Unknown product stage: {stage}"),
    }
    Ok(())
}

pub fn record_complete(record: &Value, scenario: &Value) -> bool {
    record["target_executed"] == true
        && record["evidence"]["complete"] == true
        && record["scenario_version"] == version(scenario)
        && record["required_services"] == scenario["services"]
        && record["service_checks"].as_array().is_some_and(|checks| {
            let required = scenario["services"].as_array().unwrap();
            checks.len() == required.len()
                && required.iter().all(|id| {
                    checks.iter().any(|check| {
                        check["service"] == *id
                            && check["result"]["returncode"] == 0
                            && check["result"]["timed_out"] == false
                            && (if check["source"] == "external" {
                                check["endpoint"].as_str().is_some_and(|s| !s.is_empty())
                            } else {
                                let mut identity = check.clone();
                                identity["service"] = check["compose_service"].clone();
                                crate::guards::real_services::check_verified(&identity)
                            })
                    })
                })
        })
}

fn path(record: &Value) -> Option<String> {
    record["directory"].as_str().map(|p| {
        Path::new(p)
            .join("execution.json")
            .to_string_lossy()
            .into_owned()
    })
}

pub fn ground_review(output: &Value, artifacts: &Value, execution: &Value) -> Value {
    let mut grounded = output.clone();
    let operations = execution["operations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let scenarios = artifacts["scenario-designer"]["scenarios"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for result in grounded["scenario_results"].as_array_mut().unwrap() {
        if !matches!(
            result["status"].as_str(),
            Some("passed" | "failed" | "flaky")
        ) {
            continue;
        }
        let scenario = scenarios
            .iter()
            .find(|s| s["id"] == result["scenario_id"])
            .unwrap();
        let records: Vec<_> = operations
            .iter()
            .filter(|r| r["scenario_id"] == scenario["id"] && record_complete(r, scenario))
            .collect();
        let evidence = result["evidence"].as_array().unwrap();
        let allowed: HashSet<_> = records.iter().filter_map(|r| path(r)).collect();
        let assessment = &result["assessment"];
        let complete = !evidence.is_empty()
            && evidence
                .iter()
                .all(|e| e.as_str().is_some_and(|p| allowed.contains(p)))
            && scenario["steps"].as_array().unwrap().iter().all(|step| {
                records.iter().any(|r| {
                    r["step_id"] == step["id"]
                        && r["purpose"] != "cleanup"
                        && path(r).is_some_and(|p| evidence.iter().any(|e| *e == p))
                })
            })
            && [
                "executed",
                "no_mocks",
                "real_services_verified",
                "entry_point_verified",
            ]
            .iter()
            .all(|key| assessment[*key] == true)
            && assessment["infrastructure_failure"] == false
            && (result["status"] != "passed" || assessment["expectation_met"] == true)
            && (result["status"] != "failed" || assessment["expectation_met"] == false);
        if !complete {
            result["agent_status"] = result["status"].clone();
            result["status"] = json!("blocked");
            result["rationale"] = json!(format!(
                "执行证据不足或与当前操作文档不一致；复核说明：{}",
                result["rationale"].as_str().unwrap_or("")
            ));
        }
    }
    let results = grounded["scenario_results"].as_array().unwrap().clone();
    for result in grounded["issue_results"].as_array_mut().unwrap() {
        if !matches!(
            result["status"].as_str(),
            Some("confirmed" | "not_reproduced" | "flaky")
        ) {
            continue;
        }
        let issue = artifacts["issue-reproducer"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == result["issue_id"])
            .unwrap();
        let ids = issue["scenario_ids"].as_array().unwrap();
        let related: Vec<_> = results
            .iter()
            .filter(|s| ids.contains(&s["scenario_id"]))
            .collect();
        let allowed: HashSet<_> = operations
            .iter()
            .filter(|r| ids.contains(&r["scenario_id"]))
            .filter(|r| {
                scenarios
                    .iter()
                    .any(|s| s["id"] == r["scenario_id"] && record_complete(r, s))
            })
            .filter_map(path)
            .collect();
        let evidence = result["evidence"].as_array().unwrap();
        let valid = !related.is_empty()
            && related
                .iter()
                .all(|s| matches!(s["status"].as_str(), Some("passed" | "failed" | "flaky")))
            && !evidence.is_empty()
            && evidence
                .iter()
                .all(|e| e.as_str().is_some_and(|p| allowed.contains(p)))
            && (result["status"] != "confirmed" || related.iter().any(|s| s["status"] == "failed"))
            && (result["status"] != "not_reproduced"
                || related.iter().all(|s| s["status"] == "passed"));
        if !valid {
            result["agent_status"] = result["status"].clone();
            result["status"] = json!("blocked");
            result["rationale"] = json!("缺少支持该问题结论的当前场景执行证据");
        }
    }
    grounded
}
