"""Guardrails for the CI lane selection (tools/ci_lanes.py).

They run without cargo, git or the network, on the workspace graph as it stands
(written out below, and checked against the fuzz harness's real manifest), so the
`Changes` job can run them before it trusts the selection it is about to make.
"""

from __future__ import annotations

import json
from pathlib import Path
import unittest

import ci_lanes as lanes
from ci_lanes import Package

REPOSITORY = Path(__file__).resolve().parent.parent


def package(name: str, directory: str, *dependencies: str) -> Package:
    return Package(name, directory + "/", frozenset(dependencies))


# The workspace members and their internal edges (normal, build and development)
# as `cargo metadata` reported them on 2026-10-16.
WORKSPACE = (
    package("vsift-domain", "crates/vsift-domain"),
    package("vsift-application", "crates/vsift-application", "vsift-domain"),
    package("vsift-contract", "crates/vsift-contract", "vsift-application", "vsift-domain"),
    package(
        "vsift-infrastructure", "crates/vsift-infrastructure", "vsift-application", "vsift-domain"
    ),
    package(
        "vsift",
        "crates/vsift",
        "vsift-application",
        "vsift-contract",
        "vsift-domain",
        "vsift-infrastructure",
    ),
    package(
        "vsift-cli",
        "crates/vsift-cli",
        "vsift",
        "vsift-contract",
        "vsift-application",
        "vsift-domain",
        "vsift-infrastructure",
    ),
    package(
        "vsift-crash-campaign",
        "tools/p10-crash-campaign",
        "vsift",
        "vsift-application",
        "vsift-contract",
        "vsift-domain",
        "vsift-infrastructure",
    ),
    package("vsift-agent-trials", "tools/vsift-agent-trials", "vsift", "vsift-contract"),
    package("vsift-governance", "tools/vsift-governance"),
    package("vsift-release", "tools/vsift-release"),
)
FUZZ = frozenset({"vsift-application", "vsift-contract", "vsift-domain", "vsift-infrastructure"})
EVERY_PACKAGE = sorted(member.name for member in WORKSPACE)
PLAIN_RUST = "pub fn add(left: u32, right: u32) -> u32 { left + right }\n"


def choose(*changed: str, texts: dict[str, list[str]] | None = None, labels: tuple[str, ...] = ()):
    selection = lanes.select(
        list(changed), WORKSPACE, texts or {}, fuzz_depends_on=FUZZ, labels=labels
    )
    return selection, selection.outputs(WORKSPACE)


class DocumentationLaneTests(unittest.TestCase):
    def test_prose_and_the_work_record_run_only_documentation_and_governance(self) -> None:
        selection, outputs = choose(
            "docs/development.md",
            "docs/planning/known-limits.md",
            "memory/TODO.md",
            "CHANGELOG.md",
            "README.md",
        )
        self.assertEqual(selection.lane, "docs")
        self.assertEqual(outputs["quality"], "false")
        self.assertEqual(json.loads(outputs["systems"]), ["ubuntu-latest"])
        self.assertEqual(outputs["packages"], "")
        for flag in ("platform", "fuzz", "npm", "worker_boundary"):
            self.assertEqual(outputs[flag], "false", flag)

    def test_tools_with_workflows_of_their_own_are_documentation_here(self) -> None:
        selection, _ = choose(
            "tools/p14_journeys.py",
            "tools/test_p14_journeys.py",
            "tools/guide/check-examples.cjs",
            "tools/p14-published/lib/verify.cjs",
            ".github/ISSUE_TEMPLATE/bug_report.yml",
            "deny.toml",
        )
        self.assertEqual(selection.lane, "docs")


class FullLaneTests(unittest.TestCase):
    def assert_full(self, *changed: str) -> None:
        selection, outputs = choose(*changed)
        self.assertEqual(selection.lane, "full", changed)
        self.assertEqual(outputs["packages"].split(), EVERY_PACKAGE)
        self.assertEqual(json.loads(outputs["systems"]), list(lanes.SYSTEMS_ALL))
        for flag in ("platform", "fuzz", "npm", "worker_boundary"):
            self.assertEqual(outputs[flag], "true", flag)

    def test_build_wide_files_select_everything(self) -> None:
        for path in (
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "rustfmt.toml",
            ".gitattributes",
            ".github/workflows/ci.yml",
            ".github/workflows/nightly.yml",
            "tools/ci_lanes.py",
            "tools/test_ci_lanes.py",
        ):
            self.assert_full(path)

    def test_a_member_manifest_or_build_script_selects_everything(self) -> None:
        self.assert_full("crates/vsift-domain/Cargo.toml")
        self.assert_full("crates/vsift-cli/build.rs")
        self.assert_full("tools/vsift-release/Cargo.toml")

    def test_a_path_no_rule_knows_selects_everything(self) -> None:
        self.assert_full("new-folder/file.txt")
        self.assert_full("LICENSE-NEW", "tools/unknown-tool/run.cjs")

    def test_the_label_or_an_empty_change_selects_everything(self) -> None:
        selection, _ = choose("docs/development.md", labels=(lanes.FULL_LABEL,))
        self.assertEqual(selection.lane, "full")
        self.assertEqual(choose()[0].lane, "full")

    def test_one_full_trigger_wins_over_narrower_paths(self) -> None:
        self.assert_full("docs/development.md", "crates/vsift-cli/src/main.rs", "Cargo.lock")


class CratesLaneTests(unittest.TestCase):
    def test_a_domain_change_selects_every_dependent_and_the_fuzz_harness(self) -> None:
        selection, outputs = choose(
            "crates/vsift-domain/src/timeline.rs", texts={"crates/vsift-domain/src/timeline.rs": [PLAIN_RUST]}
        )
        self.assertEqual(selection.lane, "crates")
        self.assertEqual(
            outputs["packages"].split(),
            sorted(set(EVERY_PACKAGE) - {"vsift-governance", "vsift-release"}),
        )
        self.assertEqual(outputs["fuzz"], "true")
        self.assertEqual(json.loads(outputs["systems"]), ["ubuntu-latest"])

    def test_a_cli_change_selects_the_cli_alone(self) -> None:
        path = "crates/vsift-cli/src/human/render.rs"
        selection, outputs = choose(path, texts={path: [PLAIN_RUST]})
        self.assertEqual(outputs["packages"], "vsift-cli")
        self.assertEqual(outputs["package_arguments"], "--package vsift-cli")
        self.assertEqual(outputs["fuzz"], "false")
        self.assertEqual(outputs["quality"], "true")

    def test_an_engine_change_selects_its_dependents_but_not_its_dependencies(self) -> None:
        path = "crates/vsift/src/search.rs"
        _, outputs = choose(path, texts={path: [PLAIN_RUST]})
        self.assertEqual(
            outputs["packages"].split(),
            ["vsift", "vsift-agent-trials", "vsift-cli", "vsift-crash-campaign"],
        )

    def test_files_another_package_reads_select_that_package(self) -> None:
        _, outputs = choose("schemas/v1/frame-get.schema.json")
        for name in ("vsift-contract", "vsift", "vsift-cli", "vsift-infrastructure", "vsift-release"):
            self.assertIn(name, outputs["packages"].split())
        _, outputs = choose("docs/contracts/cli-v1.md")
        self.assertEqual(outputs["packages"], "vsift-cli")
        _, outputs = choose("docs/operations/release.md")
        self.assertEqual(outputs["packages"], "vsift-release")
        _, outputs = choose("crates/vsift-infrastructure/tests/data/visual_samples/F02.json")
        self.assertIn("vsift-contract", outputs["packages"].split())
        self.assertEqual(outputs["fuzz"], "true")

    def test_the_skill_selects_its_readers_and_the_fuzz_replay(self) -> None:
        _, outputs = choose("skills/vsift/SKILL.md")
        for name in ("vsift-contract", "vsift-cli", "vsift-agent-trials", "vsift-release"):
            self.assertIn(name, outputs["packages"].split())
        self.assertEqual(outputs["fuzz"], "true")

    def test_the_fuzz_harness_alone_runs_its_job_but_no_quality_job(self) -> None:
        selection, outputs = choose("fuzz/fuzz_targets/transcript_srt.rs")
        self.assertEqual(selection.lane, "crates")
        self.assertEqual(outputs["fuzz"], "true")
        self.assertEqual(outputs["quality"], "false")
        self.assertEqual(json.loads(outputs["systems"]), ["ubuntu-latest"])

    def test_the_launcher_runs_its_tests_and_every_system(self) -> None:
        selection, outputs = choose("npm/vsift-cli/lib/launcher.cjs")
        self.assertEqual(selection.lane, "crates")
        self.assertEqual(outputs["npm"], "true")
        self.assertEqual(json.loads(outputs["systems"]), list(lanes.SYSTEMS_ALL))
        self.assertEqual(outputs["packages"].split(), ["vsift-agent-trials", "vsift-release"])


class PlatformTests(unittest.TestCase):
    def test_platform_code_before_or_after_the_change_adds_windows_and_macos(self) -> None:
        path = "crates/vsift-infrastructure/src/session_root.rs"
        for texts in (
            [PLAIN_RUST, "#[cfg(windows)]\nfn acl() {}\n"],
            ["#[cfg(all(unix, not(target_os = \"macos\")))]\nfn f() {}\n"],
            ["use std::os::unix::fs::PermissionsExt;\n"],
            ["use cap_std::fs::Dir;\n"],
            ["let _ = tokio::signal::ctrl_c();\n"],
            ["file.try_lock_shared()?;\n"],
        ):
            _, outputs = choose(path, texts={path: texts})
            self.assertEqual(json.loads(outputs["systems"]), list(lanes.SYSTEMS_ALL), texts)

    def test_plain_code_stays_on_ubuntu(self) -> None:
        path = "crates/vsift-domain/src/timeline.rs"
        _, outputs = choose(path, texts={path: [PLAIN_RUST, "let command = Command::new(\"x\");\n"]})
        self.assertEqual(json.loads(outputs["systems"]), ["ubuntu-latest"])

    def test_scripts_are_platform_sensitive(self) -> None:
        _, outputs = choose("tools/send-console-ctrl.ps1")
        self.assertEqual(json.loads(outputs["systems"]), list(lanes.SYSTEMS_ALL))
        self.assertEqual(outputs["packages"], "vsift-cli")

    def test_the_markers_find_the_repositorys_platform_code(self) -> None:
        for relative in (
            "crates/vsift-infrastructure/src/process_supervisor.rs",
            "crates/vsift-infrastructure/src/private_user_root.rs",
            "crates/vsift-infrastructure/src/file_lock.rs",
            "crates/vsift-infrastructure/src/session_root.rs",
            "crates/vsift-cli/src/signal.rs",
            "crates/vsift/src/worker.rs",
        ):
            text = (REPOSITORY / relative).read_text(encoding="utf-8")
            self.assertTrue(lanes.is_platform_sensitive(relative, [text]), relative)
        for relative in (
            "crates/vsift-domain/src/timeline.rs",
            "crates/vsift-application/src/lib.rs",
        ):
            text = (REPOSITORY / relative).read_text(encoding="utf-8")
            self.assertFalse(lanes.is_platform_sensitive(relative, [text]), relative)

    def test_the_worker_boundary_runs_for_the_supervisor(self) -> None:
        path = "crates/vsift-infrastructure/src/process_supervisor.rs"
        _, outputs = choose(path, texts={path: [PLAIN_RUST]})
        self.assertEqual(outputs["worker_boundary"], "true")


class CodeQLTests(unittest.TestCase):
    """`Analyze Rust` is required, so it always reports; this decides only whether it analyses."""

    def test_rust_and_what_cargo_resolves_are_analysed(self) -> None:
        for path in (
            "crates/vsift-domain/src/lib.rs",
            "crates/vsift-cli/tests/anything.txt",
            "fuzz/fuzz_targets/transcript_srt.rs",
            "tools/p10-crash-campaign/src/main.rs",
            "tools/vsift-release/Cargo.toml",
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            ".github/workflows/codeql.yml",
            ".github/codeql/codeql-config.yml",
        ):
            self.assertTrue(lanes.reads_rust(path), path)
            self.assertEqual(choose(path)[1]["codeql"], "true", path)

    def test_documentation_and_workflows_that_are_not_codeql_need_no_analysis(self) -> None:
        for path in (
            "docs/development.md",
            "memory/TODO.md",
            "README.md",
            "CHANGELOG.md",
            "npm/vsift-cli/package.json",
            "skills/vsift/SKILL.md",
            "schemas/v1/example.json",
            "tools/ci_lanes.py",
            "tools/p14-published/upgrade.cjs",
            ".github/workflows/ci.yml",
            ".github/workflows/nightly.yml",
            ".github/workflows/p14-local-upgrade.yml",
        ):
            self.assertFalse(lanes.reads_rust(path), path)
            self.assertEqual(choose(path)[1]["codeql"], "false", path)

    def test_the_choice_is_independent_of_the_lane(self) -> None:
        # A workflow change runs the full lane of tests but gives CodeQL nothing new to read.
        selection, outputs = choose(".github/workflows/ci.yml")
        self.assertEqual(selection.lane, "full")
        self.assertEqual(outputs["codeql"], "false")
        # A Rust change in the crates lane is analysed.
        selection, outputs = choose("crates/vsift-cli/src/main.rs", texts={"crates/vsift-cli/src/main.rs": [PLAIN_RUST]})
        self.assertEqual(selection.lane, "crates")
        self.assertEqual(outputs["codeql"], "true")

    def test_one_file_that_it_reads_among_documents_selects_it(self) -> None:
        self.assertTrue(lanes.codeql_selected(["docs/development.md", "crates/vsift/src/lib.rs"]))
        self.assertFalse(lanes.codeql_selected(["docs/development.md", "memory/TODO.md"]))

    def test_an_unreadable_change_or_a_request_for_everything_is_analysed(self) -> None:
        self.assertTrue(lanes.codeql_selected([]))
        self.assertTrue(lanes.codeql_selected(["docs/development.md"], force_full=True))
        self.assertEqual(choose()[1]["codeql"], "true")

    def test_the_label_does_not_ask_for_the_analysis(self) -> None:
        _, outputs = choose("docs/development.md", labels=(lanes.FULL_LABEL,))
        self.assertEqual(outputs["codeql"], "false")


class LocalUpgradeTests(unittest.TestCase):
    """`P14 local upgrade` runs in the nightly; a pull request gets it only by the label."""

    def test_no_change_asks_for_it_without_the_label(self) -> None:
        for changed in (
            ("crates/vsift-cli/src/main.rs",),
            ("npm/vsift-cli/package.json",),
            ("schemas/v1/example.json",),
            ("Cargo.lock",),
            (".github/workflows/p14-local-upgrade.yml",),
            ("tools/p14-published/upgrade.cjs",),
            ("docs/development.md",),
        ):
            self.assertEqual(choose(*changed)[1]["local_upgrade"], "false", changed)

    def test_the_full_ci_label_asks_for_it(self) -> None:
        for changed in (("docs/development.md",), ("crates/vsift-cli/src/main.rs",)):
            _, outputs = choose(*changed, labels=(lanes.FULL_LABEL,))
            self.assertEqual(outputs["local_upgrade"], "true", changed)

    def test_other_labels_and_the_full_lane_alone_do_not(self) -> None:
        _, outputs = choose("Cargo.lock", labels=("dependencies",))
        self.assertEqual(outputs["lane"], "full")
        self.assertEqual(outputs["local_upgrade"], "false")


class OutputTests(unittest.TestCase):
    def test_outputs_carry_only_fixed_words_and_package_names(self) -> None:
        hostile = "crates/vsift-cli/src/$(curl example.invalid)`x`\n.rs"
        selection, outputs = choose(hostile, "odd$(x)/a\nb.txt")
        for value in outputs.values():
            self.assertNotIn("$(", value)
            self.assertNotIn("example.invalid", value)
            self.assertNotIn("\n", value)
        for reason in selection.reasons:
            self.assertNotIn("$(", reason)
            self.assertNotIn("\n", reason)

    def test_the_fuzz_dependencies_are_read_from_its_manifest(self) -> None:
        manifest = (REPOSITORY / lanes.FUZZ_MANIFEST).read_text(encoding="utf-8")
        self.assertEqual(lanes.fuzz_dependencies(manifest), FUZZ)

    def test_metadata_gives_folders_and_path_edges_only(self) -> None:
        metadata = {
            "workspace_root": "/w",
            "workspace_members": ["a 0.1.0 (path+file:///w/crates/a)", "b 0.1.0 (path+file:///w/tools/b)"],
            "packages": [
                {
                    "id": "a 0.1.0 (path+file:///w/crates/a)",
                    "name": "a",
                    "manifest_path": "/w/crates/a/Cargo.toml",
                    "dependencies": [{"name": "serde"}],
                },
                {
                    "id": "b 0.1.0 (path+file:///w/tools/b)",
                    "name": "b",
                    "manifest_path": "/w/tools/b/Cargo.toml",
                    "dependencies": [{"name": "a", "path": "/w/crates/a"}, {"name": "serde"}],
                },
            ],
        }
        packages = lanes.workspace_packages(metadata)
        self.assertEqual(
            packages,
            [Package("a", "crates/a/", frozenset()), Package("b", "tools/b/", frozenset({"a"}))],
        )
        self.assertEqual(lanes.dependents(packages, {"a"}), {"a", "b"})


if __name__ == "__main__":
    unittest.main()
