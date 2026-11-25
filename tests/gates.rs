use std::{fs, path::Path, process::Command};

use aaqe::{
    evidence::parse_junit,
    patchgate::{PatchGate, safe_path},
};
use serde_json::json;
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
    fs::write(root.join("app.py"), "def value():\n    return 1\n").unwrap();
    fs::create_dir(root.join("tests")).unwrap();
    fs::write(
        root.join("tests/test_app.py"),
        "def test_value():\n    assert True\n",
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
        .apply(&patch("tests/new.py", "", "assert 2 + 2 == 4\n"))
        .unwrap();
    assert_eq!(paths, vec!["tests/new.py"]);
    assert!(repo.path().join("tests/new.py").exists());
    assert_eq!(
        fs::read_to_string(repo.path().join("app.py")).unwrap(),
        "def value():\n    return 1\n"
    );
    assert_eq!(git(repo.path(), &["ls-files", "--stage"]), before_index);
    gate(repo.path()).verify_worktree().unwrap();
}

#[test]
fn production_rejection_is_atomic() {
    let repo = repository();
    let change = patch("tests/new.py", "", "assert True\n")
        + &patch(
            "app.py",
            "def value():\n    return 1\n",
            "def value():\n    return 2\n",
        );
    let error = gate(repo.path()).apply(&change).unwrap_err().to_string();
    assert!(error.contains("production"), "{error}");
    assert!(!repo.path().join("tests/new.py").exists());
    assert!(
        fs::read_to_string(repo.path().join("app.py"))
            .unwrap()
            .contains("return 1")
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
        "../escaped.py",
        ".git/config",
        "tests/../../escaped.py",
        "tests\\escaped.py",
        "/tmp/escaped.py",
        "tests/a:b",
    ] {
        assert!(safe_path(repo.path(), path).is_err(), "{path}");
        assert!(
            gate(repo.path())
                .apply(&patch(path, "", "escaped = True\n"))
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
                .apply(&patch("tests/link/escaped.py", "", "escaped = True\n"))
                .is_err()
        );
        assert!(!outside.path().join("escaped.py").exists());
    }
}

#[test]
fn deletion_mode_binary_and_changed_base_are_rejected() {
    let repo = repository();
    for change in [
        "--- a/tests/test_app.py\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-def test_value():\n-    assert True\n",
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
            .apply(&patch("tests/new.py", "", "assert True\n"))
            .is_err()
    );
}

#[test]
fn execution_rejects_untracked_or_removed_production() {
    let repo = repository();
    fs::write(repo.path().join("new_production.py"), "value = 1\n").unwrap();
    assert!(
        gate(repo.path())
            .verify_worktree()
            .unwrap_err()
            .to_string()
            .contains("production")
    );
    fs::remove_file(repo.path().join("new_production.py")).unwrap();
    fs::remove_file(repo.path().join("tests/test_app.py")).unwrap();
    assert!(
        gate(repo.path())
            .verify_worktree()
            .unwrap_err()
            .to_string()
            .contains("removed")
    );
}

#[test]
fn test_path_patterns_keep_python_fnmatch_semantics() {
    let repo = repository();
    let custom = PatchGate::new(repo.path(), &["**/tests/**".into()], None).unwrap();
    assert!(
        custom
            .apply(&patch("tests/new.py", "", "assert True\n"))
            .is_err()
    );
    let custom = PatchGate::new(repo.path(), &["tests/{one,two}/**".into()], None).unwrap();
    assert!(
        custom
            .apply(&patch("tests/one/new.py", "", "assert True\n"))
            .is_err()
    );
    let custom = PatchGate::new(repo.path(), &["tests/test_[!a-c]?.py".into()], None).unwrap();
    custom
        .apply(&patch("tests/test_z1.py", "", "assert True\n"))
        .unwrap();
    assert!(
        custom
            .apply(&patch("tests/test_a1.py", "", "assert True\n"))
            .is_err()
    );
}

fn evidence(xml: &str, names: &[&str]) -> serde_json::Value {
    let directory = tempfile::tempdir().unwrap();
    let report = directory.path().join("junit.xml");
    fs::write(&report, xml).unwrap();
    parse_junit(
        &report,
        &names
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>(),
    )
}

#[test]
fn junit_missing_case_and_empty_expectations_are_incomplete() {
    let result = evidence(
        "<testsuite><testcase name=\"unrelated\"/></testsuite>",
        &["regression"],
    );
    assert_eq!(result["complete"], false);
    assert_eq!(result["missing"], json!(["regression"]));
    assert_eq!(evidence("<testsuite/>", &[])["complete"], false);
}

#[test]
fn junit_failed_target_retains_detail_while_skipped_is_incomplete() {
    let result = evidence(
        "<testsuite><testcase classname=\"suite\" name=\"regression\"><failure message=\"wrong value\">assertion details</failure></testcase><testcase name=\"other\"><skipped/></testcase></testsuite>",
        &["suite::regression", "other"],
    );
    assert_eq!(result["cases"][0]["status"], "failed");
    assert_eq!(
        result["cases"][0]["detail"],
        "wrong value\nassertion details"
    );
    assert_eq!(result["cases"][1]["status"], "skipped");
    assert_eq!(result["complete"], false);
}

#[test]
fn junit_duplicate_identity_requires_classname() {
    let xml = "<testsuite><testcase classname=\"one\" name=\"regression\"/><testcase classname=\"two\" name=\"regression\"/></testsuite>";
    assert_eq!(evidence(xml, &["regression"])["complete"], false);
    assert_eq!(evidence(xml, &["two::regression"])["complete"], true);
}

#[test]
fn junit_utf16_reports_preserve_target_case_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("junit.xml");
    let source = "<?xml version=\"1.0\" encoding=\"utf-16\"?><testsuite><testcase name=\"回归\"/></testsuite>";
    let mut bytes = vec![0xff, 0xfe];
    for character in source.encode_utf16() {
        bytes.extend_from_slice(&character.to_le_bytes());
    }
    fs::write(&path, bytes).unwrap();
    assert_eq!(parse_junit(&path, &["回归".into()])["complete"], true);
}

#[test]
fn junit_latin1_reports_preserve_accented_identity_and_failure_details() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("junit.xml");
    fs::write(&path, b"<?xml version='1.0' encoding='ISO-8859-1'?><testsuite><testcase name='caf\xe9'><failure message='d\xe9faut'>r\xe9sultat\x80</failure></testcase></testsuite>").unwrap();
    let parsed = parse_junit(&path, &["café".into()]);
    assert_eq!(parsed["complete"], true);
    assert_eq!(parsed["cases"][0]["detail"], "défaut\nrésultat\u{80}");
}

#[test]
fn junit_declared_windows1252_decodes_without_latin1_aliasing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("junit.xml");
    fs::write(&path, b"<?xml version='1.0' encoding='windows-1252'?><testsuite><testcase name='cost-\x80'/></testsuite>").unwrap();
    let parsed = parse_junit(&path, &["cost-€".into()]);
    assert_eq!(parsed["complete"], true);
}

#[test]
fn junit_declared_encodings_reject_invalid_bytes_instead_of_replacing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("junit.xml");
    for (encoding, invalid) in [
        ("ASCII", b"\xe9".as_slice()),
        ("UTF-8", b"\xff".as_slice()),
        ("Shift_JIS", b"\x82".as_slice()),
    ] {
        let mut bytes = format!("<?xml version='1.0' encoding='{encoding}'?><testsuite><testcase name='regression'><failure>").into_bytes();
        bytes.extend_from_slice(invalid);
        bytes.extend_from_slice(b"</failure></testcase></testsuite>");
        fs::write(&path, bytes).unwrap();
        let parsed = parse_junit(&path, &["regression".into()]);
        assert_eq!(parsed["complete"], false, "{encoding}");
        assert_eq!(parsed["cases"], json!([]), "{encoding}");
    }
}

#[test]
fn junit_internal_entities_expand_in_target_identities_and_details() {
    let result = evidence(
        "<!DOCTYPE testsuite [<!ENTITY label 'café'><!ENTITY detail 'résultat attendu'>]><testsuite><testcase name='&label;'><failure>&detail;</failure></testcase></testsuite>",
        &["café"],
    );
    assert_eq!(result["complete"], true);
    assert_eq!(result["cases"][0]["detail"], "résultat attendu");
}

#[test]
fn junit_external_entities_remain_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    let external = directory.path().join("external.txt");
    fs::write(&external, "sensitive external content").unwrap();
    let source = format!(
        "<!DOCTYPE testsuite [<!ENTITY external SYSTEM 'file://{}'>]><testsuite><testcase name='regression'><failure>&external;</failure></testcase></testsuite>",
        external.display()
    );
    let result = evidence(&source, &["regression"]);
    assert_eq!(result["complete"], false);
    assert_eq!(result["cases"], json!([]));
}

#[test]
fn junit_placeholder_disabled_error_and_malformed_report_are_incomplete() {
    for xml in [
        "<testsuite><testcase name=\"ctest-incomplete\"><failure/></testcase></testsuite>",
        "<testsuite><testcase name=\"regression\" status=\"notrun\"/></testsuite>",
        "<testsuite><testcase name=\"regression\"><error/></testcase></testsuite>",
        "<testsuite><testcase classname=\"worker-admission\" name=\"regression\"/></testsuite>",
        "<not-xml",
    ] {
        assert_eq!(evidence(xml, &["regression"])["complete"], false, "{xml}");
    }
    assert_eq!(
        parse_junit(
            Path::new("/does-not-exist/aaqe-junit.xml"),
            &["regression".into()]
        )["complete"],
        false
    );
}
