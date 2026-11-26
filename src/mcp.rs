//! Private stdio MCP transport for one audit's coordinator. No stdout diagnostics.
use crate::{
    codex::Codex, config::Config, environment::ComposeEnvironment, harness::AuditHarness,
    state::read_json,
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
}

pub fn tools() -> Value {
    json!([
        tool(
            "status",
            "Read current accepted artifacts, execution, verdicts, attempts and budgets.",
            json!({}),
            &[]
        ),
        tool(
            "run_stage",
            "Start a fresh independent background Codex stage process, wait for its result and actual execution. Failures are feedback. Repair defective tests/fixtures only; retain product failures as evidence.",
            json!({"stage":{"type":"string","enum":crate::workflow::STAGES},"instruction":{"type":"string","minLength":1}}),
            &["stage", "instruction"]
        ),
        tool(
            "inspect_environment",
            "Inspect bounded read-only environment capabilities and retain evidence.",
            json!({"probe":crate::environment_probe::schema()}),
            &["probe"]
        ),
        tool(
            "prepare_environment",
            "Prepare or retry this audit's immutable runner before generating tests.",
            json!({"environment":{"type":["object","null"]}}),
            &[]
        ),
        tool(
            "execute_tests",
            "Execute accepted tests again without another Developer call; optionally select unique test IDs.",
            json!({"test_ids":{"type":["array","null"],"items":{"type":"string","minLength":1},"minItems":1,"uniqueItems":true}}),
            &[]
        ),
        tool(
            "finish",
            "Finish explicitly with a reason. Completed requires current analysis and independent validation for candidates. Blocked preserves partial evidence.",
            json!({"status":{"type":"string","enum":["completed","blocked"]},"reason":{"type":"string","minLength":1}}),
            &["status", "reason"]
        )
    ])
}

fn call(harness: &mut AuditHarness<'_>, params: &Value) -> Result<Value> {
    let name = params["name"].as_str().context("Missing tool name")?;
    let definition = tools()
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .cloned()
        .context("Unknown harness tool")?;
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    let validator = jsonschema::validator_for(&definition["inputSchema"])?;
    validator
        .validate(&arguments)
        .map_err(|e| anyhow::anyhow!("Invalid tool arguments: {e}"))?;
    match name {
        "status" => Ok(harness.status()),
        "run_stage" => harness.run_stage(
            arguments["stage"].as_str().unwrap(),
            arguments["instruction"].as_str().unwrap(),
        ),
        "inspect_environment" => harness.inspect_environment(&arguments["probe"]),
        "prepare_environment" => harness.prepare_environment(
            arguments
                .get("environment")
                .filter(|v| !v.is_null())
                .cloned(),
        ),
        "execute_tests" => harness.execute_tests(
            arguments
                .get("test_ids")
                .filter(|v| !v.is_null())
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()?,
        ),
        "finish" => {
            harness.finish(
                arguments["status"].as_str().unwrap(),
                arguments["reason"].as_str().unwrap(),
            )?;
            Ok(harness.status())
        }
        _ => bail!("Unknown harness tool"),
    }
}

struct ServerGuard {
    _lock: crate::process::WorkerLock,
    stopped: Arc<AtomicBool>,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

pub fn serve(directory: &Path) -> Result<()> {
    let saved = read_json(&directory.join("harness.json"))?;
    let config: Config = serde_json::from_value(saved["state"]["config"].clone())?;
    let agent = Codex::new(&config);
    let factory = |workspace: &Path,
                   directory: &Path,
                   id: &str,
                   spec: Value,
                   timeout: u64|
     -> Box<dyn crate::workflow::TestRuntime> {
        Box::new(ComposeEnvironment::new(
            workspace, directory, id, spec, timeout,
        ))
    };
    let progress = |message: &str| eprintln!("{message}");
    let mut harness = AuditHarness::open(directory, &agent, &factory, &progress)?;
    let coordinator = directory.join("stages/coordinator");
    fs::create_dir_all(&coordinator)?;
    let marker = coordinator.join("mcp-running");
    let stopped = Arc::new(AtomicBool::new(false));
    let _guard = ServerGuard {
        _lock: crate::process::WorkerLock::new(&marker)?,
        stopped: stopped.clone(),
    };
    let cancel = coordinator.join("cancel");
    let watch_cancel = saved["state"]["status"] == "running";
    let parent = unsafe { libc::getppid() };
    thread::spawn(move || {
        while !stopped.load(Ordering::SeqCst) {
            if (watch_cancel && cancel.exists()) || unsafe { libc::getppid() } != parent {
                crate::process::request_interrupt();
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
    });
    let mut stdout = std::io::stdout().lock();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    loop {
        if crate::process::interrupted() {
            break;
        }
        let line = match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(line) => line?,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                writeln!(
                    stdout,
                    "{}",
                    json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":error.to_string()}})
                )?;
                stdout.flush()?;
                continue;
            }
        };
        let Some(id) = request.get("id") else {
            continue;
        };
        let method = request["method"].as_str().unwrap_or("");
        let response = match method {
            "initialize" => {
                let requested = request["params"]["protocolVersion"].as_str().unwrap_or("");
                let version = if ["2024-11-05", "2025-03-26", "2025-06-18"].contains(&requested) {
                    requested
                } else {
                    "2025-06-18"
                };
                json!({"result":{"protocolVersion":version,"capabilities":{"tools":{}},"serverInfo":{"name":"aaqe-harness","version":env!("CARGO_PKG_VERSION")}}})
            }
            "ping" => json!({"result":{}}),
            "tools/list" => json!({"result":{"tools":tools()}}),
            "tools/call" => {
                harness.reload()?;
                let result = call(&mut harness, &request["params"]);
                let (value, error) = match result {
                    Ok(value) => {
                        let failed = value["status"] == "failed";
                        (value, failed)
                    }
                    Err(error) => (
                        json!({"status":"failed","gate":"tool_dispatch","error":format!("{error:#}"),"recovery":harness.recovery()}),
                        true,
                    ),
                };
                json!({"result":{"content":[{"type":"text","text":serde_json::to_string(&value)?}],"structuredContent":value,"isError":error}})
            }
            _ => json!({"error":{"code":-32601,"message":"Method not found"}}),
        };
        if method == "tools/call" {
            let log = directory.join("stages/coordinator/tools.jsonl");
            std::fs::create_dir_all(log.parent().unwrap())?;
            writeln!(
                OpenOptions::new().create(true).append(true).open(log)?,
                "{}",
                json!({"at":crate::state::now(),"request":request,"response":response})
            )?;
        }
        let mut reply = json!({"jsonrpc":"2.0","id":id});
        reply
            .as_object_mut()
            .unwrap()
            .extend(response.as_object().unwrap().clone());
        writeln!(stdout, "{reply}")?;
        stdout.flush()?;
        if method == "tools/call"
            && request["params"]["name"] == "finish"
            && reply["result"]["isError"] == false
        {
            File::create(coordinator.join("audit-finished"))?;
        }
    }
    Ok(())
}
