import fnmatch
import os
import re
import tempfile
from pathlib import Path, PurePosixPath

import tree_sitter_rust
from tree_sitter import Language, Parser

from .config import Config
from .errors import AAQEError
from .process import run


def safe_path(root, relative):
    path = PurePosixPath(relative)
    if (
        not relative
        or path.is_absolute()
        or "\\" in relative
        or any(part in ("..", ".git") or ":" in part for part in path.parts)
    ):
        raise AAQEError(f"Unsafe patch path: {relative}")
    destination = Path(root) / relative
    for parent in [destination, *destination.parents]:
        if parent == Path(root):
            break
        if parent.is_symlink():
            raise AAQEError(f"Symbolic link in patch path: {relative}")
    if not destination.resolve().is_relative_to(Path(root).resolve()):
        raise AAQEError(f"Path escapes worktree: {relative}")
    return destination


def rust_production(source):
    parser = Parser(Language(tree_sitter_rust.language()))
    tree = parser.parse(source)
    if tree.root_node.has_error:
        raise AAQEError("Cannot verify production boundary in invalid Rust syntax")
    children = tree.root_node.children
    output = []
    index = 0
    while index < len(children):
        node = children[index]
        text = source[node.start_byte : node.end_byte]
        if node.type == "attribute_item" and re.fullmatch(
            rb"#\[\s*cfg\s*\(\s*test\s*\)\s*\]", text
        ):
            end = index + 1
            while end < len(children) and children[end].type == "attribute_item":
                end += 1
            if end < len(children) and children[end].type == "mod_item":
                if any(child.type == "declaration_list" for child in children[end].children):
                    index = end + 1
                    continue
        output.append(text)
        index += 1
    return b"".join(output)


class PatchGate:
    def __init__(self, root, test_paths=None, *, base_commit=None):
        self.root = Path(root).resolve()
        self.test_paths = test_paths if test_paths is not None else Config().test_paths
        current = run(["git", "rev-parse", "HEAD"], cwd=self.root)
        if current.returncode:
            raise AAQEError("Cannot determine worktree base commit")
        self.base_commit = base_commit or current.stdout.strip()

    def verify_base(self):
        current = run(["git", "rev-parse", "HEAD"], cwd=self.root)
        if current.returncode or current.stdout.strip() != self.base_commit:
            raise AAQEError("Worktree base commit differs from the recorded audit version")

    def test_only(self, path):
        return any(fnmatch.fnmatchcase(path, pattern) for pattern in self.test_paths)

    def apply(self, patch):
        self.verify_base()
        if not patch.strip():
            raise AAQEError("Empty test patch")
        if any(
            marker in patch
            for marker in (
                "GIT binary patch",
                "rename from ",
                "rename to ",
                "old mode ",
                "new mode ",
                "new file mode 120000",
                "new file mode 160000",
                "+++ /dev/null",
            )
        ):
            raise AAQEError("Deletion, rename, mode, link and binary patches are not allowed")
        summary = run(["git", "apply", "--numstat", "-z", "-"], cwd=self.root, input_text=patch)
        if summary.returncode:
            raise AAQEError("Invalid unified test patch: " + summary.stderr.strip())
        paths = []
        for entry in summary.stdout.split("\0"):
            if not entry:
                continue
            parts = entry.split("\t", 2)
            if len(parts) != 3 or not parts[0].isdigit() or not parts[1].isdigit():
                raise AAQEError("Unsupported patch format")
            path = parts[2]
            safe_path(self.root, path)
            if path in paths:
                raise AAQEError("Duplicate patch path")
            paths.append(path)
        if not paths:
            raise AAQEError("Patch contains no file changes")
        with tempfile.TemporaryDirectory(prefix="aaqe-index-") as temporary:
            environment = {**os.environ, "GIT_INDEX_FILE": str(Path(temporary) / "index")}
            base = run(["git", "read-tree", self.base_commit], cwd=self.root, env=environment)
            staged = run(
                ["git", "apply", "--cached", "--whitespace=error", "-"],
                cwd=self.root,
                env=environment,
                input_text=patch,
            )
            if base.returncode or staged.returncode:
                raise AAQEError(
                    "Patch does not apply to the recorded base: " + staged.stderr.strip()
                )
            for path in paths:
                if self.test_only(path):
                    continue
                existing = self.root / path
                if existing.suffix != ".rs" or not existing.is_file():
                    raise AAQEError(f"Patch modifies production code: {path}")
                updated = run(["git", "show", ":" + path], cwd=self.root, env=environment)
                if updated.returncode or rust_production(existing.read_bytes()) != rust_production(
                    updated.stdout.encode()
                ):
                    raise AAQEError(
                        f"Patch modifies production code outside Rust test modules: {path}"
                    )
        check = run(["git", "apply", "--check", "-"], cwd=self.root, input_text=patch)
        if check.returncode:
            raise AAQEError("Patch does not apply cleanly: " + check.stderr.strip())
        applied = run(["git", "apply", "-"], cwd=self.root, input_text=patch)
        if applied.returncode:
            raise AAQEError("Patch application failed: " + applied.stderr.strip())
        return paths

    def verify_worktree(self):
        self.verify_base()
        untracked = run(["git", "ls-files", "--others", "--exclude-standard", "-z"], cwd=self.root)
        if untracked.returncode:
            raise AAQEError("Cannot verify new worktree files")
        for path in untracked.stdout.split("\0"):
            if path:
                safe_path(self.root, path)
                if not self.test_only(path):
                    raise AAQEError(f"New production file appeared during execution: {path}")
        tracked = run(["git", "diff", "--name-only", "HEAD", "-z"], cwd=self.root)
        if tracked.returncode:
            raise AAQEError("Cannot verify production worktree")
        for path in tracked.stdout.split("\0"):
            if not path:
                continue
            destination = safe_path(self.root, path)
            if not destination.is_file():
                raise AAQEError(f"Tracked file was removed: {path}")
            if self.test_only(path):
                continue
            previous = run(["git", "show", "HEAD:" + path], cwd=self.root)
            if (
                destination.suffix != ".rs"
                or previous.returncode
                or rust_production(previous.stdout.encode())
                != rust_production(destination.read_bytes())
            ):
                raise AAQEError(f"Production code changed during execution: {path}")
