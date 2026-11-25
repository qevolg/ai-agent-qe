use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

fn full_schema(stage: &str) -> Result<Value> {
    let text = match stage {
        "code-explorer" => include_str!("resources/schemas/code-explorer.json"),
        "bug-hunter" => include_str!("resources/schemas/bug-hunter.json"),
        "test-designer" => include_str!("resources/schemas/test-designer.json"),
        "test-developer" => include_str!("resources/schemas/test-developer.json"),
        "test-validator" => include_str!("resources/schemas/test-validator.json"),
        _ => bail!("Unknown stage: {stage}"),
    };
    Ok(serde_json::from_str(text)?)
}

pub fn schema(stage: &str) -> Result<Value> {
    fn model_subset(value: &mut Value) {
        match value {
            Value::Object(map) => {
                map.remove("uniqueItems");
                for value in map.values_mut() {
                    model_subset(value);
                }
            }
            Value::Array(items) => {
                for item in items {
                    model_subset(item);
                }
            }
            _ => {}
        }
    }
    let mut schema = full_schema(stage)?;
    model_subset(&mut schema);
    Ok(schema)
}

pub fn validate(stage: &str, artifact: &Value) -> Result<()> {
    let schema = full_schema(stage)?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|e| anyhow::anyhow!("Invalid {stage} output schema: {e}"))?;
    if let Err(error) = validator.validate(artifact) {
        bail!("Invalid {stage} output contract: {error}");
    }
    Ok(())
}

fn items<'a>(artifact: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    artifact[key]
        .as_array()
        .with_context(|| format!("Invalid artifact array: {key}"))
}

fn unique(items: &[Value], key: &str, label: &str) -> Result<()> {
    let mut seen = HashSet::new();
    for item in items {
        let value = item[key]
            .as_str()
            .with_context(|| format!("Invalid {label} identity"))?;
        if !seen.insert(value) {
            bail!("Duplicate {label} identity");
        }
    }
    Ok(())
}

fn identities<'a>(items: &'a [Value], key: &str) -> HashSet<&'a str> {
    items.iter().filter_map(|item| item[key].as_str()).collect()
}

pub fn validate_links(stage: &str, artifact: &Value, artifacts: &Value) -> Result<()> {
    match stage {
        "code-explorer" => unique(items(artifact, "references")?, "id", "reference")?,
        "bug-hunter" => {
            let candidates = items(artifact, "candidates")?;
            unique(candidates, "id", "candidate")?;
            let references = identities(items(&artifacts["code-explorer"], "references")?, "id");
            for candidate in candidates {
                for evidence in items(candidate, "evidence_ids")? {
                    if !evidence.as_str().is_some_and(|id| references.contains(id)) {
                        bail!("Candidate references unknown source evidence");
                    }
                }
            }
        }
        "test-designer" => {
            let plans = items(artifact, "plans")?;
            unique(plans, "test_id", "test plan")?;
            let candidates = identities(items(&artifacts["bug-hunter"], "candidates")?, "id");
            if identities(plans, "candidate_id") != candidates {
                bail!("Test plans must cover every candidate");
            }
        }
        "test-developer" => {
            let tests = items(artifact, "tests")?;
            unique(tests, "id", "test")?;
            let plans: HashMap<_, _> = items(&artifacts["test-designer"], "plans")?
                .iter()
                .filter(|plan| plan["blocked_reason"].is_null() || plan["blocked_reason"] == "")
                .filter_map(|plan| plan["test_id"].as_str().map(|id| (id, plan)))
                .collect();
            if plans.keys().copied().collect::<HashSet<_>>() != identities(tests, "id") {
                bail!("Generated tests must match all unblocked test plans");
            }
            for test in tests {
                let plan = plans
                    .get(test["id"].as_str().context("Invalid test identity")?)
                    .context("Unknown test plan")?;
                if plan["candidate_id"] != test["candidate_id"] {
                    bail!("Generated test refers to the wrong candidate");
                }
            }
        }
        "test-validator" => {
            let verdicts = items(artifact, "verdicts")?;
            unique(verdicts, "candidate_id", "verdict")?;
            let candidates = identities(items(&artifacts["bug-hunter"], "candidates")?, "id");
            if identities(verdicts, "candidate_id") != candidates {
                bail!("Validation must cover every candidate");
            }
            let tests: HashMap<_, _> = artifacts["test-developer"]["tests"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|test| test["id"].as_str().map(|id| (id, test)))
                .collect();
            for verdict in verdicts {
                for id in items(verdict, "test_ids")? {
                    let Some(test) = id.as_str().and_then(|id| tests.get(id)) else {
                        bail!("Verdict references an unrelated or unknown test");
                    };
                    if test["candidate_id"] != verdict["candidate_id"] {
                        bail!("Verdict references an unrelated or unknown test");
                    }
                }
            }
        }
        _ => bail!("Unknown stage: {stage}"),
    }
    Ok(())
}
