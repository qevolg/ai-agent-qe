use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use regex::bytes::Regex;

use crate::process::{ProcessResult, run, run_env};

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
