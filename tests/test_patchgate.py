import difflib

import pytest
from conftest import git

from aaqe.errors import AAQEError
from aaqe.patchgate import PatchGate


def patch(path, before, after):
    return "".join(
        difflib.unified_diff(
            before.splitlines(keepends=True),
            after.splitlines(keepends=True),
            fromfile="a/" + path if before else "/dev/null",
            tofile="b/" + path,
        )
    )


def test_new_test_is_applied_and_production_code_preserved(repository):
    change = patch("tests/test_new.py", "", "def test_new():\n    assert 2 + 2 == 4\n")
    paths = PatchGate(repository).apply(change)
    assert paths == ["tests/test_new.py"]
    assert (repository / "tests/test_new.py").exists()
    assert (repository / "app.py").read_text() == "def value():\n    return 1\n"


def test_production_patch_is_rejected_without_partial_application(repository):
    change = patch("tests/test_new.py", "", "assert True\n")
    change += patch("app.py", "def value():\n    return 1\n", "def value():\n    return 2\n")
    with pytest.raises(AAQEError, match="production"):
        PatchGate(repository).apply(change)
    assert not (repository / "tests/test_new.py").exists()
    assert "return 1" in (repository / "app.py").read_text()


def test_rust_test_module_changes_are_allowed_but_production_change_is_rejected(repository):
    before = "pub fn value() -> u8 { 1 }\n#[cfg(test)]\nmod tests { #[test] fn old() { assert_eq!(super::value(), 1); } }\n"
    path = repository / "src/lib.rs"
    path.parent.mkdir()
    path.write_text(before)
    git(repository, "add", ".")
    git(repository, "commit", "-m", "rust fixture")
    after = before.replace("fn old()", "fn regression()")
    PatchGate(repository).apply(patch("src/lib.rs", before, after))
    assert "fn regression()" in path.read_text()
    path.write_text(before)
    with pytest.raises(AAQEError, match="production"):
        PatchGate(repository).apply(patch("src/lib.rs", before, after.replace("{ 1 }", "{ 2 }")))


def test_fake_rust_test_marker_in_string_does_not_authorize_changes(repository):
    before = 'pub const TEXT: &str = "#[cfg(test)] mod tests {}";\n'
    (repository / "lib.rs").write_text(before)
    git(repository, "add", ".")
    git(repository, "commit", "-m", "string fixture")
    with pytest.raises(AAQEError):
        PatchGate(repository).apply(patch("lib.rs", before, before.replace("TEXT", "OTHER")))


def test_test_path_symlink_is_rejected(repository, tmp_path):
    outside = tmp_path / "outside"
    outside.mkdir()
    (repository / "tests/link").symlink_to(outside, target_is_directory=True)
    with pytest.raises(AAQEError):
        PatchGate(repository).apply(patch("tests/link/escaped.py", "", "escaped = True\n"))
    assert not (outside / "escaped.py").exists()


def test_traversal_and_git_metadata_are_rejected(repository):
    for path in ("../escaped.py", ".git/config", "tests/../../escaped.py"):
        with pytest.raises(AAQEError):
            PatchGate(repository).apply(patch(path, "", "escaped = True\n"))


def test_existing_test_removal_is_rejected(repository):
    change = "--- a/tests/test_app.py\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-def test_value():\n-    assert True\n"
    with pytest.raises(AAQEError):
        PatchGate(repository).apply(change)


def test_untracked_production_file_is_rejected(repository):
    (repository / "new_production.py").write_text("value = 1\n")
    with pytest.raises(AAQEError, match="production"):
        PatchGate(repository).verify_worktree()
