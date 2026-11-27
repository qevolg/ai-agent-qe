use crate::state::{read_json, write_json};
use anyhow::Result;
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub(crate) fn denial(directory: &Path) -> Option<Value> {
    read_json(&directory.join("policy-denial.json")).ok()
}

pub(crate) fn record(run: &Path, source: &Path, reason: Value) -> Result<Value> {
    if let Some(saved) = denial(run) {
        return Ok(saved);
    }
    let value = json!({"code":"policy_denied","reason":reason,"source":source,
        "message":format!("安全策略拒绝，已终止本次审计：{}；日志：{}", reason, source.display())});
    let temporary = run.join(format!("policy-denial-{}.tmp", uuid::Uuid::new_v4()));
    write_json(&temporary, &value)?;
    let published = fs::hard_link(&temporary, run.join("policy-denial.json"));
    let _ = fs::remove_file(temporary);
    match published {
        Ok(()) => Ok(value),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            read_json(&run.join("policy-denial.json"))
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) struct Monitor {
    path: PathBuf,
    offset: u64,
    pending: Vec<u8>,
    terminal: bool,
}

impl Monitor {
    pub(crate) fn new(path: PathBuf, terminal: bool) -> Self {
        Self {
            path,
            offset: 0,
            pending: Vec::new(),
            terminal,
        }
    }

    pub(crate) fn check(&mut self) -> Result<Option<Value>> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        file.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = [0; 8192];
        loop {
            let count = file.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            self.offset += count as u64;
            self.pending.extend_from_slice(&bytes[..count]);
            if self.terminal {
                if let Some(reason) = terminal_reason(&String::from_utf8_lossy(&self.pending)) {
                    return Ok(Some(reason));
                }
                if self.pending.len() > 65536
                    && let Some(end) = self.pending[..self.pending.len() - 65536]
                        .iter()
                        .rposition(|byte| *byte == b'\n')
                {
                    self.pending.drain(..=end);
                }
            } else {
                while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
                    let reason = error_reason(&String::from_utf8_lossy(&self.pending[..end]));
                    self.pending.drain(..=end);
                    if reason.is_some() {
                        return Ok(reason);
                    }
                }
            }
        }
        Ok(if self.terminal {
            None
        } else {
            error_reason(&String::from_utf8_lossy(&self.pending))
        })
    }
}

fn terminal_reason(text: &str) -> Option<Value> {
    static ANSI: OnceLock<regex::Regex> = OnceLock::new();
    let clean = ANSI
        .get_or_init(|| regex::Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]").unwrap())
        .replace_all(text, "");
    let lines: Vec<_> = clean.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let Some(notice) = line
            .strip_prefix('■')
            .or_else(|| line.strip_prefix("ERROR:"))
        else {
            continue;
        };
        let mut notice = notice.to_owned();
        for continuation in lines[index + 1..].iter().take(8) {
            if !continuation.starts_with(' ') {
                break;
            }
            notice.push(' ');
            notice.push_str(continuation.trim());
        }
        let normalized = notice
            .replace('’', "'")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if normalized
            .to_lowercase()
            .starts_with("this content can't be shown")
            || policy_message(&normalized)
        {
            return Some(json!(notice.chars().take(4096).collect::<String>()));
        }
    }
    None
}

fn error_reason(text: &str) -> Option<Value> {
    if let Ok(event) = serde_json::from_str::<Value>(text) {
        let event = if event["type"] == "event_msg" {
            &event["payload"]
        } else {
            &event
        };
        if !matches!(event["type"].as_str(), Some("error" | "turn.failed")) {
            return None;
        }
        let error = event.get("error").unwrap_or(event);
        let message = error["message"]
            .as_str()
            .or_else(|| error.as_str())
            .unwrap_or("");
        let codes = [
            "cyber_policy",
            "bio_policy",
            "misalignment_policy_violation",
            "content_policy_violation",
            "content_filter",
            "policy_violation",
            "policy_denied",
        ];
        let code = ["codex_error_info", "code", "type"]
            .into_iter()
            .filter_map(|key| error[key].as_str())
            .find(|code| codes.contains(code));
        if code.is_some() || policy_message(message) {
            return Some(
                json!({"code":code.unwrap_or("policy_denied"),"message":message.chars().take(4096).collect::<String>()}),
            );
        }
        return None;
    }
    let text = text.trim();
    let diagnostic = text
        .strip_prefix("401:")
        .or_else(|| text.strip_prefix("ERROR:"))
        .or_else(|| text.strip_prefix("Error:"));
    if diagnostic.is_some_and(policy_message) {
        return Some(json!(text.chars().take(4096).collect::<String>()));
    }
    None
}

fn policy_message(text: &str) -> bool {
    let text = text.to_lowercase();
    text.contains("token disabled for content policy violation")
        || text.contains("blocked by cybersecurity policy")
        || text.contains("request rejected due to content policy")
        || text.contains("this request has been flagged for possible cybersecurity risk.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_detection_ignores_source_quotations_and_ordinary_failures() {
        for line in [
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"cyber_policy: blocked by cybersecurity policy"}}"#,
            r#"{"type":"turn.failed","error":{"message":"401 Unauthorized: invalid API key"}}"#,
            r#"{"type":"error","message":"429 rate limit exceeded"}"#,
            "Connection reset by peer",
            "source says: blocked by cybersecurity policy",
        ] {
            assert!(error_reason(line).is_none(), "{line}");
        }
        assert_eq!(error_reason(r#"{"type":"turn.failed","error":{"code":"content_policy_violation","message":"request denied"}}"#).unwrap()["code"], "content_policy_violation");
        for line in [
            r#"{"type":"error","message":"This request has been flagged for possible cybersecurity risk."}"#,
            r#"{"type":"turn.failed","error":{"message":"This request has been flagged for possible cybersecurity risk."}}"#,
        ] {
            assert!(error_reason(line).is_some(), "{line}");
        }
    }

    #[test]
    fn terminal_policy_notice_survives_ansi_and_line_wrapping() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("terminal.log");
        std::fs::write(
            &path,
            "\x1b[31m■ This content can’t\r\n be shown\x1b[0m\r\n cybersecurity safety policies",
        )
        .unwrap();
        assert!(Monitor::new(path, true).check().unwrap().is_some());
    }

    #[test]
    fn terminal_provider_policy_error_is_also_fatal() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("terminal.log");
        std::fs::write(&path, "■ 401: token disabled for content policy violation").unwrap();
        assert!(Monitor::new(path, true).check().unwrap().is_some());
    }

    #[test]
    fn terminal_source_and_prompt_quotations_are_not_errors() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("terminal.log");
        for text in [
            "› Explain why This content can't be shown by cybersecurity safety policies",
            "  └ source: blocked by cybersecurity policy",
            "• The log contains 401: token disabled for content policy violation",
            "• Source quotation: ■ This content can't be shown by cybersecurity safety policies",
            "• Called aaqe.status\n  └ source log:\n    ERROR: request rejected due to content policy",
            "• Called aaqe.status\n  └ source log:\n    ■ This content can't be shown by cybersecurity safety policies",
        ] {
            std::fs::write(&path, text).unwrap();
            assert!(
                Monitor::new(path.clone(), true).check().unwrap().is_none(),
                "{text}"
            );
        }
    }

    #[test]
    fn large_split_json_records_preserve_provenance_and_utf8() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("stdout.log");
        let mut output = File::create(&path).unwrap();
        let mut monitor = Monitor::new(path, false);
        write!(output, "{{\"type\":\"item.completed\",\"item\":{{\"type\":\"command_execution\",\"aggregated_output\":\"{}", "x".repeat(150_000)).unwrap();
        assert!(monitor.check().unwrap().is_none());
        writeln!(output, "blocked by cybersecurity policy\"}}}}").unwrap();
        assert!(monitor.check().unwrap().is_none());
        let event = json!({"type":"error","message":format!("{}This request has been flagged for possible cybersecurity risk.", "汉".repeat(30_000))}).to_string();
        let split = event.find('汉').unwrap() + 1;
        output.write_all(&event.as_bytes()[..split]).unwrap();
        assert!(monitor.check().unwrap().is_none());
        output.write_all(&event.as_bytes()[split..]).unwrap();
        let reason = monitor.check().unwrap().unwrap();
        assert!(!reason["message"].as_str().unwrap().contains('�'));
    }

    #[test]
    fn concurrent_denial_writers_preserve_one_complete_first_cause() {
        let temp = tempfile::tempdir().unwrap();
        let barrier = std::sync::Barrier::new(16);
        let values = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..16)
                .map(|index| {
                    let barrier = &barrier;
                    let directory = temp.path();
                    scope.spawn(move || {
                        barrier.wait();
                        record(directory, directory, json!(index)).unwrap()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        let saved = denial(temp.path()).unwrap();
        assert!(values.iter().all(|value| value == &saved));
    }
}
