use std::{fs, path::Path, process::Command};

use super::*;
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn repository() -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.name", "Test User"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    fs::write(root.join("app.rs"), "pub fn value() -> i32 { 1 }\n").unwrap();
    fs::create_dir(root.join("tests")).unwrap();
    fs::write(
        root.join("tests/test_app.rs"),
        "#[test]\nfn test_value() { assert_eq!(1 + 1, 2); }\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);
    directory
}

fn gate(root: &Path) -> PatchGate {
    PatchGate::new(root, &["tests/**".into(), "**/tests/**".into()], None).unwrap()
}

fn patch(path: &str, before: &str, after: &str) -> String {
    let mut result = format!(
        "--- {}\n+++ b/{path}\n",
        if before.is_empty() {
            "/dev/null".into()
        } else {
            format!("a/{path}")
        }
    );
    let before_count = before.lines().count();
    let after_count = after.lines().count();
    result.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        usize::from(before_count != 0),
        before_count,
        usize::from(after_count != 0),
        after_count
    ));
    for line in before.lines() {
        result.push_str(&format!("-{line}\n"));
    }
    for line in after.lines() {
        result.push_str(&format!("+{line}\n"));
    }
    result
}

#[test]
fn new_test_applies_without_touching_caller_index() {
    let repo = repository();
    let before_index = git(repo.path(), &["ls-files", "--stage"]);
    let paths = gate(repo.path())
        .apply(&patch(
            "tests/new.rs",
            "",
            "#[test] fn sum() { assert_eq!(2 + 2, 4); }\n",
        ))
        .unwrap();
    assert_eq!(paths, vec!["tests/new.rs"]);
    assert!(repo.path().join("tests/new.rs").exists());
    assert_eq!(
        fs::read_to_string(repo.path().join("app.rs")).unwrap(),
        "pub fn value() -> i32 { 1 }\n"
    );
    assert_eq!(git(repo.path(), &["ls-files", "--stage"]), before_index);
    gate(repo.path()).verify_worktree().unwrap();
}

#[test]
fn production_rejection_is_atomic() {
    let repo = repository();
    let change = patch(
        "tests/new.rs",
        "",
        "#[test] fn regression() { assert_eq!(1 + 1, 2); }\n",
    ) + &patch(
        "app.rs",
        "pub fn value() -> i32 { 1 }\n",
        "pub fn value() -> i32 { 2 }\n",
    );
    let error = gate(repo.path()).apply(&change).unwrap_err().to_string();
    assert!(error.contains("production"), "{error}");
    assert!(!repo.path().join("tests/new.rs").exists());
    assert!(
        fs::read_to_string(repo.path().join("app.rs"))
            .unwrap()
            .contains("{ 1 }")
    );
}

#[test]
fn rust_test_module_change_preserves_production() {
    let repo = repository();
    let before = "pub fn value() -> u8 { 1 }\n#[cfg(test)]\nmod tests { #[test] fn old() { assert_eq!(super::value(), 1); } }\n";
    fs::create_dir(repo.path().join("src")).unwrap();
    fs::write(repo.path().join("src/lib.rs"), before).unwrap();
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-m", "rust fixture"]);
    let after = before.replace("fn old()", "fn regression()");
    gate(repo.path())
        .apply(&patch("src/lib.rs", before, &after))
        .unwrap();
    gate(repo.path()).verify_worktree().unwrap();
    fs::write(repo.path().join("src/lib.rs"), before).unwrap();
    assert!(
        gate(repo.path())
            .apply(&patch(
                "src/lib.rs",
                before,
                &after.replace("{ 1 }", "{ 2 }")
            ))
            .unwrap_err()
            .to_string()
            .contains("production")
    );
}

#[test]
fn string_fake_test_attribute_and_invalid_rust_do_not_authorize_changes() {
    let repo = repository();
    let before = "pub const TEXT: &str = \"#[cfg(test)] mod tests {}\";\n";
    fs::write(repo.path().join("lib.rs"), before).unwrap();
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-m", "string fixture"]);
    assert!(
        gate(repo.path())
            .apply(&patch("lib.rs", before, &before.replace("TEXT", "OTHER")))
            .is_err()
    );
    assert!(
        gate(repo.path())
            .apply(&patch("lib.rs", before, "#[cfg(test)] mod tests {\n"))
            .is_err()
    );
}

#[test]
fn paths_reject_traversal_git_metadata_and_links() {
    let repo = repository();
    for path in [
        "../escaped.rs",
        ".git/config",
        "tests/../../escaped.rs",
        "tests\\escaped.rs",
        "/tmp/escaped.rs",
        "tests/a:b",
    ] {
        assert!(safe_path(repo.path(), path).is_err(), "{path}");
        assert!(
            gate(repo.path())
                .apply(&patch(path, "", "pub const ESCAPED: bool = true;\n"))
                .is_err(),
            "{path}"
        );
    }
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), repo.path().join("tests/link")).unwrap();
        assert!(
            gate(repo.path())
                .apply(&patch(
                    "tests/link/escaped.rs",
                    "",
                    "pub const ESCAPED: bool = true;\n"
                ))
                .is_err()
        );
        assert!(!outside.path().join("escaped.rs").exists());
    }
}

#[test]
fn deletion_mode_binary_and_changed_base_are_rejected() {
    let repo = repository();
    for change in [
        "--- a/tests/test_app.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-#[test]\n-fn test_value() { assert_eq!(1 + 1, 2); }\n",
        "old mode 100644\nnew mode 100755\n",
        "GIT binary patch\n",
        "",
    ] {
        assert!(gate(repo.path()).apply(change).is_err());
    }
    let old_gate = gate(repo.path());
    git(
        repo.path(),
        &["commit", "--allow-empty", "-m", "changed base"],
    );
    assert!(
        old_gate
            .verify_worktree()
            .unwrap_err()
            .to_string()
            .contains("base commit")
    );
    assert!(
        old_gate
            .apply(&patch(
                "tests/new.rs",
                "",
                "#[test] fn regression() { assert_eq!(1 + 1, 2); }\n"
            ))
            .is_err()
    );
}

#[test]
fn execution_rejects_untracked_or_removed_production() {
    let repo = repository();
    fs::write(
        repo.path().join("new_production.rs"),
        "pub const VALUE: i32 = 1;\n",
    )
    .unwrap();
    assert!(
        gate(repo.path())
            .verify_worktree()
            .unwrap_err()
            .to_string()
            .contains("production")
    );
    fs::remove_file(repo.path().join("new_production.rs")).unwrap();
    fs::remove_file(repo.path().join("tests/test_app.rs")).unwrap();
    assert!(
        gate(repo.path())
            .verify_worktree()
            .unwrap_err()
            .to_string()
            .contains("removed")
    );
}

#[test]
fn test_path_patterns_respect_wildcards_and_negated_character_classes() {
    let repo = repository();
    let custom = PatchGate::new(repo.path(), &["**/tests/**".into()], None).unwrap();
    assert!(
        custom
            .apply(&patch(
                "tests/new.rs",
                "",
                "#[test] fn regression() { assert_eq!(1 + 1, 2); }\n"
            ))
            .is_err()
    );
    let custom = PatchGate::new(repo.path(), &["tests/{one,two}/**".into()], None).unwrap();
    assert!(
        custom
            .apply(&patch(
                "tests/one/new.rs",
                "",
                "#[test] fn regression() { assert_eq!(1 + 1, 2); }\n"
            ))
            .is_err()
    );
    let custom = PatchGate::new(repo.path(), &["tests/test_[!a-c]?.rs".into()], None).unwrap();
    custom
        .apply(&patch(
            "tests/test_z1.rs",
            "",
            "#[test] fn regression() { assert_eq!(1 + 1, 2); }\n",
        ))
        .unwrap();
    assert!(
        custom
            .apply(&patch(
                "tests/test_a1.rs",
                "",
                "#[test] fn regression() { assert_eq!(1 + 1, 2); }\n"
            ))
            .is_err()
    );
}
