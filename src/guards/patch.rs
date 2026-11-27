use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};

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
    expected_patch: Option<String>,
    base_commit: String,
}

impl PatchGate {
    pub fn new(workspace: &Path, base_commit: Option<&str>) -> Result<Self> {
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
            expected_patch: None,
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

    /// The accepted cumulative patch; an empty string requires the recorded base.
    pub fn with_patch(mut self, patch: &str) -> Self {
        self.expected_patch = Some(patch.to_owned());
        self
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
            changed.push(path.to_string());
        }
        let tracked = self.command(&["diff", "--name-only", "HEAD", "-z"], None);
        ensure!(tracked.returncode == 0, "Cannot verify production worktree");
        for path in tracked.stdout.split('\0').filter(|path| !path.is_empty()) {
            let destination = safe_path(&self.root, path)?;
            ensure!(destination.is_file(), "Tracked file was removed: {path}");
            changed.push(path.to_string());
        }
        if let Some(patch) = &self.expected_patch {
            let temporary = TemporaryIndex::new()?;
            let environment = [(
                "GIT_INDEX_FILE".into(),
                temporary.0.join("index").display().to_string(),
            )];
            let indexed = |args: &[&str], input| {
                let argv = std::iter::once("git")
                    .chain(args.iter().copied())
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                run_env(&argv, Some(&self.root), 600, input, &environment)
            };
            ensure!(
                indexed(&["read-tree", &self.base_commit], None).returncode == 0,
                "Cannot read recorded base"
            );
            if !patch.is_empty() {
                ensure!(
                    indexed(&["apply", "--cached", "-"], Some(patch)).returncode == 0,
                    "Accepted patch no longer applies to recorded base"
                );
            }
            let drift = indexed(&["diff", "--exit-code", "--no-ext-diff", "--", "."], None);
            let extra = indexed(&["ls-files", "--others", "--exclude-standard", "-z"], None);
            ensure!(
                drift.returncode == 0 && extra.returncode == 0 && extra.stdout.is_empty(),
                "Worktree differs from the accepted patch (unaccepted changes)"
            );
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
        PatchGate::new(root, None).unwrap()
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
    fn tests_in_nonstandard_paths_and_embedded_source_are_allowed() {
        let repo = repository();
        let change = patch("checks/value_case.go", "", "package checks\n")
            + &patch(
                "app.rs",
                "pub fn value() -> i32 { 1 }\n",
                "pub fn value() -> i32 { 1 }\n#[test] fn inline() { assert_eq!(value(), 1); }\n",
            );
        gate(repo.path()).apply(&change).unwrap();
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
    fn accepted_patch_detects_tracked_and_untracked_drift() {
        let repo = repository();
        let change = patch("checks/new.rs", "", "#[test] fn test() {}\n");
        let gate = gate(repo.path()).with_patch(&change);
        gate.apply(&change).unwrap();
        gate.verify_worktree().unwrap();
        fs::write(repo.path().join("app.rs"), "changed\n").unwrap();
        assert!(
            gate.verify_worktree()
                .unwrap_err()
                .to_string()
                .contains("accepted patch")
        );
        git(repo.path(), &["checkout", "--", "app.rs"]);
        fs::write(repo.path().join("extra.rs"), "extra\n").unwrap();
        assert!(gate.verify_worktree().is_err());
        fs::remove_file(repo.path().join("extra.rs")).unwrap();
        fs::remove_file(repo.path().join("checks/new.rs")).unwrap();
        assert!(gate.verify_worktree().is_err());
    }
}
