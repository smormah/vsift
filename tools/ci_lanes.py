"""Choose the lanes of the CI pull-request gate from the files a change touches.

The `CI` workflow's `Changes` job runs this once per pull request (and per push to
`main`) and every other job of the fast gate reads its outputs. The rules, in plain
words (`docs/development.md`, "The CI lanes", is the authoritative description):

- **full**: a change to anything that can alter what every crate compiles or how CI
  itself runs (the root manifest and lockfile, the toolchain, any member's
  `Cargo.toml` or `build.rs`, `rustfmt.toml`, `.gitattributes`, the workflows, this
  script), a path no rule below knows, the `full-ci` label, or an explicit request.
  The full lane is the old CI: the whole workspace, all three systems, every job
  except the two slow checks below.
- **crates**: the touched workspace packages, every package that depends on them
  (from `cargo metadata`, normal, build and development edges), and the packages
  whose tests read a touched file outside their own folder (`SHARED_INPUTS`). Lint
  and tests run for exactly those packages, on Ubuntu; on Windows and macOS too when
  a touched file is platform-sensitive (`PLATFORM_MARKERS`, `PLATFORM_PATHS`).
- **docs**: every touched file is documentation or a tool with its own workflow, so
  only the documentation and governance checks run.

Two slow checks are chosen apart from the lanes (decided 2026-10-16), because a lane
says what the *tests* need and these two have narrower reasons to run:

- **codeql**: the `CodeQL` workflow's `Analyze Rust` job analyses a pull request only
  when the change touches something CodeQL reads (`reads_rust`: Rust sources, Cargo
  manifests and lockfile, the toolchain file, the CodeQL workflow and configuration).
  The job always reports, so the required check never goes missing; this only decides
  whether it does the analysis. Pushes to `main` and the weekly schedule always analyse.
- **local_upgrade**: `P14 local upgrade` (13 to 15 minutes on three systems) runs in
  the nightly and on demand; a pull request gets it only through the `full-ci` label.

Selection must never be optimistic: when a rule is unsure it chooses more, and the
nightly run on `main` runs everything regardless. Nothing here runs a shell; every
command is an explicit argument list, and file names from the change are never
written to an output (only package names from `cargo metadata` and fixed words).
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass, field
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
from typing import Iterable, Mapping, Sequence

REPOSITORY = Path(__file__).resolve().parent.parent

SYSTEMS_UBUNTU = ("ubuntu-latest",)
SYSTEMS_ALL = ("ubuntu-latest", "windows-latest", "macos-latest")

# Requesting the full lane on a pull request: add this label (it stays until removed).
# It also asks for `P14 local upgrade`, which no other pull request runs.
FULL_LABEL = "full-ci"

# What CodeQL reads when it analyses Rust without a build (`build-mode: none`): every
# Rust source file wherever it is (the workspace, the excluded fuzz harness, the tools),
# the manifests and lockfile it resolves dependencies from, the toolchain, the Cargo
# configuration, and the workflow and configuration that say how it runs. Nothing else
# can change what it reports, so a change to none of these needs no analysis.
CODEQL_FILES = frozenset(
    {
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "rust-toolchain",
        ".github/workflows/codeql.yml",
    }
)
CODEQL_PREFIXES = ("crates/", "fuzz/", ".cargo/", ".github/codeql/")
CODEQL_FILE_NAMES = frozenset({"Cargo.toml", "Cargo.lock", "build.rs"})
CODEQL_SUFFIXES = (".rs",)


def reads_rust(path: str) -> bool:
    """Whether CodeQL's Rust analysis reads `path` (a repository-relative POSIX path)."""
    posix = PurePosixPath(path).as_posix()
    return (
        posix in CODEQL_FILES
        or posix.startswith(CODEQL_PREFIXES)
        or PurePosixPath(posix).name in CODEQL_FILE_NAMES
        or posix.endswith(CODEQL_SUFFIXES)
    )


def codeql_selected(changed: Sequence[str], *, force_full: bool = False) -> bool:
    """Whether a change needs the CodeQL analysis.

    A request for everything, or a change that lists no files, selects it: when the
    change cannot be read, nothing can be excluded.
    """
    return force_full or not changed or any(reads_rust(path) for path in changed)

# Exact paths whose change selects the full lane.
FULL_FILES = frozenset(
    {
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "rustfmt.toml",
        ".gitattributes",
        "tools/ci_lanes.py",
        "tools/test_ci_lanes.py",
    }
)
# Path prefixes whose change selects the full lane.
FULL_PREFIXES = (".github/workflows/", ".github/actions/", ".cargo/")
# File names that select the full lane wherever they are in the workspace: a member's
# manifest can change features that other members unify with, and a build script can
# change what any dependent compiles.
FULL_FILE_NAMES = frozenset({"Cargo.toml", "build.rs"})

# The fuzz harness is its own workspace (`fuzz/`, excluded from the root one), so
# `cargo metadata` of the root does not list it. It is treated as one more dependent
# of the workspace packages its manifest names by path (`fuzz_dependencies`), and its
# replay test reads files of other folders (the `fuzz` rows of `SHARED_INPUTS`).
FUZZ_DIRECTORY = "fuzz/"
FUZZ_MANIFEST = "fuzz/Cargo.toml"
FUZZ_PATH_DEPENDENCY = re.compile(
    r'^([A-Za-z0-9_-]+)\s*=\s*\{[^}\n]*\bpath\s*=\s*"\.\./crates/[^"]+"', re.MULTILINE
)


def fuzz_dependencies(manifest: str) -> frozenset[str]:
    """The workspace packages the fuzz harness depends on by path, in any section."""
    return frozenset(FUZZ_PATH_DEPENDENCY.findall(manifest))


@dataclass(frozen=True)
class SharedInput:
    """Files outside a package's folder that its tests or build read."""

    prefix: str
    packages: frozenset[str]
    fuzz: bool = False
    npm: bool = False
    platform: bool = False


# Found by reading every package's sources and tests for paths they open outside
# their own folder (`include_str!`, `include_bytes!`, `CARGO_MANIFEST_DIR` joins and
# the repository-root helpers). A test that starts reading another shared file needs
# a row here; until it has one, only the nightly run catches a change to that file
# (known limit L-168).
SHARED_INPUTS: tuple[SharedInput, ...] = (
    SharedInput(
        "schemas/",
        frozenset(
            {"vsift-contract", "vsift", "vsift-cli", "vsift-infrastructure", "vsift-release"}
        ),
    ),
    SharedInput(
        "fixtures/",
        frozenset(
            {
                "vsift",
                "vsift-cli",
                "vsift-infrastructure",
                "vsift-agent-trials",
                "vsift-governance",
            }
        ),
        fuzz=True,
    ),
    SharedInput(
        "skills/",
        frozenset({"vsift-contract", "vsift-cli", "vsift-agent-trials", "vsift-release"}),
        fuzz=True,
    ),
    SharedInput("docs/contracts/", frozenset({"vsift-cli"})),
    SharedInput("docs/agents/", frozenset({"vsift-cli", "vsift-agent-trials"})),
    SharedInput("docs/planning/p12-agent-trials/", frozenset({"vsift-agent-trials"})),
    SharedInput("docs/planning/p14-agent-trials/", frozenset({"vsift-agent-trials"})),
    SharedInput("docs/operations/release.md", frozenset({"vsift-release"})),
    SharedInput(
        "npm/",
        frozenset({"vsift-release", "vsift-agent-trials"}),
        npm=True,
        platform=True,
    ),
    SharedInput("LICENSE", frozenset({"vsift-release"})),
    SharedInput("tools/send-console-ctrl.ps1", frozenset({"vsift-cli"}), platform=True),
    # Test data of one crate that the tests of another crate read.
    SharedInput(
        "crates/vsift-infrastructure/tests/",
        frozenset({"vsift-contract", "vsift"}),
        fuzz=True,
    ),
    SharedInput("crates/vsift-contract/tests/", frozenset(), fuzz=True),
)

# Paths that no job of the fast gate other than Documentation and Governance reads:
# prose, the work record, repository metadata, and tools that have workflows of their
# own (the Guide, the P14 workflows, the Python drivers). Governance reads many of
# these and runs on every change.
DOCUMENT_PREFIXES = (
    "docs/",
    "memory/",
    ".github/ISSUE_TEMPLATE/",
    "tools/guide/",
    "tools/p14-campaigns/",
    "tools/p14-published/",
)
DOCUMENT_FILES = frozenset(
    {
        "AGENTS.md",
        "CHANGELOG.md",
        "CODE_OF_CONDUCT.md",
        "CONTRIBUTING.md",
        "README.md",
        "SECURITY.md",
        "SUPPORT.md",
        ".editorconfig",
        ".gitignore",
        "deny.toml",
        ".github/CODEOWNERS",
        ".github/PULL_REQUEST_TEMPLATE.md",
        ".github/dependabot.yml",
    }
)
# Python drivers and their tests at the top of `tools/`: each has its own workflow.
DOCUMENT_PATTERN = re.compile(r"^tools/[A-Za-z0-9_.-]+\.(py|txt)$")

# Platform-sensitive code: a Rust file (before or after the change) containing one
# of these is run on Windows and macOS as well as Ubuntu. They mark code whose
# behaviour differs by operating system: conditional compilation on the target, the
# standard library's OS-specific extensions, the capability-based filesystem,
# process-group, permission and ACL crates, signals, file locks, permissions, links
# and path canonicalisation. Measured on 2026-10-16 they select 118 of the 476 Rust
# files (process supervision, private folders and roots, the session and managed
# stores, the archive readers, the signal handling, the worker, the platform tests);
# plain spawning of the built binary is not a marker, so the CLI's contract tests
# count only where they hold platform code of their own.
PLATFORM_MARKERS = re.compile(
    r"cfg!?\s*\(\s*(?:all\s*\(|any\s*\(|not\s*\()*\s*(?:windows|unix|target_os|target_family|target_vendor)"
    r"|\bstd::os::"
    r"|\b(?:cap_std|cap_fs_ext|rustix|windows_sys|windows_acl|libc|process_wrap)::"
    r"|\btokio::signal\b"
    r"|\b(?:try_lock|lock_shared|lock_exclusive|try_lock_shared)\s*\("
    r"|\bset_permissions\b|\bPermissionsExt\b"
    r"|\b(?:symlink|hard_link|canonicalize)\b"
)
# Non-Rust files that are platform-sensitive by nature.
PLATFORM_PATHS = ("npm/", "tools/vsift-agent-trials/campaigns/")
PLATFORM_SUFFIXES = (".ps1", ".cmd", ".bat", ".sh")

# The worker-boundary job runs the process supervisor's tests inside a hardened
# container; any change to these runs it on the pull request.
WORKER_BOUNDARY_PATHS = (
    "crates/vsift-infrastructure/src/process_supervisor.rs",
    "crates/vsift-infrastructure/src/host_attestation.rs",
    "crates/vsift-infrastructure/tests/process_supervisor.rs",
)


@dataclass(frozen=True)
class Package:
    """A workspace member: its name, its folder and the members it depends on."""

    name: str
    directory: str
    dependencies: frozenset[str]


@dataclass
class Selection:
    """What the fast gate runs, and why."""

    lane: str = "docs"
    packages: set[str] = field(default_factory=set)
    platform: bool = False
    fuzz: bool = False
    npm: bool = False
    worker_boundary: bool = False
    codeql: bool = False
    local_upgrade: bool = False
    reasons: list[str] = field(default_factory=list)

    def outputs(self, workspace: Sequence[Package]) -> dict[str, str]:
        """The values the workflow reads; only fixed words and package names."""
        full = self.lane == "full"
        names = sorted(package.name for package in workspace) if full else sorted(self.packages)
        systems = SYSTEMS_ALL if full or self.platform else SYSTEMS_UBUNTU
        return {
            "lane": self.lane,
            "quality": json.dumps(bool(names)),
            "packages": " ".join(names),
            "package_arguments": " ".join(f"--package {name}" for name in names),
            # Never empty: a matrix of no systems is an error even in a skipped job.
            "systems": json.dumps(list(systems)),
            "platform": json.dumps(full or self.platform),
            "fuzz": json.dumps(full or self.fuzz),
            "npm": json.dumps(full or self.npm),
            "worker_boundary": json.dumps(full or self.worker_boundary),
            # Not part of the lanes: the full lane of a workflow-only change does not
            # analyse Rust, and only the label (not the lane) asks for the upgrade run.
            "codeql": json.dumps(self.codeql),
            "local_upgrade": json.dumps(self.local_upgrade),
        }


def workspace_packages(metadata: Mapping[str, object]) -> list[Package]:
    """The workspace members of `cargo metadata --no-deps` output."""
    root = Path(str(metadata["workspace_root"]))
    members = set(metadata["workspace_members"])  # type: ignore[arg-type]
    raw = [package for package in metadata["packages"] if package["id"] in members]  # type: ignore[union-attr]
    names = {package["name"] for package in raw}
    packages = []
    for package in raw:
        directory = Path(package["manifest_path"]).parent.relative_to(root).as_posix()
        dependencies = frozenset(
            dependency["name"]
            for dependency in package["dependencies"]
            if dependency["name"] in names and dependency.get("path")
        )
        packages.append(Package(package["name"], directory + "/", dependencies))
    return packages


def dependents(packages: Sequence[Package], touched: Iterable[str]) -> set[str]:
    """`touched` and every member that depends on one of them, transitively."""
    reverse: dict[str, set[str]] = {package.name: set() for package in packages}
    for package in packages:
        for dependency in package.dependencies:
            reverse.setdefault(dependency, set()).add(package.name)
    selected = set(touched)
    queue = list(selected)
    while queue:
        name = queue.pop()
        for dependent in reverse.get(name, ()):
            if dependent not in selected:
                selected.add(dependent)
                queue.append(dependent)
    return selected


def is_document(path: str) -> bool:
    return (
        path in DOCUMENT_FILES
        or path.startswith(DOCUMENT_PREFIXES)
        or DOCUMENT_PATTERN.match(path) is not None
    )


def is_platform_sensitive(path: str, texts: Iterable[str]) -> bool:
    if path.startswith(PLATFORM_PATHS) or path.endswith(PLATFORM_SUFFIXES):
        return True
    if not path.endswith(".rs"):
        return False
    return any(PLATFORM_MARKERS.search(text) for text in texts)


def select(
    changed: Sequence[str],
    packages: Sequence[Package],
    texts: Mapping[str, Sequence[str]],
    *,
    fuzz_depends_on: frozenset[str],
    labels: Iterable[str] = (),
    force_full: bool = False,
) -> Selection:
    """The lanes for `changed` (repository-relative POSIX paths).

    `texts` holds, for each changed Rust file, its contents before and after the
    change (whichever exist), for the platform markers.
    """
    selection = Selection()
    selection.codeql = codeql_selected(changed, force_full=force_full)
    label_names = set(labels)
    selection.local_upgrade = FULL_LABEL in label_names
    if force_full:
        selection.lane = "full"
        selection.reasons.append("the full run was requested")
    if FULL_LABEL in label_names:
        selection.lane = "full"
        selection.reasons.append(f"the pull request has the {FULL_LABEL} label")
    if not changed:
        selection.lane = "full"
        selection.reasons.append("the change lists no files, so nothing can be excluded")
    by_directory = sorted(packages, key=lambda package: len(package.directory), reverse=True)
    touched: set[str] = set()
    for path in changed:
        posix = PurePosixPath(path).as_posix()
        if posix in FULL_FILES or posix.startswith(FULL_PREFIXES):
            selection.lane = "full"
            selection.reasons.append(f"a build-wide file changed ({_area(posix)})")
            continue
        owner = next((p for p in by_directory if posix.startswith(p.directory)), None)
        if owner is not None and PurePosixPath(posix).name in FULL_FILE_NAMES:
            selection.lane = "full"
            selection.reasons.append(f"a manifest or build script of {owner.name} changed")
            continue
        known = False
        if owner is not None:
            touched.add(owner.name)
            known = True
        if posix.startswith(FUZZ_DIRECTORY):
            selection.fuzz = True
            known = True
        for shared in SHARED_INPUTS:
            if posix.startswith(shared.prefix):
                touched.update(shared.packages)
                selection.fuzz |= shared.fuzz
                selection.npm |= shared.npm
                selection.platform |= shared.platform
                known = True
        if is_platform_sensitive(posix, texts.get(posix, ())):
            selection.platform = True
        if posix.startswith(WORKER_BOUNDARY_PATHS):
            selection.worker_boundary = True
        if not known and not is_document(posix):
            selection.lane = "full"
            selection.reasons.append(f"no rule covers a changed path ({_area(posix)})")
    affected = dependents(packages, touched)
    if affected & fuzz_depends_on:
        selection.fuzz = True
    selection.packages = affected
    if selection.lane != "full" and (affected or selection.fuzz or selection.npm):
        selection.lane = "crates"
    if selection.lane == "crates":
        selection.reasons.append(
            "packages: " + (", ".join(sorted(affected)) if affected else "none")
        )
        if selection.platform:
            selection.reasons.append("platform-sensitive files changed: Windows and macOS run too")
    if selection.lane == "docs":
        selection.reasons.append("only documentation and separately tested tools changed")
    return selection


def _area(path: str) -> str:
    """The top folder of a path, for a reason line that names no file of the change."""
    parts = PurePosixPath(path).parts
    if len(parts) < 2:
        return "a top-level file"
    return parts[0] if re.fullmatch(r"[A-Za-z0-9._-]{1,40}", parts[0]) else "a folder"


def _git(arguments: Sequence[str]) -> str:
    return subprocess.run(
        ["git", *arguments],
        cwd=REPOSITORY,
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
    ).stdout


def _show(revision: str, path: str) -> str | None:
    result = subprocess.run(
        ["git", "show", f"{revision}:{path}"],
        cwd=REPOSITORY,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    return result.stdout if result.returncode == 0 else None


def changed_files(base: str, head: str) -> list[str]:
    """Files that differ between the merge base of `base` and `head`, and `head`."""
    output = _git(["diff", "--name-only", "--no-renames", "-z", f"{base}...{head}"])
    return [path for path in output.split("\0") if path]


def contents(base: str, head: str, paths: Iterable[str]) -> dict[str, list[str]]:
    texts: dict[str, list[str]] = {}
    merge_base = _git(["merge-base", base, head]).strip()
    for path in paths:
        if path.endswith(".rs"):
            texts[path] = [
                text for text in (_show(merge_base, path), _show(head, path)) if text is not None
            ]
    return texts


def cargo_metadata() -> dict[str, object]:
    output = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
        cwd=REPOSITORY,
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
    ).stdout
    return json.loads(output)


CODEQL_SUMMARY_ANALYSING = (
    "### CodeQL: analysing\n\n"
    "The change touches something CodeQL reads, or lists no files.\n"
)
CODEQL_SUMMARY_SKIPPED = (
    "### CodeQL: no analysis needed\n\n"
    "The change touches no Rust source, Cargo manifest or lockfile, toolchain file, "
    "or CodeQL workflow or configuration. `main` is analysed on every push and weekly.\n"
)


def main_codeql(arguments: argparse.Namespace) -> int:
    """The `--codeql-only` mode: the one decision the CodeQL workflow needs.

    It needs neither the toolchain nor the workspace graph, so the job that asks
    stays a few seconds long on a change that needs no analysis.
    """
    force_full = arguments.full or not arguments.base or set(arguments.base) == {"0"}
    changed = [] if force_full else changed_files(arguments.base, arguments.head)
    selected = codeql_selected(changed, force_full=force_full)
    line = f"codeql={json.dumps(selected)}"
    if arguments.output:
        with arguments.output.open("a", encoding="utf-8") as handle:
            handle.write(line + "\n")
    print(line)
    if arguments.summary:
        text = CODEQL_SUMMARY_ANALYSING if selected else CODEQL_SUMMARY_SKIPPED
        with arguments.summary.open("a", encoding="utf-8") as handle:
            handle.write(text)
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", help="the base revision (a pull request's base, or the push's before)")
    parser.add_argument("--head", default="HEAD", help="the revision under test")
    parser.add_argument("--labels", default="[]", help="the pull request's labels, as a JSON list of names")
    parser.add_argument("--full", action="store_true", help="select the full lane")
    parser.add_argument(
        "--codeql-only",
        action="store_true",
        help="decide only whether CodeQL analyses (no cargo metadata needed), and write only `codeql`",
    )
    parser.add_argument("--output", type=Path, help="append name=value lines here (GITHUB_OUTPUT)")
    parser.add_argument("--summary", type=Path, help="append a Markdown summary here (GITHUB_STEP_SUMMARY)")
    arguments = parser.parse_args(argv)

    if arguments.codeql_only:
        return main_codeql(arguments)

    packages = workspace_packages(cargo_metadata())
    try:
        decoded = json.loads(arguments.labels)
    except json.JSONDecodeError:
        decoded = None
    # A push has no pull request, so no labels (`null`).
    labels = [str(label) for label in decoded] if isinstance(decoded, list) else []
    force_full = arguments.full or not arguments.base or set(arguments.base) == {"0"}
    if force_full:
        changed: list[str] = []
        texts: dict[str, list[str]] = {}
    else:
        changed = changed_files(arguments.base, arguments.head)
        texts = contents(arguments.base, arguments.head, changed)
    manifest = (REPOSITORY / FUZZ_MANIFEST).read_text(encoding="utf-8")
    selection = select(
        changed,
        packages,
        texts,
        fuzz_depends_on=fuzz_dependencies(manifest),
        labels=labels,
        force_full=force_full,
    )
    outputs = selection.outputs(packages)

    lines = [f"{name}={value}" for name, value in outputs.items()]
    if arguments.output:
        with arguments.output.open("a", encoding="utf-8") as handle:
            handle.write("\n".join(lines) + "\n")
    print("\n".join(lines))
    reasons = list(dict.fromkeys(selection.reasons))
    for reason in reasons:
        print(f"reason: {reason}")
    if arguments.summary:
        with arguments.summary.open("a", encoding="utf-8") as handle:
            handle.write(f"### CI lane: {outputs['lane']}\n\n")
            for reason in reasons:
                handle.write(f"- {reason}\n")
            handle.write(f"\nSystems: {', '.join(json.loads(outputs['systems'])) or 'none'}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
