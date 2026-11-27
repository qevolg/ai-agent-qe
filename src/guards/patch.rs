use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use regex::bytes::Regex;

use crate::runtime::process::{ProcessResult, run, run_env};

pub fn safe_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    ensure!(
        !relative.is_empty()
            && !path.is_absolute()
            && !relative.contains('\\')
            && !relative.contains(':')
            && !path
                .components()
                .any(|part| matches!(part, Component::ParentDir) || part.as_os_str() == ".git"),
        "Unsafe patch path: {relative}"
    );
    let canonical_root = root.canonicalize().context("Cannot resolve worktree")?;
    let destination = root.join(path);
    for parent in destination.ancestors() {
        if parent == root {
            break;
        }
        if fs::symlink_metadata(parent).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            bail!("Symbolic link in patch path: {relative}");
        }
    }
    let existing = destination
        .ancestors()
        .find(|parent| parent.exists())
        .context("Cannot resolve patch destination")?;
    ensure!(
        existing.canonicalize()?.starts_with(canonical_root),
        "Path escapes worktree: {relative}"
    );
    Ok(destination)
}

fn rust_production(source: &[u8]) -> Result<Vec<u8>> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
    let tree = parser
        .parse(source, None)
        .context("Cannot parse Rust source")?;
    ensure!(
        !tree.root_node().has_error(),
        "Cannot verify production boundary in invalid Rust syntax"
    );
    let mut cursor = tree.root_node().walk();
    let children: Vec<_> = tree.root_node().children(&mut cursor).collect();
    let attribute = Regex::new(r"^#\[\s*cfg\s*\(\s*test\s*\)\s*\]$")?;
    let mut output = Vec::new();
    let mut index = 0;
    while index < children.len() {
        let node = children[index];
        let text = &source[node.byte_range()];
        if node.kind() == "attribute_item" && attribute.is_match(text) {
            let mut end = index + 1;
            while end < children.len() && children[end].kind() == "attribute_item" {
                end += 1;
            }
            if end < children.len() && children[end].kind() == "mod_item" {
                let mut module_cursor = children[end].walk();
                if children[end]
                    .children(&mut module_cursor)
                    .any(|child| child.kind() == "declaration_list")
                {
                    index = end + 1;
                    continue;
                }
            }
        }
        output.extend_from_slice(text);
        index += 1;
    }
    Ok(output)
}

struct TemporaryIndex(PathBuf);

impl TemporaryIndex {
    fn new() -> Result<Self> {
        let directory =
            std::env::temp_dir().join(format!("aaqe-index-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self(directory))
    }
}

impl Drop for TemporaryIndex {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub struct PatchGate {
    root: PathBuf,
    test_paths: Vec<Vec<PatternPart>>,
    base_commit: String,
}

#[derive(Clone)]
enum PatternPart {
    AnySequence,
    AnyCharacter,
    Literal(char),
    CharacterSet {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

// Match Python fnmatchcase: slashes are ordinary characters, `**` is `*`,
// and braces are literal. Filesystem glob semantics would widen permissions.
fn parse_pattern(pattern: &str) -> Vec<PatternPart> {
    let characters: Vec<_> = pattern.chars().collect();
    let mut parts = Vec::new();
    let mut index = 0;
    while index < characters.len() {
        match characters[index] {
            '*' => parts.push(PatternPart::AnySequence),
            '?' => parts.push(PatternPart::AnyCharacter),
            '[' => {
                let mut end = index + 1;
                let negated = characters.get(end) == Some(&'!');
                if negated {
                    end += 1;
                }
                if characters.get(end) == Some(&']') {
                    end += 1;
                }
                while end < characters.len() && characters[end] != ']' {
                    end += 1;
                }
                if end == characters.len() {
                    parts.push(PatternPart::Literal('['));
                } else {
                    let mut cursor = index + 1 + usize::from(negated);
                    let mut ranges = Vec::new();
                    while cursor < end {
                        if cursor + 2 < end && characters[cursor + 1] == '-' {
                            ranges.push((characters[cursor], characters[cursor + 2]));
                            cursor += 3;
                        } else {
                            ranges.push((characters[cursor], characters[cursor]));
                            cursor += 1;
                        }
                    }
                    parts.push(PatternPart::CharacterSet { negated, ranges });
                    index = end;
                }
            }
            character => parts.push(PatternPart::Literal(character)),
        }
        index += 1;
    }
    parts
}

fn matches_pattern(pattern: &[PatternPart], path: &str) -> bool {
    let characters: Vec<_> = path.chars().collect();
    let mut previous = vec![false; characters.len() + 1];
    previous[0] = true;
    for part in pattern {
        let mut current = vec![false; previous.len()];
        for index in 0..=characters.len() {
            current[index] = match part {
                PatternPart::AnySequence => previous[index] || (index > 0 && current[index - 1]),
                _ if index == 0 => false,
                PatternPart::AnyCharacter => previous[index - 1],
                PatternPart::Literal(character) => {
                    previous[index - 1] && *character == characters[index - 1]
                }
                PatternPart::CharacterSet { negated, ranges } => {
                    previous[index - 1]
                        && (ranges.iter().any(|(start, end)| {
                            *start <= characters[index - 1] && characters[index - 1] <= *end
                        }) != *negated)
                }
            };
        }
        previous = current;
    }
    previous[characters.len()]
}

impl PatchGate {
    pub fn new(workspace: &Path, test_paths: &[String], base_commit: Option<&str>) -> Result<Self> {
        let root = workspace
            .canonicalize()
            .context("Cannot resolve worktree")?;
        let current = Self::command_at(&root, &["rev-parse", "HEAD"], None);
        ensure!(
            current.returncode == 0,
            "Cannot determine worktree base commit"
        );
        Ok(Self {
            root,
            test_paths: test_paths
                .iter()
                .map(|pattern| parse_pattern(pattern))
                .collect(),
            base_commit: base_commit.unwrap_or(current.stdout.trim()).to_string(),
        })
    }

    fn command_at(root: &Path, arguments: &[&str], input: Option<&str>) -> ProcessResult {
        let command: Vec<String> = std::iter::once("git")
            .chain(arguments.iter().copied())
            .map(str::to_string)
            .collect();
        run(&command, Some(root), 600, input)
    }

    fn command(&self, arguments: &[&str], input: Option<&str>) -> ProcessResult {
        Self::command_at(&self.root, arguments, input)
    }

    fn verify_base(&self) -> Result<()> {
        let current = self.command(&["rev-parse", "HEAD"], None);
        ensure!(
            current.returncode == 0 && current.stdout.trim() == self.base_commit,
            "Worktree base commit differs from the recorded audit version"
        );
        Ok(())
    }

    fn test_only(&self, path: &str) -> bool {
        self.test_paths
            .iter()
            .any(|pattern| matches_pattern(pattern, path))
    }

    pub fn apply(&self, patch: &str) -> Result<Vec<String>> {
        self.verify_base()?;
        ensure!(!patch.trim().is_empty(), "Empty test patch");
        let markers = [
            "GIT binary patch",
            "rename from ",
            "rename to ",
            "old mode ",
            "new mode ",
            "new file mode 120000",
            "new file mode 160000",
            "+++ /dev/null",
        ];
        ensure!(
            !markers.iter().any(|marker| patch.contains(marker)),
            "Deletion, rename, mode, link and binary patches are not allowed"
        );
        let summary = self.command(&["apply", "--numstat", "-z", "-"], Some(patch));
        ensure!(
            summary.returncode == 0,
            "Invalid unified test patch: {}",
            summary.stderr.trim()
        );
        let mut paths = Vec::new();
        for entry in summary.stdout.split('\0').filter(|entry| !entry.is_empty()) {
            let parts: Vec<_> = entry.splitn(3, '\t').collect();
            ensure!(
                parts.len() == 3
                    && !parts[0].is_empty()
                    && parts[0].bytes().all(|byte| byte.is_ascii_digit())
                    && !parts[1].is_empty()
                    && parts[1].bytes().all(|byte| byte.is_ascii_digit()),
                "Unsupported patch format"
            );
            let path = parts[2];
            safe_path(&self.root, path)?;
            ensure!(
                !paths.iter().any(|existing| existing == path),
                "Duplicate patch path"
            );
            paths.push(path.to_string());
        }
        ensure!(!paths.is_empty(), "Patch contains no file changes");
        {
            let temporary = TemporaryIndex::new()?;
            let environment = [(
                "GIT_INDEX_FILE".into(),
                temporary.0.join("index").to_string_lossy().into_owned(),
            )];
            let staged_command = |arguments: &[&str], input| {
                let command: Vec<String> = std::iter::once("git")
                    .chain(arguments.iter().copied())
                    .map(str::to_string)
                    .collect();
                run_env(&command, Some(&self.root), 600, input, &environment)
            };
            let base = staged_command(&["read-tree", &self.base_commit], None);
            let staged = staged_command(
                &["apply", "--cached", "--whitespace=error", "-"],
                Some(patch),
            );
            ensure!(
                base.returncode == 0 && staged.returncode == 0,
                "Patch does not apply to the recorded base: {}",
                staged.stderr.trim()
            );
            for path in &paths {
                if self.test_only(path) {
                    continue;
                }
                let existing = self.root.join(path);
                ensure!(
                    existing.extension().and_then(|value| value.to_str()) == Some("rs")
                        && existing.is_file(),
                    "Patch modifies production code: {path}"
                );
                let updated = staged_command(&["show", &format!(":{path}")], None);
                ensure!(
                    updated.returncode == 0
                        && rust_production(&fs::read(&existing)?)?
                            == rust_production(updated.stdout.as_bytes())?,
                    "Patch modifies production code outside Rust test modules: {path}"
                );
            }
        }
        let check = self.command(&["apply", "--check", "-"], Some(patch));
        ensure!(
            check.returncode == 0,
            "Patch does not apply cleanly: {}",
            check.stderr.trim()
        );
        let applied = self.command(&["apply", "-"], Some(patch));
        ensure!(
            applied.returncode == 0,
            "Patch application failed: {}",
            applied.stderr.trim()
        );
        Ok(paths)
    }

    pub fn verify_worktree(&self) -> Result<Vec<String>> {
        self.verify_base()?;
        let untracked = self.command(&["ls-files", "--others", "--exclude-standard", "-z"], None);
        ensure!(
            untracked.returncode == 0,
            "Cannot verify new worktree files"
        );
        let mut changed = Vec::new();
        for path in untracked.stdout.split('\0').filter(|path| !path.is_empty()) {
            safe_path(&self.root, path)?;
            ensure!(
                self.test_only(path),
                "New production file appeared during execution: {path}"
            );
            changed.push(path.to_string());
        }
        let tracked = self.command(&["diff", "--name-only", "HEAD", "-z"], None);
        ensure!(tracked.returncode == 0, "Cannot verify production worktree");
        for path in tracked.stdout.split('\0').filter(|path| !path.is_empty()) {
            let destination = safe_path(&self.root, path)?;
            ensure!(destination.is_file(), "Tracked file was removed: {path}");
            if !self.test_only(path) {
                let previous = self.command(&["show", &format!("HEAD:{path}")], None);
                ensure!(
                    destination.extension().and_then(|value| value.to_str()) == Some("rs")
                        && previous.returncode == 0
                        && rust_production(previous.stdout.as_bytes())?
                            == rust_production(&fs::read(destination)?)?,
                    "Production code changed during execution: {path}"
                );
            }
            changed.push(path.to_string());
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
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
}
