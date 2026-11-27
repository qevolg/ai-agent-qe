//! Bounded, recorded inventory. Remote hosts are never deployment targets.
use crate::{
    process::{OUTPUT_TRUNCATED, run_env_bounded, save_result},
    state::{absolute_path, write_json},
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
use uuid::Uuid;

const TOOLS: &[(&str, &[&str])] = &[
    ("git", &["--version"]),
    ("docker", &["--version"]),
    ("java", &["-version"]),
    ("javac", &["-version"]),
    ("mvn", &["--version"]),
    ("gradle", &["--version"]),
    ("python", &["--version"]),
    ("python3", &["--version"]),
    ("pip", &["--version"]),
    ("pip3", &["--version"]),
    ("node", &["--version"]),
    ("npm", &["--version"]),
    ("cargo", &["--version"]),
    ("rustc", &["--version"]),
    ("go", &["version"]),
    ("cmake", &["--version"]),
    ("make", &["--version"]),
    ("gcc", &["--version"]),
    ("clang", &["--version"]),
    ("curl", &["--version"]),
    ("ssh", &["-V"]),
];
const INVENTORY: &str =
    "uname -s; docker version --format '{{json .}}'; docker ps --all --format '{{json .}}'";
const CAPTURE_LIMIT: usize = 65536;

pub fn schema() -> Value {
    let tool_names: Vec<_> = TOOLS.iter().map(|(name, _)| *name).collect();
    json!({"type":"object","properties":{
        "kind":{"type":"string","enum":["tools","docker","endpoint","ssh_inventory"]},
        "target":{"type":"string","enum":["runner","host"]},
        "tools":{"type":"array","minItems":1,"maxItems":16,"items":{"type":"string","enum":tool_names}},
        "url":{"type":"string","minLength":1,"maxLength":4096},
        "host":{"type":"string","minLength":1,"maxLength":253},
        "user":{"type":"string","minLength":1,"maxLength":64},
        "port":{"type":"integer","minimum":1,"maximum":65535},
        "identity_file":{"type":"string","minLength":1},
        "password_env":{"type":"string","pattern":"^[A-Za-z_][A-Za-z0-9_]*$"}
    },"required":["kind"],"additionalProperties":false})
}

fn required<'a>(probe: &'a Value, field: &str) -> Result<&'a str> {
    probe[field]
        .as_str()
        .filter(|value| !value.is_empty())
        .with_context(|| format!("Probe requires {field}"))
}

fn token(value: &str, extra: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || extra.contains(ch))
}

/// Evidence includes unsuccessful process results; malformed requests never execute.
pub fn inspect(workspace: &Path, directory: &Path, probe: &Value, timeout: u64) -> Result<Value> {
    ensure!(
        probe["target"] != "runner",
        "Prepare a runner before probing test dependencies"
    );
    inspect_scoped(workspace, directory, probe, timeout, None)
}

pub fn inspect_runner(
    workspace: &Path,
    directory: &Path,
    probe: &Value,
    timeout: u64,
    prefix: &[String],
    service: &str,
) -> Result<Value> {
    ensure!(
        probe["target"] != "host",
        "Host probes cannot execute in a runner"
    );
    ensure!(
        matches!(probe["kind"].as_str(), Some("tools" | "endpoint")),
        "Only tools and endpoint probes execute in a runner"
    );
    inspect_scoped(
        workspace,
        directory,
        probe,
        timeout,
        Some((prefix, service)),
    )
}

fn inspect_scoped(
    workspace: &Path,
    directory: &Path,
    probe: &Value,
    timeout: u64,
    runner: Option<(&[String], &str)>,
) -> Result<Value> {
    ensure!(timeout > 0, "Probe timeout must be positive");
    let timeout = timeout.min(60);
    jsonschema::validator_for(&schema())?
        .validate(probe)
        .map_err(|_| anyhow::anyhow!("Invalid environment probe"))?;
    let kind = required(probe, "kind")?;
    let allowed: &[&str] = match kind {
        "tools" => &["kind", "target", "tools"],
        "docker" => &["kind", "target"],
        "endpoint" => &["kind", "target", "url"],
        "ssh_inventory" => &[
            "kind",
            "target",
            "host",
            "user",
            "port",
            "identity_file",
            "password_env",
        ],
        _ => bail!("Unknown environment probe kind"),
    };
    ensure!(
        probe
            .as_object()
            .context("Probe must be an object")?
            .keys()
            .all(|key| allowed.contains(&key.as_str())),
        "Unexpected fields for environment probe kind"
    );
    let mut commands: Vec<Vec<String>> = Vec::new();
    let mut env = Vec::new();
    let mut password = None;
    match kind {
        "tools" => {
            let tools = probe["tools"]
                .as_array()
                .context("Tools probe requires tool names")?;
            ensure!(
                !tools.is_empty() && tools.len() <= 16,
                "Tools probe requires 1 to 16 tool names"
            );
            for tool in tools {
                let tool = tool.as_str().context("Invalid tool name")?;
                let (_, arguments) = TOOLS
                    .iter()
                    .find(|(name, _)| *name == tool)
                    .context("Unsupported inventory tool")?;
                let mut argv = vec![tool.into()];
                argv.extend(arguments.iter().map(|argument| (*argument).into()));
                commands.push(argv);
            }
        }
        "docker" => {
            for argv in [
                vec!["docker", "version", "--format", "{{json .}}"],
                vec!["docker", "info", "--format", "{{json .}}"],
                vec!["docker", "compose", "version"],
                vec!["docker", "ps", "--all", "--format", "{{json .}}"],
            ] {
                commands.push(argv.into_iter().map(str::to_owned).collect());
            }
        }
        "endpoint" => {
            let url = required(probe, "url")?;
            let authority = url
                .strip_prefix("https://")
                .or_else(|| url.strip_prefix("http://"))
                .context("Endpoint must use HTTP or HTTPS")?
                .split(['/', '?', '#'])
                .next()
                .unwrap_or_default();
            ensure!(
                !authority.is_empty()
                    && !authority.contains('@')
                    && !url.chars().any(|ch| ch.is_control() || ch.is_whitespace()),
                "Invalid endpoint URL or embedded credentials"
            );
            commands.push(vec![
                "curl".into(),
                "--disable".into(),
                "--silent".into(),
                "--show-error".into(),
                "--fail".into(),
                "--head".into(),
                "--proto".into(),
                "=http,https".into(),
                "--connect-timeout".into(),
                timeout.min(10).to_string(),
                "--max-time".into(),
                timeout.to_string(),
                "--max-filesize".into(),
                "65536".into(),
                "--".into(),
                url.into(),
            ]);
        }
        "ssh_inventory" => {
            let host = required(probe, "host")?;
            ensure!(
                host.len() <= 253 && token(host, ".:-_[]"),
                "Invalid SSH inventory host"
            );
            let mut argv = vec![
                "ssh".into(),
                "-F".into(),
                "/dev/null".into(),
                "-T".into(),
                "-o".into(),
                "StrictHostKeyChecking=yes".into(),
                "-o".into(),
                "ClearAllForwardings=yes".into(),
                "-o".into(),
                "PermitLocalCommand=no".into(),
                "-o".into(),
                format!("ConnectTimeout={}", timeout.min(10)),
                "-o".into(),
                "ConnectionAttempts=1".into(),
            ];
            if let Some(user) = probe.get("user") {
                let user = user.as_str().context("Invalid SSH inventory user")?;
                ensure!(
                    user.len() <= 64 && token(user, "_.-"),
                    "Invalid SSH inventory user"
                );
                argv.extend(["-l".into(), user.into()]);
            }
            if let Some(port) = probe.get("port") {
                let port = crate::config::unsigned_integer(port).context("Invalid SSH port")?;
                ensure!((1..=65535).contains(&port), "Invalid SSH port");
                argv.extend(["-p".into(), port.to_string()]);
            }
            ensure!(
                probe.get("identity_file").is_none() || probe.get("password_env").is_none(),
                "Choose SSH key or password authentication"
            );
            if let Some(key) = probe["identity_file"].as_str() {
                let path = absolute_path(&workspace.join(key));
                ensure!(path.is_file(), "SSH identity file is missing");
                argv.extend([
                    "-i".into(),
                    path.to_string_lossy().into_owned(),
                    "-o".into(),
                    "IdentitiesOnly=yes".into(),
                ]);
            }
            if let Some(name) = probe["password_env"].as_str() {
                let value = std::env::var(name)
                    .ok()
                    .filter(|value| !value.is_empty())
                    .context("SSH password environment variable is missing or empty")?;
                password = Some(value);
                argv.extend([
                    "-o".into(),
                    "BatchMode=no".into(),
                    "-o".into(),
                    "PreferredAuthentications=password,keyboard-interactive".into(),
                    "-o".into(),
                    "NumberOfPasswordPrompts=1".into(),
                ]);
            } else {
                argv.extend(["-o".into(), "BatchMode=yes".into()]);
            }
            argv.extend([host.into(), INVENTORY.into()]);
            commands.push(argv);
        }
        _ => unreachable!(),
    }
    let directory = absolute_path(directory).join(format!("probe-{}", Uuid::new_v4().simple()));
    ensure!(
        !directory.starts_with(absolute_path(workspace)),
        "Probe evidence must be outside the worktree"
    );
    fs::create_dir_all(&directory)?;
    let askpass = directory.join("ssh-askpass");
    if let Some(password) = &password {
        fs::write(
            &askpass,
            "#!/bin/sh\nprintf '%s\\n' \"$AAQE_SSH_PASSWORD\"\n",
        )?;
        fs::set_permissions(&askpass, fs::Permissions::from_mode(0o700))?;
        env.extend([
            ("SSH_ASKPASS".into(), askpass.to_string_lossy().into_owned()),
            ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
            ("DISPLAY".into(), "aaqe:0".into()),
            ("AAQE_SSH_PASSWORD".into(), password.clone()),
        ]);
    }
    let started_at = std::time::Instant::now();
    let mut results = Vec::new();
    let mut budget_exhausted = false;
    for (index, argv) in commands.iter().enumerate() {
        let elapsed = started_at.elapsed().as_secs();
        if elapsed >= timeout {
            budget_exhausted = true;
            break;
        }
        let remaining = timeout - elapsed;
        let mut wrapped = Vec::new();
        if let Some((prefix, _)) = runner {
            wrapped.extend_from_slice(prefix);
        }
        wrapped.extend_from_slice(argv);
        let mut result = run_env_bounded(
            &wrapped,
            Some(workspace),
            remaining,
            None,
            &env,
            CAPTURE_LIMIT,
        );
        if let Some(password) = &password {
            for text in &mut result.argv {
                *text = text.replace(password, "[REDACTED]");
            }
            for output in [&mut result.stdout, &mut result.stderr] {
                let truncated = output.ends_with(OUTPUT_TRUNCATED);
                let mut body = output
                    .strip_suffix(OUTPUT_TRUNCATED)
                    .unwrap_or(output)
                    .replace(password, "[REDACTED]");
                if truncated {
                    // A password can straddle the capture boundary. Full matches
                    // were redacted above; discard the remaining possible prefix
                    // plus a possible UTF-8 replacement character at the cutoff.
                    let mut end = body.len().saturating_sub(password.len().saturating_add(3));
                    while !body.is_char_boundary(end) {
                        end -= 1;
                    }
                    body.truncate(end);
                }
                let expanded = body.len() > CAPTURE_LIMIT;
                if expanded {
                    let mut end = CAPTURE_LIMIT;
                    while !body.is_char_boundary(end) {
                        end -= 1;
                    }
                    body.truncate(end);
                }
                if truncated || expanded {
                    body.push_str(&OUTPUT_TRUNCATED.replace(password, "[REDACTED]"));
                }
                *output = body;
            }
        }
        save_result(&result, &directory.join(format!("process-{index}")))?;
        results.push(result);
    }
    if askpass.exists() {
        fs::remove_file(askpass)?;
    }
    let mut evidence = json!({"probe":probe,"target":if runner.is_some() { "runner" } else { "host" },"runner_service":runner.map(|(_, service)| service),"directory":directory,"capture_limit_bytes":CAPTURE_LIMIT,"budget_exhausted":budget_exhausted,"success":!budget_exhausted && results.iter().all(|result|result.returncode == 0),"results":results});
    // A caller can accidentally reuse a password as another field. Redact the entire
    // evidence document as well as process streams before it reaches disk.
    if let Some(password) = password {
        fn redact(value: &mut Value, password: &str) {
            match value {
                Value::String(text) => *text = text.replace(password, "[REDACTED]"),
                Value::Array(values) => {
                    for value in values {
                        redact(value, password)
                    }
                }
                Value::Object(values) => {
                    for value in values.values_mut() {
                        redact(value, password)
                    }
                }
                _ => {}
            }
        }
        redact(&mut evidence, &password);
    }
    write_json(&directory.join("probe.json"), &evidence)?;
    Ok(evidence)
}
