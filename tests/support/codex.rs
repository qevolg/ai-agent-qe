//! Deterministic Codex boundary; actual AAQE, Git, tmux, MCP and Docker remain real.
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::{self, BufRead, BufReader, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

struct Client {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    sequence: u64,
}
impl Client {
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.sequence += 1;
        writeln!(
            self.input,
            "{}",
            json!({"jsonrpc":"2.0","id":self.sequence,"method":method,"params":params})
        )
        .unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        let reply: Value =
            serde_json::from_str(&line).unwrap_or_else(|e| panic!("MCP response {line:?}: {e}"));
        assert!(reply.get("error").is_none(), "{reply}");
        reply["result"].clone()
    }

    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name":name,"arguments":arguments}))
    }
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn argument<'a>(args: &'a [String], name: &str) -> &'a str {
    &args.windows(2).find(|a| a[0] == name).unwrap()[1]
}

fn load(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn save(path: &Path, value: &Value) {
    fs::write(path, value.to_string()).unwrap();
}

fn coordinate(args: &[String], directory: &Path, settings: BTreeMap<&str, &str>) {
    assert!(!flag(args, "exec") && !flag(args, "--json") && !flag(args, "--output-schema"));
    assert!(args.last().unwrap().contains("Initial audit status:"));
    save(
        &directory.join("terminal.json"),
        &json!({"stdin":io::stdin().is_terminal(),"stdout":io::stdout().is_terminal(),"stderr":io::stderr().is_terminal()}),
    );
    fs::write(directory.join("tui.pid"), std::process::id().to_string()).unwrap();
    if flag(args, "--policy-coordinator") || flag(args, "--policy-fast-coordinator") {
        println!(
            "\x1b[31m■ This content can’t be shown\x1b[0m because it violates cybersecurity safety policies."
        );
        io::stdout().flush().unwrap();
        if flag(args, "--policy-fast-coordinator") {
            return;
        }
        thread::sleep(Duration::from_secs(30));
        fs::write(directory.join("continued-after-policy"), "").unwrap();
        return;
    }
    assert_eq!(
        settings.get("mcp_servers.aaqe.default_tools_approval_mode"),
        Some(&"\"approve\"")
    );
    let executable: String = serde_json::from_str(settings["mcp_servers.aaqe.command"]).unwrap();
    let arguments: Vec<String> = serde_json::from_str(settings["mcp_servers.aaqe.args"]).unwrap();
    let mut child = Command::new(executable)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let terminated = Arc::new(AtomicBool::new(false));
    if flag(args, "--kill-mcp-test") {
        signal_hook::flag::register(libc::SIGTERM, terminated.clone()).unwrap();
    }
    let kill_pid = child.id();
    let kill_flag = terminated.clone();
    if flag(args, "--kill-mcp-test") {
        thread::spawn(move || {
            loop {
                if kill_flag.load(Ordering::Relaxed) {
                    unsafe {
                        libc::kill(kill_pid as i32, libc::SIGKILL);
                    }
                    std::process::exit(0);
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
    }
    let mut client = Client {
        input: child.stdin.take().unwrap(),
        output: BufReader::new(child.stdout.take().unwrap()),
        child,
        sequence: 0,
    };
    client.request("initialize",json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"scripted-codex","version":"1"}}));
    let names = client.request("tools/list", json!({}));
    let names: BTreeSet<_> = names["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from([
            "status",
            "run_stage",
            "inspect_environment",
            "prepare_environment",
            "execute_tests",
            "finish",
            "resume"
        ])
    );
    assert_eq!(
        client.tool(
            "run_stage",
            json!({"stage":"test-developer","instruction":"too early"})
        )["isError"],
        true
    );
    assert_eq!(
        client.tool("finish", json!({"status":"completed","reason":"too early"}))["isError"],
        true
    );
    if flag(args, "--early-inline") {
        let inspected = client.tool("inspect_environment", json!({"probe":{"kind":"docker"}}));
        assert_eq!(inspected["isError"], false, "{inspected}");
        let prepared=client.tool("prepare_environment",json!({"environment":{
            "compose":{"services":{"runner":{"image":super::project::image(),"command":["sleep","infinity"],"init":true}}},
            "services":["runner"],"runner_service":"runner","runner_workdir":"/workspace","external_containers":[],"readiness_timeout":90,"setup":[["rustup","component","add","llvm-tools-preview"]],
            "readiness":[["rustc","--edition=2024","/workspace/run_tests.rs","-o","/aaqe-results/project-runner"],["/aaqe-results/project-runner","ready"]]
        }}));
        assert_eq!(prepared["isError"], true, "{prepared}");
        let inspected = client.tool(
            "inspect_environment",
            json!({"probe":{"kind":"tools","tools":["rustc"]}}),
        );
        assert_eq!(inspected["structuredContent"]["result"]["target"], "runner");
        assert_eq!(
            inspected["structuredContent"]["result"]["success"], true,
            "{inspected}"
        );
        let prepared=client.tool("prepare_environment",json!({"setup":[["rustc","--edition=2024","--crate-name","aaqe_test_dependency","--crate-type","rlib","/workspace/dependency.rs","--out-dir","/aaqe-results"]]}));
        assert_eq!(prepared["isError"], false, "{prepared}");
        assert_eq!(
            prepared["structuredContent"]["result"]["environment_ready"],
            true
        );
    }
    let mut stages = vec![
        "code-explorer",
        "bug-hunter",
        "test-designer",
        "test-developer",
        "test-validator",
    ];
    if flag(args, "--retry-developer") {
        stages.insert(4, "test-developer");
    }
    if flag(args, "--loop-test") || flag(args, "--cancel-test") {
        stages = vec!["code-explorer", "code-explorer", "bug-hunter"];
    }
    let mut failed = false;
    for stage in stages {
        if stage == "test-validator" && flag(args, "--early-inline") {
            let rerun = client.tool("execute_tests", json!({"test_ids":["negative-input"]}));
            assert_eq!(rerun["isError"], false, "{rerun}");
            assert_eq!(
                client.tool("status", json!({"section":"execution"}))["structuredContent"]["data"]
                    ["tests"][0]["evidence"]["complete"],
                true
            );
        }
        let result=client.tool("run_stage",json!({"stage":stage,"instruction":"Inspect current evidence and perform this role independently"}));
        if (flag(args, "--policy-stage")
            || flag(args, "--policy-provider")
            || flag(args, "--policy-message-stage"))
            && stage == "bug-hunter"
        {
            thread::sleep(Duration::from_secs(30));
            fs::write(directory.join("continued-after-policy"), "").unwrap();
        }
        if stage == "code-explorer" && result["isError"] == false {
            let status = client.tool("status", json!({}));
            assert_eq!(
                status["structuredContent"]["artifacts"][stage]["references"]["total"],
                1
            );
            assert!(status["structuredContent"]["state"].get("stages").is_none());
            let detailed = client.tool("status", json!({"section":"artifacts","stage":stage}));
            assert_eq!(
                detailed["structuredContent"]["data"][stage]["references"][0]["id"],
                "source-1"
            );
        }
        if result["isError"] == true {
            client.tool(
                "finish",
                json!({"status":"blocked","reason":result["content"][0]["text"]}),
            );
            failed = true;
            break;
        }
    }
    if !failed {
        let result=client.tool("finish",json!({"status":if flag(args,"--resume-test") {"blocked"} else {"completed"},"reason":"All current candidates were checked independently"}));
        assert_eq!(result["isError"], false, "{result}");
    }
    println!("Audit finished via harness tools; ready for follow-up");
    io::stdout().flush().unwrap();
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        if line.contains("继续") {
            let status = client.tool("status", json!({}));
            if status["structuredContent"]["state"]["status"] != "running" {
                assert_eq!(
                    client.tool("resume", json!({"from_stage":"bug-hunter"}))["isError"],
                    false
                );
            }
            assert_eq!(
                client.tool(
                    "run_stage",
                    json!({"stage":"bug-hunter","instruction":"resume independent stage"})
                )["isError"],
                false
            );
            assert_eq!(
                client.tool(
                    "finish",
                    json!({"status":"completed","reason":"Resumed in original conversation"})
                )["isError"],
                false
            );
            fs::write(directory.join("resumed.txt"), &line).unwrap();
            continue;
        }
        let status = client.tool("status", json!({}));
        assert!(status["structuredContent"]["state"]["finished_at"].is_string());
        assert_eq!(
            client.tool(
                "run_stage",
                json!({"stage":"code-explorer","instruction":"after audit"})
            )["isError"],
            true
        );
        fs::write(directory.join("follow-up.txt"), format!("{line}\n")).unwrap();
        println!("Follow-up received");
        io::stdout().flush().unwrap();
    }
    drop(client.input);
    assert!(client.child.wait().unwrap().success());
}

pub fn run(args: &[String]) {
    if !flag(args, "--output-schema") {
        let settings: BTreeMap<_, _> = args
            .windows(2)
            .filter(|a| a[0] == "-c")
            .filter_map(|a| a[1].split_once('='))
            .collect();
        let paths: Vec<String> = serde_json::from_str(settings["mcp_servers.aaqe.args"]).unwrap();
        coordinate(
            args,
            &PathBuf::from(&paths[1]).join("stages/coordinator"),
            settings,
        );
        return;
    }
    let directory = Path::new(argument(args, "--output-schema"))
        .parent()
        .unwrap();
    let inputs = load(&directory.join("input.json"));
    let mut prompt = String::new();
    io::stdin().read_to_string(&mut prompt).unwrap();
    let stage = inputs["stage"]
        .as_str()
        .unwrap_or_else(|| directory.file_name().unwrap().to_str().unwrap());
    if (flag(args, "--policy-stage")
        || flag(args, "--policy-provider")
        || flag(args, "--policy-message-stage"))
        && stage == "bug-hunter"
    {
        save(
            Path::new(argument(args, "--output-last-message")),
            &json!({"summary":"must not accept","candidates":[],"excluded_areas":[]}),
        );
        if flag(args, "--policy-provider") {
            eprintln!("401: token disabled for content policy violation");
        } else if flag(args, "--policy-message-stage") {
            println!(
                "{}",
                json!({"type":"turn.failed","error":{"message":"This request has been flagged for possible cybersecurity risk."}})
            );
        } else {
            print!("{{\"type\":\"error\",\"codex_error_info\":\"cyber_");
            io::stdout().flush().unwrap();
            thread::sleep(Duration::from_millis(40));
            println!("policy\",\"message\":\"blocked by cybersecurity policy\"}}");
        }
        io::stdout().flush().unwrap();
        thread::sleep(Duration::from_secs(30));
        fs::write(directory.join("continued-after-policy"), "").unwrap();
        return;
    }
    if flag(args, "--cancel-test") {
        let mut child = Command::new(env::current_exe().unwrap())
            .arg("idle")
            .spawn()
            .unwrap();
        fs::write(directory.join("child.pid"), child.id().to_string()).unwrap();
        thread::spawn(move || child.wait().unwrap());
        fs::write(directory.join("worker.pid"), std::process::id().to_string()).unwrap();
        thread::sleep(Duration::from_secs(60));
    }
    let mut output = match stage {
        "code-explorer" => {
            json!({"summary":"Rust scope examined","scope":["app.rs"],"references":[{"id":"source-1","path":"app.rs","line":1,"claim":"Implementation clamps only the upper bound; README specifies zero to ten"}],"uncertainties":[],"environment":{"compose_files":["compose.yaml"],"services":["runner"],"runner_service":"runner","runner_workdir":"/workspace","external_containers":[],"readiness_timeout":90,"readiness":[["rustc","--edition=2024","/workspace/run_tests.rs","-o","/aaqe-results/project-runner"]]},"baseline":[["/aaqe-results/project-runner","test","tests/baseline.rs","{result_dir}/baseline.xml"]]})
        }
        "bug-hunter" => {
            json!({"summary":"Negative input bypasses lower bound","candidates":[{"id":"lower-bound","title":"Negative inputs remain negative","expected":"zero","observed":"negative value","trigger":"bounded_value(-1)","evidence_ids":["source-1"],"confidence":"high"}],"excluded_areas":[]})
        }
        "test-designer" => {
            json!({"summary":"Check lower bound","plans":[{"candidate_id":"lower-bound","test_id":"negative-input","expectation":"zero","oracle":"README contract: inclusive zero to ten","steps":["call bounded_value(-1)","assert equal to zero"],"reproduction":["run saved test"],"blocked_reason":null}]})
        }
        "test-developer" => {
            let mut source = String::from(
                "#[path = \"../app.rs\"] mod app;\n#[test] fn test_negative() { assert_eq!(app::bounded_value(-1), 0); }\n",
            );
            if flag(args, "--early-inline") {
                source.push_str("#[test] fn dependency_ready() { assert_eq!(aaqe_test_dependency::VALUE, 17); }\n");
            }
            if inputs["attempt"].as_u64().unwrap_or(1) > 1 {
                assert!(!inputs["execution"]["tests"].as_array().unwrap().is_empty());
                source.insert_str(0, "// Reviewed against execution feedback\n");
            }
            let patch = format!(
                "--- /dev/null\n+++ b/tests/regression.rs\n@@ -0,0 +1,{} @@\n{}",
                source.lines().count(),
                source
                    .lines()
                    .map(|line| format!("+{line}\n"))
                    .collect::<String>()
            );
            json!({"summary":"Add lower-bound regression","coverage":{"command":["/aaqe-results/project-runner","coverage","{result_dir}"],"report":"coverage.json","files":["app.rs"]},"patch":patch,"tests":[{"id":"negative-input","candidate_id":"lower-bound","command":["/aaqe-results/project-runner","test","tests/regression.rs","{result_dir}/junit.xml"],"report":"junit.xml","expected_names":["test_negative"],"reproduction_commands":[]}],"limitations":[]})
        }
        "test-validator" => {
            json!({"summary":"Assertion demonstrates lower-bound defect","coverage_assessment":{"scope_valid":true,"measurement_valid":true,"meaningful_assertions":true,"rationale":"Reviewed native evidence and complete production scope"},"verdicts":[{"candidate_id":"lower-bound","test_ids":["negative-input"],"status":"confirmed","assessment":{"executed":true,"assertion_failed":true,"infrastructure_failure":false},"rationale":"Baseline passes; regression observes -1 instead of zero","evidence":[Path::new(inputs["execution"]["tests"][0]["directory"].as_str().unwrap()).join("execution.json")]}]})
        }
        _ => panic!("unknown stage {stage}"),
    };
    if flag(args, "--early-inline") {
        assert_eq!(inputs["environment_ready"], true);
        if stage == "code-explorer" {
            output["environment"] = inputs["environment"].clone();
        }
        if stage == "test-developer" {
            output["tests"][0]["baseline"] = json!([[
                "/aaqe-results/project-runner",
                "test",
                "tests/baseline.rs",
                "{result_dir}/baseline.xml"
            ]]);
        }
    }
    if flag(args, "--loop-test") && stage == "bug-hunter" {
        output["candidates"] = json!([]);
    }
    save(Path::new(argument(args, "--output-last-message")), &output);
}
