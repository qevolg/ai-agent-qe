use super::AuditHarness;
use crate::agents::stage::STAGES;
use serde_json::{Value, json};

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
