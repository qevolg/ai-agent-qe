use super::AuditHarness;
use crate::agents::stage::STAGES;
use serde_json::{Value, json};

pub(crate) fn artifact_summary(value: &Value) -> Value {
    let mut view = json!({"summary":value["summary"].as_str().map(|text|excerpt(text,2048))});
    if value["real_services"].is_array() {
        view["real_services"] = value["real_services"].clone();
    }
    for key in ["references", "candidates", "plans", "tests", "verdicts"] {
        if let Some(items) = value[key].as_array() {
            let preview: Vec<_> = items
                .iter()
                .take(20)
                .map(|item| {
                    let mut entry = json!({});
                    for field in [
                        "id",
                        "candidate_id",
                        "test_id",
                        "test_ids",
                        "title",
                        "status",
                        "confidence",
                        "blocked_reason",
                        "execution_blocked_reason",
                        "real_services",
                    ] {
                        if let Some(value) = item.get(field) {
                            entry[field] = value.clone();
                        }
                    }
                    entry
                })
                .collect();
            view[key] =
                json!({"total":items.len(),"items":preview,"has_more":items.len()>preview.len()});
        }
    }
    view
}

pub(crate) fn execution_summary(value: &Value) -> Value {
    let mut view = json!({"coverage":value["coverage"]});
    for key in ["baseline", "tests"] {
        let items = value[key].as_array().cloned().unwrap_or_default();
        let preview: Vec<_> = items.iter().take(20).map(|item| {
            json!({"test_id":item["test_id"],"directory":item["directory"],
                "result":{"returncode":item["result"]["returncode"],"timed_out":item["result"]["timed_out"],"stderr":item["result"]["stderr"].as_str().map(|text|excerpt(text,1024))},
                "evidence":item["evidence"],"error":item["error"],
                "target_executed":item["target_executed"],"required_services":item["required_services"],
                "service_checks":item["service_checks"].as_array().map(|checks| checks.iter().map(|check| json!({"service":check["service"],"directory":check["directory"],"result":{"returncode":check["result"]["returncode"],"timed_out":check["result"]["timed_out"]}})).collect::<Vec<_>>())})
        }).collect();
        view[key] =
            json!({"total":items.len(),"items":preview,"has_more":items.len()>preview.len()});
    }
    view
}

pub(crate) fn tool_view(value: &Value) -> Value {
    let mut view = evidence_view(value);
    if let Some(artifact) = value.get("artifact") {
        view["artifact"] = artifact_summary(artifact);
    }
    if let Some(execution) = value.get("execution") {
        view["execution"] = execution_summary(execution);
    }
    if let Some(verdicts) = value.get("verdicts") {
        view["verdicts"] = artifact_summary(&json!({"verdicts":verdicts}))["verdicts"].clone();
    }
    view
}

/// Log summaries in model payloads; complete logs remain at the evidence directory.
pub(crate) fn evidence_view(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    let value = if matches!(key.as_str(), "stdout" | "stderr") {
                        value
                            .as_str()
                            .map(|text| Value::String(excerpt(text, 4096)))
                            .unwrap_or_else(|| value.clone())
                    } else {
                        evidence_view(value)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        Value::Array(array) => Value::Array(array.iter().map(evidence_view).collect()),
        _ => value.clone(),
    }
}

fn excerpt(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut head = limit / 2;
    let mut tail = text.len() - limit / 2;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    format!(
        "{}\n[excerpt; full output in evidence directory]\n{}",
        &text[..head],
        &text[tail..]
    )
}

impl AuditHarness<'_> {
    pub fn model_status(&self, section: Option<&str>, stage: Option<&str>) -> Value {
        let full = self.status();
        if let Some(section) = section {
            let value = match section {
                "artifacts" => stage
                    .map(|role| json!({role:self.artifacts[role]}))
                    .unwrap_or_else(|| self.artifacts.clone()),
                "history" => self.state["stages"].clone(),
                "environment" => {
                    json!({"environment":self.state["environment"],"setup":self.state["environment_setup"],"ready":self.state["environment_ready"],"actions":self.state["environment_actions"]})
                }
                _ => full[section].clone(),
            };
            return evidence_view(&json!({"section":section,"stage":stage,"data":value}));
        }
        let mut state = json!({});
        for key in [
            "id",
            "prompt",
            "status",
            "stage",
            "finished_at",
            "error",
            "environment_ready",
        ] {
            state[key] = self.state[key].clone();
        }
        let artifacts: serde_json::Map<String, Value> = self
            .artifacts
            .as_object()
            .into_iter()
            .flatten()
            .map(|(role, value)| {
                let mut receipt = artifact_summary(value);
                receipt["path"] = json!(
                    self.directory
                        .join("artifacts")
                        .join(format!("{role}.json"))
                );
                (role.clone(), receipt)
            })
            .collect();
        json!({"state":state,"workspace":full["workspace"],"artifacts":artifacts,
            "execution":execution_summary(&self.execution),"verdicts":artifact_summary(&json!({"verdicts":self.verdicts}))["verdicts"],
            "allowed_stages":full["allowed_stages"],"stage_availability":full["stage_availability"],"recovery":full["recovery"],"budgets":full["budgets"],
            "attempt_count":self.state["stages"].as_array().map(Vec::len),"directory":self.directory})
    }

    pub(super) fn build_stage_inputs(&self, stage: &str, attempt: u64, instruction: &str) -> Value {
        let index = STAGES.iter().position(|role| *role == stage).unwrap();
        let mut artifacts = json!({});
        for role in &STAGES[..index] {
            if let Some(artifact) = self.artifacts.get(*role) {
                artifacts[*role] = artifact.clone();
            }
        }
        if stage == "test-developer"
            && let Some(previous) = self.artifacts.get(stage)
        {
            artifacts[stage] = previous.clone();
        }
        let artifact_index: Vec<_> = self.artifacts.as_object().into_iter().flatten().map(|(role, artifact)| {
            json!({"stage":role,"summary":artifact["summary"],"path":self.directory.join("artifacts").join(format!("{role}.json"))})
        }).collect();
        let history: Vec<_> = self.state["stages"].as_array().into_iter().flatten()
            .filter(|entry| entry["status"] != "running")
            .map(|entry| json!({"name":entry["name"],"attempt":entry["attempt"],"instruction":entry["instruction"],"status":entry["status"],"error":entry["error"],"directory":entry["directory"]})).collect();
        let actions: Vec<_> = self.state["environment_actions"].as_array().into_iter().flatten().rev().take(8).map(|action| {
            json!({"operation":action["operation"],"at":action["at"],"status":action["status"],"error":action["error"],"success":action["result"]["success"],"evidence_directory":action["result"]["evidence_directory"]})
        }).collect::<Vec<_>>().into_iter().rev().collect();
        let execution = if index >= 2 {
            evidence_view(&self.execution)
        } else {
            json!({"baseline":[],"tests":[],"path":self.directory.join("execution.json")})
        };
        json!({"stage":stage,"attempt":attempt,"instruction":instruction,"prompt":self.state["prompt"],"base_commit":self.state["workspace"]["commit"],"project_config":self.config,
            "artifacts":artifacts,"artifact_index":artifact_index,"execution":execution,"previous_attempts":history,
            "environment":self.state["environment"],"environment_setup":self.state["environment_setup"],"environment_ready":self.state["environment_ready"],"environment_actions":actions,"artifact_directory":self.directory})
    }
}
