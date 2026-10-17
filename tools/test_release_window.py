"""Tests of the release-window check (tools/release_window.py).

They build small real repositories in a temporary folder with the same shape as the
release process (a release branch cut from `main`, a window fix on it, a counterpart on
`main` made later against a `main` that moved on) and run the real git. Nothing is
mocked, and nothing leaves the temporary folder.
"""

from __future__ import annotations

from pathlib import Path
import re
import subprocess
import tempfile
import unittest

import release_window as window

REPOSITORY = Path(__file__).resolve().parent.parent
WORKFLOW = REPOSITORY / ".github" / "workflows" / "release-window.yml"

BASE_FILES = {
    "crates/engine/src/lib.rs": "fn alpha() {}\nfn beta() {\n    retries = 3;\n}\nfn gamma() {}\n",
    "Cargo.toml": 'name = "x"\nversion = "0.2.2-rc.1"\nvsift = { version = "0.2.2-rc.1", path = "crates/vsift" }\nserde = { version = "1.0.1" }\n',
    "Cargo.lock": '[[package]]\nname = "vsift"\nversion = "0.2.2-rc.1"\n[[package]]\nname = "serde"\nversion = "1.0.1"\nchecksum = "aaaa1111"\n',
    "CHANGELOG.md": "# Changelog\n\n## Unreleased\n",
    "tools/helper.py": "def keep():\n    return 1\n",
    "assets/logo.bin": "BINARY-1",
}


class Repository:
    """A throwaway repository with `main` and a release branch cut from it."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.git("init", "--quiet", "--initial-branch=main")
        for path, content in BASE_FILES.items():
            self.write(path, content)
        self.commit("The base (#1)")
        self.git("branch", "release/0.2.x")

    def git(self, *arguments: str) -> str:
        completed = subprocess.run(
            [
                "git",
                "-C",
                str(self.root),
                "-c",
                "user.name=Tester",
                "-c",
                "user.email=tester@invalid",
                "-c",
                "commit.gpgsign=false",
                *arguments,
            ],
            check=True,
            capture_output=True,
            text=True,
        )
        return completed.stdout

    def write(self, path: str, content: str) -> None:
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content.encode("utf-8"))

    def edit(self, path: str, old: str, new: str) -> None:
        content = (self.root / path).read_text(encoding="utf-8")
        self.write(path, content.replace(old, new))

    def commit(self, subject: str) -> None:
        self.git("add", "--all")
        self.git("commit", "--quiet", "-m", subject)

    def on(self, branch: str) -> None:
        self.git("switch", "--quiet", branch)

    def check(self, *, records: bool = False) -> tuple[int, str]:
        output: list[str] = []
        code = window.run(
            self.root, "main", ["release/0.2.x"], records, output.append
        )
        return code, "\n".join(output)


class WindowFixTests(unittest.TestCase):
    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.addCleanup(self._directory.cleanup)
        self.repo = Repository(Path(self._directory.name))

    def release_fix(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit("crates/engine/src/lib.rs", "retries = 3;", "retries = 5;")
        self.repo.commit("Retry five times (#10)")
        self.repo.on("main")

    def test_a_fix_missing_from_main_is_reported_and_fails(self) -> None:
        self.release_fix()
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn("NOT ON MAIN", output)
        self.assertIn("Retry five times (#10)", output)
        self.assertIn("retries = 5;", output)
        self.assertIn("still on main", output)

    def test_a_counterpart_made_against_a_main_that_moved_on_counts(self) -> None:
        self.release_fix()
        # Main moves on first, so the counterpart's context and line numbers differ from
        # the release commit's: a patch-id would not match, the content does.
        self.repo.edit("crates/engine/src/lib.rs", "fn alpha() {}", "fn alpha() {}\nfn delta() {}\nfn epsilon() {}")
        self.repo.commit("Unrelated work (#11)")
        self.repo.edit("crates/engine/src/lib.rs", "retries = 3;", "retries = 5;")
        self.repo.commit("The same fix on main (#12)")
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_OK, output)
        self.assertIn("on main", output)

    def test_a_counterpart_squashed_from_two_fixes_counts(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit("crates/engine/src/lib.rs", "retries = 3;", "retries = 5;")
        self.repo.commit("Retry five times (#10)")
        self.repo.edit("tools/helper.py", "return 1", "return 2")
        self.repo.commit("Helper returns two (#13)")
        self.repo.on("main")
        self.repo.edit("crates/engine/src/lib.rs", "retries = 3;", "retries = 5;")
        self.repo.edit("tools/helper.py", "return 1", "return 2")
        self.repo.commit("Both fixes on main (#14)")
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_OK, output)

    def test_only_the_missing_commit_of_two_is_named(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit("crates/engine/src/lib.rs", "retries = 3;", "retries = 5;")
        self.repo.commit("Retry five times (#10)")
        self.repo.edit("tools/helper.py", "return 1", "return 2")
        self.repo.commit("Helper returns two (#13)")
        self.repo.on("main")
        self.repo.edit("crates/engine/src/lib.rs", "retries = 3;", "retries = 5;")
        self.repo.commit("Only the first on main (#14)")
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_MISSING, output)
        missing = [line for line in output.splitlines() if "NOT ON MAIN" in line]
        self.assertEqual(len(missing), 1, output)
        self.assertIn("Helper returns two (#13)", missing[0])

    def test_a_removed_line_still_on_main_is_reported(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit("crates/engine/src/lib.rs", "    retries = 3;\n", "")
        self.repo.commit("Drop the retry (#10)")
        self.repo.on("main")
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn("removed line(s) still on main", output)

    def test_a_deleted_file_still_on_main_is_reported(self) -> None:
        self.repo.on("release/0.2.x")
        (self.repo.root / "tools/helper.py").unlink()
        self.repo.commit("Delete the helper (#10)")
        self.repo.on("main")
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn("deleted on the release branch, still on main", output)

    def test_a_changed_binary_file_must_be_identical(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.write("assets/logo.bin", "BINARY-2\0")
        self.repo.commit("New logo (#10)")
        self.repo.on("main")
        code, _ = self.repo.check()
        self.assertEqual(code, window.EXIT_MISSING)
        self.repo.write("assets/logo.bin", "BINARY-2\0")
        self.repo.commit("New logo on main (#12)")
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_OK, output)

    def test_a_version_bump_alone_is_not_a_missing_fix(self) -> None:
        self.repo.on("release/0.2.x")
        for path in ("Cargo.toml", "Cargo.lock"):
            self.repo.edit(path, "0.2.2-rc.1", "0.2.2-rc.2")
        self.repo.commit("The second candidate's version (#10)")
        self.repo.on("main")
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_OK, output)

    def test_a_third_party_dependency_change_in_a_version_file_is_not_hidden(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit("Cargo.toml", 'serde = { version = "1.0.1" }', 'serde = { version = "1.0.9" }')
        self.repo.edit("Cargo.lock", 'version = "1.0.1"\nchecksum = "aaaa1111"', 'version = "1.0.9"\nchecksum = "bbbb2222"')
        self.repo.commit("Bump serde (#10)")
        self.repo.on("main")
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn("Cargo.toml", output)
        self.assertIn("checksum", output)

    def test_the_work_record_is_required_only_with_records(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit("CHANGELOG.md", "## Unreleased", "## Unreleased\n\n- A window note that only the release branch has.")
        self.repo.commit("Records (#10)")
        self.repo.on("main")
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_OK, output)
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn("CHANGELOG.md (record)", output)

    def test_a_fix_with_its_changelog_entry_passes_when_main_words_it_differently(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit("crates/engine/src/lib.rs", "retries = 3;", "retries = 5;")
        self.repo.edit("CHANGELOG.md", "## Unreleased", "## Unreleased\n\n- Retries are five.")
        self.repo.commit("Retry five times (#10)")
        self.repo.on("main")
        self.repo.edit("crates/engine/src/lib.rs", "retries = 3;", "retries = 5;")
        self.repo.edit("CHANGELOG.md", "## Unreleased", "## Unreleased\n\n- Five retries now.")
        self.repo.commit("The counterpart (#12)")
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_OK, output)
        self.assertIn("work record still to be brought", output)

    def test_no_release_branch_is_not_a_failure(self) -> None:
        output: list[str] = []
        code = window.run(self.repo.root, "main", [], False, output.append)
        self.assertEqual(code, window.EXIT_OK)
        self.assertIn("nothing to compare", output[0])

    def test_a_branch_with_no_commit_since_the_cut_passes(self) -> None:
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_OK, output)
        self.assertIn("0 commit(s) since the cut", output)

    def test_a_file_line_cannot_start_a_workflow_command(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit("tools/helper.py", "return 1", "::error::injected")
        self.repo.commit("Hostile line (#10)")
        self.repo.on("main")
        _, output = self.repo.check()
        self.assertIn("injected", output)
        self.assertFalse([line for line in output.splitlines() if line.startswith("::")], output)

    def test_unrelated_histories_are_an_error_not_a_pass(self) -> None:
        self.repo.git("switch", "--quiet", "--orphan", "release/other")
        self.repo.write("orphan.txt", "orphaned content\n")
        self.repo.commit("Orphan (#10)")
        self.repo.on("main")
        with self.assertRaises(window.GitError):
            window.run(self.repo.root, "main", ["release/other"], False, lambda _: None)

    def test_branches_are_discovered_by_their_remote_ref(self) -> None:
        self.repo.git("update-ref", "refs/remotes/origin/release/0.2.x", "release/0.2.x")
        self.repo.git("update-ref", "refs/remotes/origin/release/0.3.x", "release/0.2.x")
        self.repo.git("update-ref", "refs/remotes/origin/main", "main")
        self.assertEqual(
            window.release_branches(self.repo.root, "refs/remotes/origin/release/*"),
            ["refs/remotes/origin/release/0.2.x", "refs/remotes/origin/release/0.3.x"],
        )


class ClassificationTests(unittest.TestCase):
    def test_kinds(self) -> None:
        self.assertEqual(window.kind_of("crates/vsift/src/lib.rs"), "fix")
        self.assertEqual(window.kind_of(".github/workflows/ci.yml"), "fix")
        self.assertEqual(window.kind_of("docs/operations/release.md"), "fix")
        self.assertEqual(window.kind_of("CHANGELOG.md"), "record")
        self.assertEqual(window.kind_of("memory/TODO.md"), "record")
        self.assertEqual(window.kind_of("docs/planning/p14-evidence-ledger.json"), "record")
        self.assertEqual(window.kind_of("docs/operations/install.md"), "record")
        self.assertEqual(window.kind_of("npm/vsift-cli/README.md"), "record")

    def test_only_version_keys_and_vsift_lines_lose_their_version(self) -> None:
        self.assertEqual(window.normalise("Cargo.toml", '  version = "0.2.2-rc.1"'), 'version = "<V>"')
        self.assertEqual(
            window.normalise("Cargo.toml", 'vsift = { version = "0.2.2", path = "x" }'),
            'vsift = { version = "<V>", path = "x" }',
        )
        self.assertEqual(
            window.normalise("npm/vsift-cli/package.json", '"@vsift/linux-x64": "0.2.2-rc.1",'),
            '"@vsift/linux-x64": "<V>",',
        )
        self.assertEqual(
            window.normalise("Cargo.toml", 'serde = { version = "1.0.1" }'),
            'serde = { version = "1.0.1" }',
        )
        self.assertEqual(
            window.normalise("crates/x/src/lib.rs", 'const V: &str = "0.2.2";'),
            'const V: &str = "0.2.2";',
        )

    def test_a_content_line_that_looks_like_a_header_is_content(self) -> None:
        diff = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n---- old\n++++ new\n"
        self.assertEqual(window.parse_hunks(diff), (["+++ new"], ["--- old"]))


def wiring_problems(workflow: str) -> list[str]:
    """What is wrong with the workflow that runs the check, as a list of plain sentences.

    The check is only worth having if the workflow really fetches the release branch's
    history and runs the script on the records change, so those facts are held here.
    """
    problems: list[str] = []
    if "fetch-depth: 0" not in workflow:
        problems.append("the checkout does not fetch the full history")
    if "python3 tools/release_window.py" not in workflow:
        problems.append("the script is not run")
    if "--records" not in workflow:
        problems.append("the records change does not require the work record")
    if not re.search(r"pull_request:\s+branches:\s+- main", workflow):
        problems.append("the records change (a pull request to main) does not run it")
    if "docs/planning/p14-evidence-ledger.json" not in workflow:
        problems.append("the records change is not recognised by its evidence ledger")
    if "workflow_dispatch" not in workflow:
        problems.append("it cannot be run on demand before a cut")
    if re.search(r"^permissions:\s*\n\s+contents: read\s*$", workflow, re.MULTILINE) is None:
        problems.append("the permissions are not exactly contents: read")
    if "continue-on-error" in workflow:
        problems.append("a failure would not fail the run")
    return problems


class WorkflowWiringTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_the_workflow_is_wired_to_the_script(self) -> None:
        self.assertEqual(wiring_problems(self.workflow), [])

    def test_a_broken_copy_of_the_workflow_is_noticed(self) -> None:
        mutations = {
            "no full history": ("fetch-depth: 0", "fetch-depth: 1"),
            "script not run": ("python3 tools/release_window.py", "echo skipped"),
            "records not required": ("--records", ""),
            "no pull request trigger": ("  pull_request:\n    branches:\n      - main", "  push:\n    branches:\n      - main"),
            "ledger path dropped": ("docs/planning/p14-evidence-ledger.json", "docs/planning/other.json"),
            "no dispatch": ("workflow_dispatch", "workflow_call"),
            "wider permissions": ("contents: read", "contents: write"),
            "failure ignored": ("    steps:", "    continue-on-error: true\n    steps:"),
        }
        for name, (old, new) in mutations.items():
            with self.subTest(name):
                self.assertIn(old, self.workflow, f"the mutation `{name}` no longer applies")
                broken = self.workflow.replace(old, new)
                self.assertNotEqual(wiring_problems(broken), [], name)


if __name__ == "__main__":
    unittest.main()
