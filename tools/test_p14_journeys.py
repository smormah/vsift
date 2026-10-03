"""Guardrails for the P14 journeys driver (tools/p14_journeys.py).

These run without the network, npm, Homebrew or a published package. They check
the decisions the workflow depends on: which version is chosen, which text may
reach a command line, what counts as the published binary, which tests are
compiled, and how a result is classified and reported.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import p14_journeys as journeys
from p14_journeys import Failure

COMMIT = "011bc4da1af6c7b0d4f3e2a1908877665544332f"


class VersionTests(unittest.TestCase):
    def test_a_pre_release_sorts_before_its_release(self) -> None:
        self.assertLess(journeys.semver_key("0.2.0-rc.1"), journeys.semver_key("0.2.0"))
        self.assertLess(journeys.semver_key("0.1.0"), journeys.semver_key("0.2.0-rc.1"))

    def test_pre_release_identifiers_follow_semver_precedence(self) -> None:
        ordered = ["0.2.0-alpha", "0.2.0-alpha.1", "0.2.0-alpha.beta", "0.2.0-beta",
                   "0.2.0-beta.2", "0.2.0-beta.11", "0.2.0-rc.1", "0.2.0"]
        keys = [journeys.semver_key(version) for version in ordered]
        self.assertEqual(keys, sorted(keys))

    def test_numbers_compare_as_numbers(self) -> None:
        self.assertLess(journeys.semver_key("0.9.0"), journeys.semver_key("0.10.0"))
        self.assertLess(journeys.semver_key("0.2.0-rc.2"), journeys.semver_key("0.2.0-rc.10"))

    def test_the_placeholder_is_never_the_highest(self) -> None:
        self.assertEqual(journeys.highest_published(["0.0.0", "0.1.0"]), "0.1.0")
        self.assertEqual(journeys.highest_published(["0.1.0", "0.2.0-rc.1"]), "0.2.0-rc.1")
        self.assertEqual(journeys.highest_published(["0.2.0", "0.2.0-rc.1", "0.1.0"]), "0.2.0")
        with self.assertRaises(Failure):
            journeys.highest_published(["0.0.0"])
        with self.assertRaises(Failure):
            journeys.highest_published([])

    def test_text_that_is_not_a_version_is_refused_before_a_command_sees_it(self) -> None:
        for text in ["", "v0.1.0", "0.1", "0.1.0; rm -rf /", "$(touch pwned)", "0.1.0 ",
                     "latest", "0.1.0\n--registry=http://127.0.0.1", "0.1.0-", "01.2.3"]:
            with self.assertRaises(Failure, msg=text):
                journeys.validated_version(text)
        self.assertEqual(journeys.validated_version("0.2.0-rc.1"), "0.2.0-rc.1")


class BinaryChecksTests(unittest.TestCase):
    def test_the_binary_must_name_the_version_and_the_tags_commit(self) -> None:
        line = journeys.check_version_output("vsift 0.1.0 (011bc4da1af6)\n", "0.1.0", COMMIT)
        self.assertEqual(line, "vsift 0.1.0 (011bc4da1af6)")
        for text, version in [
            ("vsift 0.1.0\n", "0.1.0"),
            ("vsift 0.2.0 (011bc4da1af6)\n", "0.1.0"),
            ("vsift 0.1.0 (ffffffffffff)\n", "0.1.0"),
            ("something else\n", "0.1.0"),
            ("vsift 0.1.0 (011bc4da1af6) extra\n", "0.1.0"),
        ]:
            with self.assertRaises(Failure, msg=text):
                journeys.check_version_output(text, version, COMMIT)

    def test_a_package_must_come_from_the_real_registry_with_an_integrity(self) -> None:
        good = {"packages": {"node_modules/vsift-cli": {
            "version": "0.1.0",
            "resolved": "https://registry.npmjs.org/vsift-cli/-/vsift-cli-0.1.0.tgz",
            "integrity": "sha512-abc"}}}
        self.assertEqual(journeys.lock_entry(good, "vsift-cli")["version"], "0.1.0")
        for bad in [
            {"packages": {}},
            {"packages": {"node_modules/vsift-cli": {
                "version": "0.1.0", "resolved": "http://127.0.0.1:4873/vsift-cli.tgz",
                "integrity": "sha512-abc"}}},
            {"packages": {"node_modules/vsift-cli": {
                "version": "0.1.0",
                "resolved": "https://registry.npmjs.org/vsift-cli/-/vsift-cli-0.1.0.tgz"}}},
        ]:
            with self.assertRaises(Failure):
                journeys.lock_entry(bad, "vsift-cli")

    def test_the_native_package_is_the_launchers_for_each_supported_machine(self) -> None:
        for system, machine, expected in [
            ("linux", "x86_64", ("@vsift/linux-x64", "vsift")),
            ("darwin", "arm64", ("@vsift/darwin-arm64", "vsift")),
            ("win32", "AMD64", ("@vsift/win32-x64", "vsift.exe")),
        ]:
            with mock.patch.object(sys, "platform", system), \
                    mock.patch("platform.machine", return_value=machine):
                self.assertEqual(journeys.native_package(), expected)
        with mock.patch.object(sys, "platform", "linux"), \
                mock.patch("platform.machine", return_value="aarch64"):
            with self.assertRaises(Failure):
                journeys.native_package()


class ManagedFilesTests(unittest.TestCase):
    def tree(self, root: Path, names: list[str]) -> None:
        for name in names:
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"x")

    def test_each_reviewed_file_is_found_exactly_once(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.tree(root, ["versions/a/ffmpeg", "versions/a/ffprobe",
                             "versions/b/whisper-cli", "versions/c/ggml-base.bin",
                             "current/ffmpeg_ffprobe.current"])
            found = journeys.find_managed_files(root)
            self.assertEqual(found["ffmpeg"].parent, found["ffprobe"].parent)
            self.assertEqual(found["whisper_model"].name, "ggml-base.bin")

    def test_a_missing_or_doubled_file_is_a_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.tree(root, ["versions/a/ffmpeg", "versions/a/ffprobe",
                             "versions/b/whisper-cli"])
            with self.assertRaises(Failure):
                journeys.find_managed_files(root)
            self.tree(root, ["versions/c/ggml-base.bin", "versions/d/ggml-base.bin"])
            with self.assertRaises(Failure):
                journeys.find_managed_files(root)

    def test_an_unfinished_stage_is_not_an_install(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.tree(root, ["versions/a/ffmpeg", "versions/a/ffprobe",
                             "versions/b/whisper-cli", "stage-1/ggml-base.bin"])
            with self.assertRaises(Failure):
                journeys.find_managed_files(root)


class SourceSelectionTests(unittest.TestCase):
    def repository(self, root: Path, with_override: bool) -> str:
        root.mkdir(parents=True)
        commands = [["init", "-q"], ["config", "user.name", "tester"],
                    ["config", "user.email", "tester@example.invalid"]]
        for command in commands:
            subprocess.run(["git", *command], cwd=root, check=True, capture_output=True)
        if with_override:
            module = root / journeys.OVERRIDE_MODULE
            module.parent.mkdir(parents=True)
            module.write_text("// override\n", encoding="utf-8")
        (root / "file.txt").write_text("x", encoding="utf-8")
        subprocess.run(["git", "add", "."], cwd=root, check=True, capture_output=True)
        subprocess.run(["git", "commit", "-q", "-m", "state"], cwd=root, check=True,
                       capture_output=True)
        subprocess.run(["git", "tag", "v0.1.0"], cwd=root, check=True, capture_output=True)
        return subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, check=True,
                              capture_output=True, text=True).stdout.strip()

    def select(self, with_override: bool, tests_from: str) -> dict[str, object]:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            tag_commit = self.repository(base / "tag", with_override)
            self.repository(base / "workflow", True)
            arguments = argparse.Namespace(
                version="0.1.0", work=str(base / "work"), tag_source=str(base / "tag"),
                workflow_source=str(base / "workflow"), tests_from=tests_from)
            with mock.patch.dict(os.environ, {}, clear=False), \
                    mock.patch.object(journeys, "append_summary"):
                os.environ.pop("GITHUB_STEP_SUMMARY", None)
                journeys.command_select_source(arguments)
            state = json.loads((base / "work" / "state.json").read_text(encoding="utf-8"))
            self.assertEqual(state["tag_commit"], tag_commit)
            state["tag_root"] = str(base / "tag")
            state["workflow_root"] = str(base / "workflow")
            return state

    def test_the_tag_that_has_the_override_supplies_its_own_tests(self) -> None:
        state = self.select(True, "auto")
        self.assertEqual(state["tests_from"], "tag")
        self.assertEqual(Path(str(state["tests_root"])), Path(str(state["tag_root"])).resolve())

    def test_a_tag_that_predates_the_override_borrows_the_workflow_refs_tests(self) -> None:
        state = self.select(False, "auto")
        self.assertEqual(state["tests_from"], "workflow_ref")
        self.assertFalse(state["tag_has_override"])

    def test_asking_for_the_tags_tests_when_it_cannot_run_them_is_a_failure(self) -> None:
        with self.assertRaises(Failure):
            self.select(False, "tag")

    def test_the_checkout_must_be_at_the_tag(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            self.repository(base / "tag", False)
            self.repository(base / "workflow", True)
            (base / "tag" / "more.txt").write_text("y", encoding="utf-8")
            subprocess.run(["git", "add", "."], cwd=base / "tag", check=True, capture_output=True)
            subprocess.run(["git", "commit", "-q", "-m", "later"], cwd=base / "tag", check=True,
                           capture_output=True)
            arguments = argparse.Namespace(
                version="0.1.0", work=str(base / "work"), tag_source=str(base / "tag"),
                workflow_source=str(base / "workflow"), tests_from="auto")
            with self.assertRaises(Failure):
                journeys.command_select_source(arguments)


class ResultsTests(unittest.TestCase):
    def rows(self, *statuses: str) -> list[dict[str, str]]:
        return [{"name": f"stage_{index}", "status": status, "detail": ""}
                for index, status in enumerate(statuses)]

    def test_an_exit_code_and_the_stage_statuses_make_one_result(self) -> None:
        self.assertEqual(journeys.classify(0, self.rows("passed", "passed")), "passed")
        self.assertEqual(journeys.classify(101, self.rows("passed", "failed")), "failed")
        self.assertEqual(journeys.classify(101, self.rows("passed", "blocked")), "blocked")
        self.assertEqual(journeys.classify(101, []), "failed")
        self.assertEqual(journeys.classify(None, self.rows("passed")), "timed_out")

    def results(self, status: str) -> dict[str, object]:
        return {
            "format": 1, "system": "ubuntu", "version": "0.1.0", "tag_commit": COMMIT,
            "workflow_commit": COMMIT, "tests_from": "workflow_ref",
            "version_line": "vsift 0.1.0 (011bc4da1af6)",
            "checkpoints": [{
                "key": "p09_evidence", "test": "p09_evidence_e2e", "what": "evidence",
                "status": status, "seconds": 12,
                "stages": self.rows("passed", "failed" if status == "failed" else "passed"),
            }],
            "not_run": [{"name": "p13", "reason": "not applicable"}],
            "overall": "passed" if status == "passed" else "failed",
        }

    def test_a_failure_is_named_in_the_summary_with_its_stage(self) -> None:
        text = journeys.render_results(self.results("failed"))
        self.assertIn("FAILED", text)
        self.assertIn("p09_evidence", text)
        self.assertIn("stage_1", text)
        self.assertIn("Not run here, and why", text)

    def test_a_pass_says_so_and_still_lists_what_was_not_run(self) -> None:
        text = journeys.render_results(self.results("passed"))
        self.assertIn("PASSED", text)
        self.assertIn("p13", text)
        self.assertNotIn("Stages that did not pass", text)

    def test_an_unfinished_results_file_is_a_failure_that_names_where_it_stopped(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            out = Path(temporary) / "out"
            out.mkdir()
            unfinished = self.results("passed")
            unfinished["complete"] = False
            (out / "results.json").write_text(json.dumps(unfinished), encoding="utf-8")
            with mock.patch.object(journeys, "append_summary") as summary:
                with self.assertRaises(Failure):
                    journeys.command_summarize(argparse.Namespace(work=temporary))
            text = summary.call_args.args[0]
            self.assertIn("(stopped)", text)
            self.assertIn("FAILED", text)

    def test_the_aggregate_fails_when_a_system_has_no_results(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "ubuntu"
            directory.mkdir()
            (directory / "results.json").write_text(
                json.dumps(self.results("passed")), encoding="utf-8")
            arguments = argparse.Namespace(results=temporary, version="0.1.0")
            with mock.patch.object(journeys, "append_summary") as summary:
                with self.assertRaises(Failure):
                    journeys.command_report(arguments)
            self.assertIn("NO RESULTS", summary.call_args.args[0])


class CheckpointListTests(unittest.TestCase):
    ROOT = Path(__file__).resolve().parents[1]

    def test_every_checkpoint_names_a_test_file_that_exists(self) -> None:
        for checkpoint in journeys.CHECKPOINTS:
            crate = self.ROOT / "crates" / checkpoint.package / "tests" / f"{checkpoint.test}.rs"
            self.assertTrue(crate.is_file(), f"{checkpoint.key}: {crate} is missing")

    def test_keys_are_unique_and_systems_are_known(self) -> None:
        keys = [checkpoint.key for checkpoint in journeys.CHECKPOINTS]
        self.assertEqual(len(keys), len(set(keys)))
        for checkpoint in journeys.CHECKPOINTS:
            self.assertTrue(set(checkpoint.systems) <= set(journeys.SYSTEMS))

    def test_a_checkpoint_of_another_package_is_labelled_as_not_the_binary(self) -> None:
        for checkpoint in journeys.CHECKPOINTS:
            if checkpoint.package != "vsift-cli":
                self.assertIn("NOT the installed binary", checkpoint.what)

    def test_every_system_says_what_it_does_not_run(self) -> None:
        for system in journeys.SYSTEMS:
            self.assertTrue(journeys.NOT_RUN_ELSEWHERE[system])
            self.assertIn(system, journeys.SYSTEM_LABELS)

    def test_a_checkpoint_can_be_skipped_by_name(self) -> None:
        arguments = journeys.build_parser().parse_args(["run", "--skip", "p07_asr_gates"])
        self.assertEqual(arguments.skip, ["p07_asr_gates"])
        with self.assertRaises(SystemExit):
            journeys.build_parser().parse_args(["run", "--skip", "no_such_checkpoint"])


class RunLoggedTests(unittest.TestCase):
    def test_output_is_copied_and_the_exit_code_returned(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            log = Path(temporary) / "logs" / "one.log"
            code, _, drained = journeys.run_logged(
                [sys.executable, "-c", "print('line one'); raise SystemExit(3)"], log,
                cwd=Path(temporary), env=dict(os.environ), timeout=60)
            self.assertEqual(code, 3)
            self.assertTrue(drained)
            self.assertIn("line one", log.read_text(encoding="utf-8"))

    def test_a_command_past_its_deadline_is_stopped(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            code, seconds, _ = journeys.run_logged(
                [sys.executable, "-c", "import time; time.sleep(60)"],
                Path(temporary) / "slow.log", cwd=Path(temporary), env=dict(os.environ),
                timeout=1)
            self.assertIsNone(code)
            self.assertLess(seconds, 30)

    def test_a_descendant_that_keeps_the_output_open_does_not_stall_the_driver(self) -> None:
        # The parent ends at once; its child inherits the output pipe and outlives it. Waiting
        # for the end of that output, or closing the pipe under the reader, would block here.
        # The child and the reader keep the working folder and the log open for a while, which
        # Windows will not let be removed, so they live outside a managed temporary folder.
        script = ("import subprocess, sys; "
                  "subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(20)']); "
                  "print('parent done')")
        log = Path(tempfile.gettempdir()) / f"p14-driver-test-{os.getpid()}.log"
        with mock.patch.object(journeys, "DRAIN_WAIT_SECONDS", 2):
            code, seconds, drained = journeys.run_logged(
                [sys.executable, "-c", script], log, cwd=Path(tempfile.gettempdir()),
                env=dict(os.environ), timeout=60)
        self.assertEqual(code, 0)
        self.assertFalse(drained)
        self.assertLess(seconds, 15)
        self.assertIn("parent done", log.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
