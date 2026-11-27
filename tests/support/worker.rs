//! Native integration-test subprocess. Never installed with AAQE.
mod codex;
#[allow(dead_code)]
mod project;

use serde_json::{Value, json};
use std::{
    env, fs,
    io::{self, Read, Write},
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn wait(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !path.exists() {
        assert!(Instant::now() < deadline, "waiting for {}", path.display());
        thread::sleep(Duration::from_millis(10));
    }
}

fn process(spec: &Value) {
    if spec["ignore_signals"] == true {
        unsafe {
            libc::signal(libc::SIGTERM, libc::SIG_IGN);
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
        }
    }
    if let Some(p) = spec["pid"].as_str() {
        fs::write(p, std::process::id().to_string()).unwrap();
    }
    if spec["child"] == true {
        let mut child_spec = json!({"delay_ms":60000});
        if spec["stubborn_child"] == true {
            child_spec["ignore_signals"] = json!(true);
        }
        if let Some(p) = spec["child_pid"].as_str() {
            child_spec["pid"] = json!(p);
        }
        let mut child = Command::new(env::current_exe().unwrap())
            .args(["process", &child_spec.to_string()])
            .spawn()
            .unwrap();
        if let Some(p) = spec["child_pid"].as_str() {
            fs::write(p, child.id().to_string()).unwrap();
        }
        thread::spawn(move || child.wait().unwrap());
    }
    if spec["stdin"] == true || spec["input_file"].is_string() {
        let mut input = String::new();
        io::stdin().read_to_string(&mut input).unwrap();
        if let Some(path) = spec["input_file"].as_str() {
            fs::write(path, &input).unwrap();
        }
        if spec["stdin"] == true {
            print!("{input}");
        }
    }
    if let Some(key) = spec["print_env"].as_str() {
        println!("{}", env::var(key).unwrap_or("unset".into()));
    }
    if let Some(key) = spec["stderr_env"].as_str() {
        eprintln!("{}", env::var(key).unwrap_or_default());
    }
    if let Some(s) = spec["stdout"].as_str() {
        print!("{s}");
    }
    if let Some(s) = spec["stderr"].as_str() {
        eprint!("{s}");
    }
    if spec["large"] == true {
        io::stdout().write_all(b"BEGIN\n").unwrap();
        io::stdout().write_all(&vec![b'x'; 2_000_000]).unwrap();
        io::stdout().write_all(b"\nEND").unwrap();
        io::stderr().write_all(b"ERROR-BEGIN\n").unwrap();
        io::stderr().write_all(&vec![b'y'; 2_000_000]).unwrap();
        io::stderr().write_all(b"\nERROR-END").unwrap();
    }
    if spec["binary"] == true {
        io::stdout().write_all(&vec![255; 60_000]).unwrap();
    }
    io::stdout().flush().unwrap();
    io::stderr().flush().unwrap();
    if let Some(p) = spec["ready"].as_str() {
        fs::write(p, "").unwrap();
    }
    if let Some(p) = spec["wait"].as_str() {
        wait(Path::new(p));
    }
    thread::sleep(Duration::from_millis(
        spec["delay_ms"].as_u64().unwrap_or(0),
    ));
    if let Some(p) = spec["touch"].as_str() {
        fs::write(p, "").unwrap();
    }
    std::process::exit(spec["exit"].as_i64().unwrap_or(0) as i32);
}

fn configured(path: &Path, args: &[String]) {
    let spec: Value =
        serde_json::from_slice(&fs::read(path.with_extension("json")).unwrap()).unwrap();
    match spec["mode"].as_str() {
        Some("gitlab") => {
            let capture = Path::new(spec["capture"].as_str().unwrap());
            let mut mr = spec["mr"].clone();
            mr["source_project_id"] = json!(42);
            mr["target_project_id"] = json!(42);
            mr["sha"] = spec["sha"]
                .as_str()
                .map(|sha| json!(sha))
                .unwrap_or_else(|| {
                    let commit = Command::new("git")
                        .args([
                            "rev-parse",
                            &format!("refs/heads/{}", mr["source_branch"].as_str().unwrap()),
                        ])
                        .output()
                        .unwrap();
                    assert!(commit.status.success());
                    json!(String::from_utf8(commit.stdout).unwrap().trim())
                });
            if let Some(input) = args.windows(2).find(|pair| pair[0] == "--input") {
                let update = args.windows(2).any(|pair| pair == ["--method", "PUT"]);
                if update {
                    fs::copy(&input[1], capture.with_extension("update.json")).unwrap();
                    let body: Value =
                        serde_json::from_slice(&fs::read(&input[1]).unwrap()).unwrap();
                    mr["title"] = body["title"].clone();
                    mr["description"] = body["description"].clone();
                    mr["draft"] = json!(body["title"].as_str().unwrap().starts_with("Draft:"));
                } else {
                    assert!(!capture.exists(), "MR was created more than once");
                    fs::copy(&input[1], capture).unwrap();
                }
                println!("{mr}");
            } else if args
                .last()
                .is_some_and(|endpoint| !endpoint.contains("merge_requests"))
            {
                println!("{}", json!({"id":42}));
            } else if capture.exists() || spec["existing"] == true {
                println!("{}", json!([mr]));
            } else {
                println!("[]");
            }
        }
        Some("runner") => {
            assert_eq!(
                args,
                ["compose", "exec", "-T", "runner", "mvn", "--version"]
            );
            println!("Maven from runner");
        }
        Some("version") => {
            assert_eq!(args, [spec["flag"].as_str().unwrap()]);
            println!("{} installed version", spec["tool"].as_str().unwrap());
        }
        Some("ssh-capture") => {
            let mut data = vec![b'x'; 65536 - 8];
            data.extend(env::var("AAQE_TEST_PASSWORD").unwrap().bytes());
            data.extend(vec![b'y'; 2 * 1024 * 1024]);
            io::stdout().write_all(&data).unwrap();
            io::stderr().write_all(&data).unwrap();
        }
        Some("ssh-password") => {
            let password = Command::new(env::var("SSH_ASKPASS").unwrap())
                .output()
                .unwrap();
            assert!(password.status.success());
            io::stdout().write_all(&password.stdout).unwrap();
            eprint!("{}", env::var("AAQE_SSH_PASSWORD").unwrap());
            println!("\n{}", args.join(" "));
        }
        Some("tmux-race") => {
            let real = env::var("AAQE_REAL_TMUX").unwrap();
            if args.first().is_some_and(|a| a == "display-message")
                && args.last().is_some_and(|a| a == "#{pane_dead}")
            {
                let root = env::var("AAQE_TUI_EXIT_RACE_DIR").unwrap();
                let root = Path::new(&root);
                fs::write(root.join("pane-query"), "").unwrap();
                let deadline = Instant::now() + Duration::from_secs(3);
                loop {
                    let output = Command::new(&real).args(args).output().unwrap();
                    if root.join("stages/coordinator/finished").exists() && output.stdout == b"1\n"
                    {
                        io::stdout().write_all(&output.stdout).unwrap();
                        io::stderr().write_all(&output.stderr).unwrap();
                        std::process::exit(output.status.code().unwrap_or(1));
                    }
                    assert!(
                        Instant::now() < deadline,
                        "TUI worker did not exit during pane query"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
            }
            use std::os::unix::process::CommandExt;
            panic!("{}", Command::new(real).args(args).exec());
        }
        _ => process(&spec),
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let executable = env::current_exe().unwrap();
    let path = executable.as_path();
    if !args.get(1).is_some_and(|a| a == "process" || a == "idle")
        && path.with_extension("json").is_file()
    {
        configured(path, &args[1..]);
        return;
    }
    match args.get(1).map(String::as_str) {
        Some("executor-snapshot") => {
            let directory = Path::new(&args[2]);
            fs::write(directory.join("ready"), "").unwrap();
            wait(&directory.join("continue"));
            let metadata = aaqe::runtime::executor::capture(directory).unwrap();
            aaqe::state::write_json(&directory.join("captured.json"), &metadata).unwrap();
        }
        Some("process") => process(&serde_json::from_str(&args[2]).unwrap()),
        Some("codex") => codex::run(&args[2..]),
        Some("codex-output") => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input).unwrap();
            fs::write("received.txt", input).unwrap();
            let output = args
                .windows(2)
                .find(|a| a[0] == "--output-last-message")
                .unwrap();
            fs::write(
                &output[1],
                if args.iter().any(|a| a == "--invalid") {
                    "{}"
                } else {
                    r#"{"summary":"done","candidates":[],"excluded_areas":[]}"#
                },
            )
            .unwrap();
            println!("visible-output");
            eprintln!("visible-diagnostic");
        }
        Some("tui-race") => {
            let root = env::var("AAQE_TUI_EXIT_RACE_DIR").unwrap();
            let root = Path::new(&root);
            wait(&root.join("pane-query"));
            if root.ends_with("completed") {
                fs::write(root.join("stages/coordinator/audit-finished"), "").unwrap();
            } else {
                std::process::exit(23);
            }
        }
        Some("idle") => thread::sleep(Duration::from_secs(60)),
        _ => {}
    }
}
