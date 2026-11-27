//! Standalone Rust test project; instantiated only in temporary repositories.
use std::{fs, path::Path};

pub fn image() -> String {
    std::env::var("AAQE_TEST_RUST_IMAGE").unwrap_or_else(|_| "rust:1.88-slim".into())
}
pub const APP: &str = "pub fn bounded_value(value: i32) -> i32 { value.min(10) }\n";
pub const BASELINE: &str = "#[path = \"../app.rs\"] mod app;\n#[test] fn test_upper_bound() { assert_eq!(app::bounded_value(20), 10); }\n";
pub const RUNNER: &str = r###"
use std::{fs, path::Path, process::Command, thread, time::Duration};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args[1].as_str() {
        "ready" => {
            assert!(
                Path::new("/aaqe-results/libaaqe_test_dependency.rlib").exists(),
                "runner dependency is missing"
            );
            if !Path::new("/aaqe-results/early-ready").exists() {
                assert!(!Path::new("/workspace/tests/regression.rs").exists());
            }
            let error = fs::write("/workspace/readiness-must-not-write", "forbidden")
                .expect_err("worktree mount was writable");
            assert_eq!(error.raw_os_error(), Some(30));
            fs::write(
                "/aaqe-results/early-ready",
                "ready before Explorer and Developer",
            )
            .unwrap();
        }
        "installer" => {
            fs::write("/aaqe-results/installer-started", "started").unwrap();
            thread::sleep(Duration::from_secs(8));
            fs::write("/aaqe-results/installer-leaked", "leaked").unwrap();
        }
        "fail" => std::process::exit(9),
        "test" => {
            let report = Path::new(&args[3]);
            fs::create_dir_all(report.parent().unwrap()).unwrap();
            let binary = report.with_extension("bin");
            let mut compiler = Command::new("rustc");
            compiler
                .args(["--edition=2024", "--test", &args[2], "-o"])
                .arg(&binary);
            if Path::new("/aaqe-results/libaaqe_test_dependency.rlib").exists() {
                compiler.args([
                    "--extern",
                    "aaqe_test_dependency=/aaqe-results/libaaqe_test_dependency.rlib",
                ]);
            }
            let compiled = compiler.output().unwrap();
            if !compiled.status.success() {
                eprint!("{}", String::from_utf8_lossy(&compiled.stderr));
                std::process::exit(2);
            }
            let output = Command::new(binary)
                .args(["--test-threads=1", "--color=never"])
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&output.stdout);
            print!("{text}");
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
            let mut xml = String::from("<testsuite>");
            for line in text.lines() {
                if let Some(case) = line.strip_prefix("test ") {
                    if let Some((name, status)) = case.split_once(" ... ") {
                        xml.push_str(&format!("<testcase name=\"{name}\">"));
                        match status {
                            "FAILED" => {
                                xml.push_str("<failure message=\"Rust assertion failed\"/>")
                            }
                            "ignored" => xml.push_str("<skipped/>"),
                            _ => {}
                        }
                        xml.push_str("</testcase>");
                    }
                }
            }
            xml.push_str("</testsuite>");
            fs::write(report, xml).unwrap();
            std::process::exit(if output.status.success() { 0 } else { 1 });
        }
        _ => panic!("unknown runner operation"),
    }
}
"###;

pub fn create(root: &Path) {
    fs::create_dir_all(root.join("tests")).unwrap();
    for (name, content) in [
        ("app.rs", APP),
        ("run_tests.rs", RUNNER),
        ("dependency.rs", "pub const VALUE: i32 = 17;\n"),
        ("tests/baseline.rs", BASELINE),
        (
            "README.md",
            "bounded_value returns a value between zero and ten, inclusive.\n",
        ),
    ] {
        fs::write(root.join(name), content).unwrap();
    }
    fs::write(
        root.join("compose.yaml"),
        format!(
            "services:\n  runner:\n    image: {}\n    command: [sleep, infinity]\n    init: true\n",
            image()
        ),
    )
    .unwrap();
    fs::write(
        root.join("aaqe.yaml"),
        r#"environment:
  compose_files: [compose.yaml]
  services: [runner]
  runner_service: runner
  runner_workdir: /workspace
  readiness:
    - [rustc, --edition=2024, /workspace/run_tests.rs, -o, /aaqe-results/project-runner]
baseline:
  - [/aaqe-results/project-runner, test, tests/baseline.rs, "{result_dir}/baseline.xml"]
"#,
    )
    .unwrap();
}
