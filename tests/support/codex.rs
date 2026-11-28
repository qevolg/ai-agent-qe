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
    let product = load(&directory.join("input.json"))["state"]["workflow"] == "product";
    let explorer = if product {
        "product-explorer"
    } else {
        "code-explorer"
    };
    let investigation = if product {
        "scenario-designer"
    } else {
        "bug-hunter"
    };
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
            "finish"
        ])
    );
    assert_eq!(
        client.tool(
            "run_stage",
            json!({"stage":if product {"product-executor"} else {"test-developer"},"instruction":"too early"})
        )["isError"],
        true
    );
    assert_eq!(
        client.tool("finish", json!({"status":"completed","reason":"too early"}))["isError"],
        true
    );
    if !product && flag(args, "--early-inline") {
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
    let mut stages = if product {
        vec![
            "product-explorer",
            "scenario-designer",
            "product-executor",
            "issue-reproducer",
            "result-reviewer",
        ]
    } else {
        vec![
            "code-explorer",
            "bug-hunter",
            "test-designer",
            "test-developer",
            "test-validator",
        ]
    };
    if !product && flag(args, "--retry-developer") {
        stages.insert(4, "test-developer");
    }
    if flag(args, "--loop-test") || flag(args, "--cancel-test") {
        if product {
            stages.insert(1, explorer);
        } else {
            stages = vec![explorer, explorer, investigation];
        }
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
            && stage == investigation
        {
            thread::sleep(Duration::from_secs(30));
            fs::write(directory.join("continued-after-policy"), "").unwrap();
        }
        if stage == explorer && result["isError"] == false {
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
        let (status, reason) = if product {
            (
                "blocked",
                "生命周期测试夹具未提供真实产品服务部署；已保存受阻场景并独立复核，未执行产品操作",
            )
        } else {
            (
                "completed",
                "All current candidates were checked independently",
            )
        };
        let result = client.tool("finish", json!({"status":status,"reason":reason}));
        assert_eq!(result["isError"], false, "{result}");
    }
    println!("Audit finished via harness tools; ready for follow-up");
    io::stdout().flush().unwrap();
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let status = client.tool("status", json!({}));
        assert!(status["structuredContent"]["state"]["finished_at"].is_string());
        assert_eq!(
            client.tool(
                "run_stage",
                json!({"stage":explorer,"instruction":"after audit"})
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
        && matches!(stage, "bug-hunter" | "scenario-designer")
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
        "product-explorer" => {
            json!({
                "summary":"检查 Rust 函数及真实服务部署前提", "scope":["app.rs"],
                "references":[{"id":"source-1","path":"app.rs","line":1,"claim":"Implementation clamps only the upper bound"}],
                "uncertainties":["生命周期测试仓库没有真实产品服务的部署定义"],
                "entry_points":[{"id":"cli-bounds","kind":"cli","description":"调用实际产品的边界值检查入口","applicable":true,"reason":"待部署真实产品后验证"}],
                "services":[{"id":"bounded-product","name":"Bounded product","source":"docker","endpoint":null,"compose_service":"bounded-product","version":null,"check":["bounded-product-client","version"]}],
                "environment":{"compose_files":[],"services":["runner"],"runner_service":"runner","runner_workdir":"/workspace","external_containers":[],"readiness_timeout":90,
                    "compose":{"services":{"runner":{"image":super::project::image(),"command":["sleep","infinity"]}}},"readiness":[],"setup":[]}
            })
        }
        "scenario-designer" => {
            json!({
                "summary":"先保存下边界场景及服务部署阻碍",
                "scenarios":[{"id":"negative-input","title":"负数输入限制为零","feature":"inclusive bounds","entry_point":"cli","category":"boundary","preconditions":["部署真实 bounded-product 服务及其客户端"],"services":["bounded-product"],"oracle":"输入 -1 应返回 0",
                    "steps":[{"id":"read-negative","action":"通过真实产品 CLI 传入 -1","expected":"实际输出 0"}],"cleanup":[],"blocked_reason":"生命周期测试夹具未提供真实产品服务部署；未执行产品操作","derived_from":null,"reason":null}],
                "coverage":[{"feature":"inclusive bounds","entry_point":"cli","category":"boundary","status":"planned","rationale":"已记录场景，等待真实服务部署","scenario_ids":["negative-input"]}],"limitations":["此测试检查协调器生命周期，不伪造产品执行证据"]
            })
        }
        "product-executor" => {
            json!({"summary":"缺少真实服务部署，保留受阻场景","operations":[],"observations":["场景已保存；真实服务尚未提供"],"limitations":["未执行产品操作"]})
        }
        "issue-reproducer" => {
            json!({"summary":"没有产品操作证据可用于缺陷复现","operations":[],"observations":[],"limitations":["真实服务部署缺失"],"issues":[],"followups":[]})
        }
        "result-reviewer" => {
            json!({"summary":"独立确认场景受真实服务部署阻碍","scenario_results":[{"scenario_id":"negative-input","status":"blocked","rationale":"缺少真实产品服务，未执行场景，不能判断功能是否正确","evidence":[],"assessment":{"executed":false,"expectation_met":false,"infrastructure_failure":true,"no_mocks":true,"real_services_verified":false,"entry_point_verified":false}}],"issue_results":[],"limitations":["没有产品执行或缺陷结论"]})
        }
        "code-explorer" => {
            json!({"summary":"Rust scope examined","scope":["app.rs"],"references":[{"id":"source-1","path":"app.rs","line":1,"claim":"Implementation clamps only the upper bound; README specifies zero to ten"}],"uncertainties":[],"real_services":[],"environment":{"compose_files":["compose.yaml"],"services":["runner"],"runner_service":"runner","runner_workdir":"/workspace","external_containers":[],"readiness_timeout":90,"readiness":[["rustc","--edition=2024","/workspace/run_tests.rs","-o","/aaqe-results/project-runner"]]},"baseline":[["/aaqe-results/project-runner","test","tests/baseline.rs","{result_dir}/baseline.xml"]]})
        }
        "bug-hunter" => {
            json!({"summary":"Negative input bypasses lower bound","candidates":[{"id":"lower-bound","title":"Negative inputs remain negative","expected":"zero","observed":"negative value","trigger":"bounded_value(-1)","evidence_ids":["source-1"],"confidence":"high"}],"excluded_areas":[]})
        }
        "test-designer" => {
            json!({"summary":"Check lower bound","plans":[{"candidate_id":"lower-bound","test_id":"negative-input","expectation":"zero","oracle":"README contract: inclusive zero to ten","steps":["call bounded_value(-1)","assert equal to zero"],"reproduction":["run saved test"],"blocked_reason":null,"real_services":[]}]})
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
            json!({"summary":"Add lower-bound regression","coverage":{"command":["/aaqe-results/project-runner","coverage","{result_dir}"],"report":"coverage.json","files":["app.rs"]},"patch":patch,"tests":[{"id":"negative-input","candidate_id":"lower-bound","command":["/aaqe-results/project-runner","test","tests/regression.rs","{result_dir}/junit.xml"],"report":"junit.xml","expected_names":["test_negative"],"reproduction_commands":[],"service_checks":[]}],"limitations":[]})
        }
        "test-validator" => {
            json!({"summary":"Assertion demonstrates lower-bound defect","coverage_assessment":{"scope_valid":true,"measurement_valid":true,"meaningful_assertions":true,"rationale":"Reviewed native evidence and complete production scope"},"verdicts":[{"candidate_id":"lower-bound","test_ids":["negative-input"],"status":"confirmed","assessment":{"executed":true,"assertion_failed":true,"infrastructure_failure":false,"no_mocks":true,"real_services_verified":true},"rationale":"Baseline passes; regression observes -1 instead of zero","evidence":[Path::new(inputs["execution"]["tests"][0]["directory"].as_str().unwrap()).join("execution.json")]}]})
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
