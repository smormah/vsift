"""Tests of the release-window check (tools/release_window.py).

They build small real repositories in a temporary folder with the same shape as the
release process (a release branch cut from `main`, a window fix on it, a counterpart on
`main` made later against a `main` that moved on) and run the real git. Nothing is
mocked, and nothing leaves the temporary folder.
"""

from __future__ import annotations

import os
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
    "memory/TODO.md": "# Work record\n\n## Now (2026-10-17): the candidate is cut\n",
    "memory/project_current_status.md": "As of 2026-10-17: the candidate is cut.\n",
    "docs/planning/known-limits.md": (
        "# Register\n\nDate: 2026-10-17 (the cut)\n\n"
        "| [L-167](#l-167) | Journeys fail on Windows | open |\n"
        "Counts: 1 high, 50 medium (157 entries).\n\n"
        "### L-167\n\n- **Next step:** release the fix.\n"
    ),
    "docs/operations/install.md": "Install guide.\n\nUse the latest version.\n",
    "docs/planning/p14-agent-trials/README.md": "# Trials\n\nThe latest trial was frozen on `aaaa1111`.\n",
    "docs/planning/p14-agent-trials/batch-2-0.2.2-rc.1/freeze.json": '{\n  "freeze": "aaaa1111"\n}\n',
    "docs/planning/p14-agent-trials/batch-2-0.2.2-rc.1/records/run-1.json": '{\n  "freeze": "aaaa1111"\n}\n',
    "docs/operations/p14-agent-trials/reading.md": "# Reading\n\nThe reading was made on freeze `aaaa1111`.\n",
}


# Committer times for the commits of the window tests (the release commit, then `main`'s rewrite).
BASE_TIME = "2026-10-10T00:00:00+00:00"
RELEASE_TIME = "2026-10-18T10:00:00+00:00"
MAIN_LATER = "2026-10-18T12:00:00+00:00"
MAIN_EARLIER = "2026-10-17T12:00:00+00:00"


class Repository:
    """A throwaway repository with `main` and a release branch cut from it."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.git("init", "--quiet", "--initial-branch=main")
        for path, content in BASE_FILES.items():
            self.write(path, content)
        self.commit("The base (#1)", BASE_TIME)
        self.git("branch", "release/0.2.x")

    def git(self, *arguments: str, date: str | None = None) -> str:
        environment = None
        if date is not None:
            environment = {**os.environ, "GIT_COMMITTER_DATE": date, "GIT_AUTHOR_DATE": date}
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
            env=environment,
        )
        return completed.stdout

    def write(self, path: str, content: str) -> None:
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content.encode("utf-8"))

    def edit(self, path: str, old: str, new: str) -> None:
        content = (self.root / path).read_text(encoding="utf-8")
        self.write(path, content.replace(old, new))

    def commit(self, subject: str, date: str | None = None) -> None:
        self.git("add", "--all")
        self.git("commit", "--quiet", "-m", subject, date=date)

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


class ExcusedLineTests(unittest.TestCase):
    """The excuses of L-173 / #458, built from the shapes #457's records change showed.

    Each excuse has a test that it holds, and tests that the strictness it must not lose still
    holds: a window fix missing from `main` is reported whatever else is excused.
    """

    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.addCleanup(self._directory.cleanup)
        self.repo = Repository(Path(self._directory.name))

    def release(self, subject: str, date: str = RELEASE_TIME) -> None:
        self.repo.commit(subject, date)

    def main_commit(self, subject: str, date: str = MAIN_LATER) -> None:
        self.repo.commit(subject, date)

    # Excuse 1: a line a later window commit replaced.

    def replaced_in_the_window(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit("crates/engine/src/lib.rs", "retries = 3;", "retries = 4;")
        self.release("Retry four times (#10)")
        self.repo.edit("crates/engine/src/lib.rs", "retries = 4;", "retries = 5;")
        self.release("Retry five times, not four (#11)")
        self.repo.on("main")

    def test_a_line_that_a_later_window_commit_replaced_is_not_reported(self) -> None:
        self.replaced_in_the_window()
        self.repo.edit("crates/engine/src/lib.rs", "retries = 3;", "retries = 5;")
        self.main_commit("The net fix on main (#12)")
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_OK, output)
        self.assertIn(
            "excused, crates/engine/src/lib.rs: 1 added line(s) replaced by a later window commit",
            output,
        )

    def test_the_net_change_of_the_branch_is_still_required_of_main(self) -> None:
        self.replaced_in_the_window()
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn("retries = 5;", output)

    def test_a_replaced_line_that_main_took_in_the_first_form_only_is_reported(self) -> None:
        self.replaced_in_the_window()
        self.repo.edit("crates/engine/src/lib.rs", "retries = 3;", "retries = 4;")
        self.main_commit("The first form on main (#12)")
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn("Retry five times, not four (#11)", output)
        self.assertIn("removed line(s) still on main", output)

    def test_a_line_that_the_head_still_has_is_never_called_superseded(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit("tools/helper.py", "return 1", "return 2")
        self.release("Helper returns two (#10)")
        self.repo.on("main")
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertNotIn("replaced by a later window commit", output)

    # Excuse 2: the files rewritten as current state.

    def rewrite_state_on_the_branch(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit("memory/TODO.md", "the candidate is cut", "the candidate is published")
        self.repo.edit(
            "memory/project_current_status.md", "the candidate is cut", "the candidate is published"
        )
        self.release("Records for the candidate (#10)")
        self.repo.on("main")

    def rewrite_state_on_main(self, date: str) -> None:
        self.repo.edit("memory/TODO.md", "the candidate is cut", "the stable release is next")
        self.repo.edit(
            "memory/project_current_status.md", "the candidate is cut", "the stable release is next"
        )
        self.main_commit("Records on main (#12)", date)

    def test_a_current_state_file_that_main_rewrote_later_is_satisfied(self) -> None:
        self.rewrite_state_on_the_branch()
        self.rewrite_state_on_main(MAIN_LATER)
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_OK, output)
        self.assertIn(
            "memory/TODO.md: the handoff, rewritten as current state by every change; "
            "main rewrote it on 2026-10-18",
            output,
        )
        self.assertIn("memory/project_current_status.md", output)

    def test_a_current_state_file_that_main_rewrote_before_the_branch_is_not(self) -> None:
        self.rewrite_state_on_the_branch()
        self.rewrite_state_on_main(MAIN_EARLIER)
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn("memory/TODO.md (record): 1 added line(s) not on main", output)

    def test_a_current_state_file_that_main_never_rewrote_is_not(self) -> None:
        self.rewrite_state_on_the_branch()
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn("memory/project_current_status.md (record)", output)

    def test_only_the_named_current_state_files_are_excused(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.write("memory/notes.md", "A note the branch wrote.\n")
        self.release("A note (#10)")
        self.repo.on("main")
        self.repo.write("memory/notes.md", "A different note.\n")
        self.main_commit("Another note (#12)")
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn("memory/notes.md (record)", output)

    def test_a_current_state_rewrite_cannot_hide_a_missing_code_fix(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit("crates/engine/src/lib.rs", "retries = 3;", "retries = 5;")
        self.repo.edit("memory/TODO.md", "the candidate is cut", "a window fix is in")
        self.release("Retry five times and say so (#10)")
        self.repo.on("main")
        self.repo.edit("memory/TODO.md", "the candidate is cut", "main moved on")
        self.main_commit("Records on main (#12)")
        for records in (False, True):
            with self.subTest(records=records):
                code, output = self.repo.check(records=records)
                self.assertEqual(code, window.EXIT_MISSING, output)
                self.assertIn(
                    "crates/engine/src/lib.rs (fix): 1 added line(s) not on main", output
                )
                self.assertIn("memory/TODO.md: the handoff", output)

    # Excuse 3: a line rewritten in place on both sides (the register, the plan pages, the changelog).

    def test_the_registers_date_and_counts_lines_rewritten_on_main_are_satisfied(self) -> None:
        register = "docs/planning/known-limits.md"
        self.repo.on("release/0.2.x")
        self.repo.edit(register, "Date: 2026-10-17 (the cut)", "Date: 2026-10-18 (the branch's records)")
        self.repo.edit(
            register,
            "Counts: 1 high, 50 medium (157 entries).",
            "Counts: 1 high, 51 medium (159 entries).",
        )
        self.release("Records for the candidate (#10)")
        self.repo.on("main")
        self.repo.edit(register, "Date: 2026-10-17 (the cut)", "Date: 2026-10-18 (the records change)")
        self.repo.edit(
            register,
            "Counts: 1 high, 50 medium (157 entries).",
            "Counts: 1 high, 50 medium (158 entries).",
        )
        self.main_commit("Records on main (#12)")
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_OK, output)
        self.assertIn("docs/planning/known-limits.md: 2 added line(s) rewritten on main", output)

    def test_a_register_line_that_main_left_as_the_cut_had_it_is_reported(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit(
            "docs/planning/known-limits.md",
            "- **Next step:** release the fix.",
            "- **Next step:** the fix is released.",
        )
        self.release("Records for the candidate (#10)")
        self.repo.on("main")
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn(
            "docs/planning/known-limits.md (record): 1 added line(s) not on main, "
            "1 removed line(s) still on main",
            output,
        )

    def test_a_register_entry_that_main_does_not_have_is_reported(self) -> None:
        register = "docs/planning/known-limits.md"
        self.repo.on("release/0.2.x")
        self.repo.edit(
            register, "### L-167", "### L-180\n\n- **Next step:** a new limit of the window fix.\n\n### L-167"
        )
        self.repo.edit(
            register,
            "| [L-167](#l-167) |",
            "| [L-180](#l-180) | A new limit | open |\n| [L-167](#l-167) |",
        )
        self.release("A new limit (#10)")
        self.repo.on("main")
        # Main added another entry meanwhile, with a bullet of the same shape.
        self.repo.edit(
            register, "### L-167", "### L-181\n\n- **Next step:** another limit.\n\n### L-167"
        )
        self.main_commit("Another entry on main (#12)")
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn("### L-180", output)
        self.assertIn("[L-180](#l-180)", output)

    def test_a_reworded_changelog_sentence_passes_and_a_missing_one_is_reported(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit(
            "CHANGELOG.md",
            "## Unreleased",
            "## Unreleased\n\n"
            "- **Nothing had been run against this release's own bytes when this was written.** Not yet.\n"
            "- **Retries are five now, for every provider call.** A window fix.",
        )
        self.release("The stable commit (#10)")
        self.repo.on("main")
        self.repo.edit(
            "CHANGELOG.md",
            "## Unreleased",
            "## Unreleased\n\n"
            "- **Nothing had been run against this release's own bytes when this was written.** Run on the 18th.",
        )
        self.main_commit("Records on main (#12)")
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn("CHANGELOG.md (record): 1 added line(s) not on main", output)
        self.assertIn("Retries are five now", output)
        quoted = [line for line in output.splitlines() if line.startswith("    >")]
        self.assertFalse([line for line in quoted if "Nothing had been run" in line], output)

    def test_a_reworded_changelog_sentence_alone_passes(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit("CHANGELOG.md", "## Unreleased", "## Unreleased\n\n- **Not published, not tagged and not announced by this change.** The release is cut here.")
        self.release("The stable commit (#10)")
        self.repo.on("main")
        self.repo.edit("CHANGELOG.md", "## Unreleased", "## Unreleased\n\n- **Not published, not tagged and not announced by this change.** It was published on the 18th.")
        self.main_commit("Records on main (#12)")
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_OK, output)
        self.assertIn("CHANGELOG.md: 1 added line(s) rewritten on main", output)

    def test_a_line_that_main_still_has_as_the_cut_had_it_is_not_a_rewrite(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit("CHANGELOG.md", "# Changelog", "# Changelog (as released)")
        self.release("A heading (#10)")
        self.repo.on("main")
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_MISSING, output)

    # Excuse 3 for the trial records (#481, run 39237738669): the reading and README pages name the
    # trial freeze, and a later freeze made `main` rewrite those lines while the branch kept its own.

    def freeze_rewritten_on_both_sides(self, path: str, release_text: str, main_text: str, old: str) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit(path, old, release_text)
        self.release("The candidate's trial records (#10)")
        self.repo.on("main")
        self.repo.edit(path, old, main_text)
        self.main_commit("The new trial freeze on main (#12)")

    def test_a_trial_page_naming_a_freeze_that_main_rewrote_is_satisfied(self) -> None:
        page = "docs/planning/p14-agent-trials/README.md"
        self.freeze_rewritten_on_both_sides(
            page,
            "The latest trial was frozen on `10180cd8` (0.2.2-rc.1).",
            "The latest trial was frozen on `30293090` (0.2.3).",
            "The latest trial was frozen on `aaaa1111`.",
        )
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_OK, output)
        self.assertIn(f"{page}: 1 added line(s) rewritten on main", output)

    def test_a_trial_page_in_a_subfolder_is_satisfied_too(self) -> None:
        page = "docs/planning/p14-agent-trials/batch-2-reading-0.2.2-rc.1.md"
        self.repo.on("main")
        self.repo.write(page, "# Reading\n\nThe reading was made on freeze `aaaa1111`.\n")
        self.repo.commit("A reading on the base (#2)", BASE_TIME)
        self.repo.git("branch", "-f", "release/0.2.x")
        self.freeze_rewritten_on_both_sides(
            page,
            "The reading was made on freeze `10180cd8`.",
            "The reading was made on freeze `30293090`.",
            "The reading was made on freeze `aaaa1111`.",
        )
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_OK, output)

    def test_a_trial_page_line_that_main_never_took_is_still_reported(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit(
            "docs/planning/p14-agent-trials/README.md",
            "The latest trial was frozen on `aaaa1111`.",
            "The latest trial was frozen on `10180cd8`.",
        )
        self.release("The candidate's trial records (#10)")
        self.repo.on("main")
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn("docs/planning/p14-agent-trials/README.md (record)", output)

    def test_a_trial_freeze_file_is_still_compared_exactly(self) -> None:
        freeze = "docs/planning/p14-agent-trials/batch-2-0.2.2-rc.1/freeze.json"
        self.freeze_rewritten_on_both_sides(
            freeze, '"freeze": "10180cd8"', '"freeze": "30293090"', '"freeze": "aaaa1111"'
        )
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn(f"{freeze} (record)", output)
        self.assertNotIn("rewritten on main", output)

    def test_a_trial_run_record_is_still_compared_exactly(self) -> None:
        record = "docs/planning/p14-agent-trials/batch-2-0.2.2-rc.1/records/run-1.json"
        self.freeze_rewritten_on_both_sides(
            record, '"freeze": "10180cd8"', '"freeze": "30293090"', '"freeze": "aaaa1111"'
        )
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn(f"{record} (record)", output)

    def test_a_page_of_the_same_shape_outside_the_planning_folder_is_still_reported(self) -> None:
        page = "docs/operations/p14-agent-trials/reading.md"
        self.freeze_rewritten_on_both_sides(
            page,
            "The reading was made on freeze `10180cd8`.",
            "The reading was made on freeze `30293090`.",
            "The reading was made on freeze `aaaa1111`.",
        )
        code, output = self.repo.check()
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn(f"{page} (fix)", output)
        self.assertNotIn("rewritten on main", output)

    def test_a_trial_page_line_that_main_kept_with_words_added_is_satisfied(self) -> None:
        page = "docs/planning/p14-agent-trials/README.md"
        self.repo.on("release/0.2.x")
        self.repo.edit(
            page,
            "The latest trial was frozen on `aaaa1111`.",
            "The latest trial was frozen on `aaaa1111`.\n\nIt ran under the freeze committed by #435 in [`batch-2/freeze.json`](batch-2/freeze.json), which the cut kept.",
        )
        self.release("The candidate's trial records (#10)")
        self.repo.on("main")
        self.repo.edit(
            page,
            "The latest trial was frozen on `aaaa1111`.",
            "The latest trial was frozen on `aaaa1111`.\n\nIt ran under the freeze committed by #435 in `batch-2/freeze.json`, kept since 2026-10-19 as [`batch-2/freeze-0.2.2-rc.1.json`](batch-2/freeze-0.2.2-rc.1.json), which the cut kept.",
        )
        self.main_commit("The moved freeze on main (#12)")
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_OK, output)
        self.assertIn(f"{page}: 1 added line(s) rewritten on main", output)

    def test_a_trial_page_line_that_main_kept_with_a_word_missing_is_reported(self) -> None:
        page = "docs/planning/p14-agent-trials/README.md"
        self.repo.on("release/0.2.x")
        self.repo.edit(
            page,
            "The latest trial was frozen on `aaaa1111`.",
            "The latest trial was frozen on `aaaa1111`.\n\nIt ran under the freeze committed by #435 and the cut kept it byte for byte.",
        )
        self.release("The candidate's trial records (#10)")
        self.repo.on("main")
        self.repo.edit(
            page,
            "The latest trial was frozen on `aaaa1111`.",
            "The latest trial was frozen on `aaaa1111`.\n\nIt ran under the freeze committed by #435 and the cut changed it byte for byte.",
        )
        self.main_commit("The reworded freeze sentence on main (#12)")
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn(f"{page} (record)", output)

    # What stays strict.

    def test_a_code_line_reworded_on_main_is_still_reported(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit(
            "crates/engine/src/lib.rs", "retries = 3;", "retries = 5; // five tries for every call"
        )
        self.release("Retry five times (#10)")
        self.repo.on("main")
        self.repo.edit("crates/engine/src/lib.rs", "retries = 3;", "retries = 5; // five tries")
        self.main_commit("The counterpart (#12)")
        for records in (False, True):
            with self.subTest(records=records):
                code, output = self.repo.check(records=records)
                self.assertEqual(code, window.EXIT_MISSING, output)
                self.assertIn("1 added line(s) not on main", output)

    def test_a_shipped_document_reworded_on_main_is_still_reported(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.edit(
            "docs/operations/install.md",
            "Use the latest version.",
            "Use the stable version 0.2.2, the latest.",
        )
        self.release("The installation guide (#10)")
        self.repo.on("main")
        self.repo.edit(
            "docs/operations/install.md", "Use the latest version.", "Use the stable version 0.2.2."
        )
        self.main_commit("The counterpart (#12)")
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn("docs/operations/install.md (record): 1 added line(s) not on main", output)
        self.assertNotIn("rewritten on main", output)

    def test_a_ledger_line_is_compared_exactly(self) -> None:
        self.repo.on("release/0.2.x")
        self.repo.write(
            "docs/planning/p14-evidence-ledger.json", '{\n  "status": "passed for the candidate"\n}\n'
        )
        self.release("The ledger (#10)")
        self.repo.on("main")
        self.repo.write(
            "docs/planning/p14-evidence-ledger.json", '{\n  "status": "passed for the stable release"\n}\n'
        )
        self.main_commit("The ledger on main (#12)")
        code, output = self.repo.check(records=True)
        self.assertEqual(code, window.EXIT_MISSING, output)
        self.assertIn("p14-evidence-ledger.json (record)", output)


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

    def test_the_rewritten_record_pages_are_the_changelog_the_planning_and_the_trial_pages_only(self) -> None:
        for path in (
            "CHANGELOG.md",
            "docs/planning/known-limits.md",
            "docs/planning/release-0.2.2.md",
            "docs/planning/p14-agent-trials/README.md",
            "docs/planning/p14-agent-trials/batch-2-reading-0.2.2-rc.1.md",
            "docs/planning/p14-agent-trials/batch-2-0.2.2-rc.1/SUMMARY.md",
        ):
            self.assertTrue(window.is_rewritten_record(path), path)
        for path in (
            "docs/planning/p14-evidence-ledger.json",
            "docs/planning/p14-agent-trials/batch-2-0.2.2-rc.1/freeze.json",
            "docs/planning/p14-agent-trials/batch-2-0.2.2-rc.1/records/run-1.json",
            "docs/planning/p14-agent-trials/batch-2-0.2.2-rc.1/records/notes.md",
            "docs/planning/other-folder/page.md",
            "docs/operations/p14-agent-trials/README.md",
            "docs/operations/install.md",
            "npm/vsift-cli/README.md",
            "memory/TODO.md",
            "crates/vsift/src/lib.rs",
        ):
            self.assertFalse(window.is_rewritten_record(path), path)

    def test_words_ignore_link_targets_and_marks_and_a_short_line_is_no_extension(self) -> None:
        self.assertEqual(
            window.words("It ran under [`a/freeze.json`](a/freeze.json), which (kept) it."),
            ["It", "ran", "under", "a/freeze.json", "which", "kept", "it"],
        )
        long_line = window.words("one two three four five six seven")
        self.assertTrue(window.is_extension_of("one two three four five six", [long_line]))
        self.assertFalse(window.is_extension_of("one two three four six five", [long_line]))
        self.assertFalse(window.is_extension_of("one two three", [long_line]))

    def test_a_lead_ignores_list_and_table_markers(self) -> None:
        self.assertEqual(window.lead("- **Next step:** x"), "Next step:** x")
        self.assertEqual(window.lead("| `0.2.2` | Not made."), "`0.2.2` | Not made.")
        self.assertEqual(len(window.lead("a" * 200)), window.LEAD_LENGTH)

    def test_hunks_keep_their_added_and_removed_lines_together(self) -> None:
        diff = (
            "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-old\n+new\n"
            "@@ -5,0 +6 @@\n+inserted\n"
        )
        self.assertEqual(
            window.parse_hunk_groups(diff),
            [window.Hunk(("new",), ("old",)), window.Hunk(("inserted",), ())],
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
