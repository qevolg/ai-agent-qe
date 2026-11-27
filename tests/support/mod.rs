#![allow(dead_code)]

use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub fn worker() -> &'static Path {
    static WORKER: OnceLock<PathBuf> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            if let Some(path) = std::env::var_os("AAQE_TEST_WORKER") {
                return PathBuf::from(path);
            }
            let executable = std::env::current_exe().unwrap();
            executable
                .ancestors()
                .map(|path| path.join("examples/aaqe-test-worker"))
                .find(|path| path.is_file())
                .expect("Build the Rust test worker first: cargo build --locked --example aaqe-test-worker")
        })
        .as_path()
}

pub fn command(spec: Value) -> Vec<String> {
    vec![
        worker().display().to_string(),
        "process".into(),
        spec.to_string(),
    ]
}

pub fn codex(flags: &[&str]) -> Vec<String> {
    std::iter::once(worker().display().to_string())
        .chain(std::iter::once("codex".into()))
        .chain(flags.iter().map(|s| (*s).to_owned()))
        .collect()
}

pub fn executable(path: &Path, spec: Value) {
    fs::copy(worker(), path).unwrap();
    fs::write(path.with_extension("json"), spec.to_string()).unwrap();
}

pub fn path_with(directory: &Path) -> std::ffi::OsString {
    let mut paths = vec![directory.to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths).unwrap()
}

pub fn repository(directory: &Path, rust: bool) -> PathBuf {
    let remote = directory.join("remote.git");
    let root = directory.join("repo");
    fs::create_dir(&root).unwrap();
    for (cwd, args) in [
        (
            directory,
            vec![
                "init",
                "--bare",
                "--initial-branch=main",
                remote.to_str().unwrap(),
            ],
        ),
        (root.as_path(), vec!["init", "-b", "main"]),
        (root.as_path(), vec!["config", "user.name", "AAQE Test"]),
        (
            root.as_path(),
            vec!["config", "user.email", "test@example.invalid"],
        ),
    ] {
        aaqe::runtime::git::git(cwd, &args).unwrap();
    }
    fs::write(root.join("app.rs"), "pub fn value() -> i32 { 1 }\n").unwrap();
    if rust {
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn value() -> i32 { 1 }\n").unwrap();
    }
    for args in [
        vec!["add", "."],
        vec!["commit", "-m", "base"],
        vec!["remote", "add", "origin", remote.to_str().unwrap()],
        vec!["push", "origin", "main"],
    ] {
        aaqe::runtime::git::git(&root, &args).unwrap();
    }
    root
}

pub fn success() -> Vec<String> {
    command(json!({}))
}

pub mod project;
