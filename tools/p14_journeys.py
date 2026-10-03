"""Drive the P14 journeys against a published vsift binary (RQ-05, RQ-06; P14 PR 3).

Only a disposable GitHub Actions runner should run this. The workflow `P14
journeys` and the published-binary mode of `P13 managed smoke` call it, one
subcommand per step, with the work folder as the only shared state:

    resolve-version  the version to qualify: the one asked for, or the highest published
    select-source    which checkout the tests are compiled from, and the tag's commit
    install          the published `vsift-cli` from the real npm registry into a fresh
                     folder, its native executable located and checked
    stage-tools      the tools one system uses (Ubuntu: installed by the published binary
                     itself; Windows: the repository's pinned builds; macOS: Homebrew's)
    run              the real-tool checkpoints against the installed binary
    summarize        the job summary and the results file of one system
    report           the aggregate of every system's results

Nothing here installs anything on a developer's machine, publishes, tags or uses a
credential. Every external command is an explicit argument list; no shell runs and
no untrusted text reaches a command string. The only network use is npm's own client
reading the public registry, the publishers' pinned files (as the repository's other
staging scripts download them) and Homebrew on macOS.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
from typing import Any

PACKAGE = "vsift-cli"
PLACEHOLDER_VERSION = "0.0.0"
OVERRIDE_MODULE = "crates/vsift-cli/tests/published_binary/mod.rs"
SEMVER = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$"
)
HEX_COMMIT = re.compile(r"^[0-9a-f]{40}$")
VERSION_LINE = re.compile(r"^vsift (\S+) \(([0-9a-f]{12})\)$")
# The launcher's own table of native packages (npm/vsift-cli/lib/launcher.cjs).
NATIVE_PACKAGES = {
    ("linux", "x64"): ("@vsift/linux-x64", "vsift"),
    ("darwin", "arm64"): ("@vsift/darwin-arm64", "vsift"),
    ("win32", "x64"): ("@vsift/win32-x64", "vsift.exe"),
}
SYSTEMS = ("ubuntu", "windows", "macos")
# Homebrew's names for whisper.cpp, in the order they are tried.
WHISPER_FORMULAE = ("whisper-cpp", "whisper.cpp")
RESULTS_FORMAT = 1
# The longest checkpoint takes under eight minutes on any system (measured), so a
# checkpoint that is still running after this is stuck, not slow; the T-04 gate on a
# 3-CPU runner takes about an hour and is given its own limit.
CHECKPOINT_TIMEOUT_SECONDS = 45 * 60
SLOW_CHECKPOINT_TIMEOUT_SECONDS = 100 * 60
INSTALL_TIMEOUT_SECONDS = 40 * 60
BUILD_TIMEOUT_SECONDS = 60 * 60


class Failure(Exception):
    """A step that must fail the job, with the reason in plain words."""


class Checkpoint:
    """One opt-in real-tool test target and what it needs."""

    def __init__(self, key: str, test: str, needs_whisper: bool, what: str,
                 environment: dict[str, str] | None = None, package: str = "vsift-cli",
                 systems: tuple[str, ...] = ("ubuntu", "windows", "macos")) -> None:
        self.key = key
        self.test = test
        self.needs_whisper = needs_whisper
        self.what = what
        self.environment = environment or {}
        # A checkpoint of another package drives the engine library in this process,
        # not the installed binary: it is a gate on the tools, and is labelled so.
        self.package = package
        self.systems = systems


# The order is the cost order: the cheap setup journeys first.
CHECKPOINTS = (
    Checkpoint("p06_setup", "p06_setup_e2e", False,
               "dependency detect, select, verify and guide (P06)"),
    Checkpoint("p07_transcript", "p07_transcript_e2e", False,
               "the supplied-transcript journey (P07, A-09)"),
    Checkpoint("p07_local_asr", "p07_local_asr_e2e", True,
               "the local-ASR journey (P07, A-08)"),
    Checkpoint("p08_search", "p08_search_e2e", False,
               "search over a supplied transcript (P08)"),
    Checkpoint("p08_candidates", "p08_candidates_e2e", True,
               "visual candidates, recall and the S-11 timings (P08)"),
    Checkpoint("p09_evidence", "p09_evidence_e2e", True,
               "frames, neighbours, bursts, crops, audio and the two mechanical journeys (P09)"),
    Checkpoint("p10_recovery", "p10_recovery_e2e", True,
               "kill, interrupt, resume and replay (P10)"),
    Checkpoint("p11_worker", "p11_worker_e2e", True,
               "batch, admission, shutdown and the durable stage (P11)"),
    Checkpoint("p14_installed", "p14_installed_binary_e2e", False,
               "hostile file names and a sentinel environment (P14, SEC-01, SEC-25)",
               {"VSIFT_P14_INSTALLED_E2E": "1"}),
    # The weekly `P07 local ASR` workflow runs this on Ubuntu and Windows with the pinned
    # tools; macOS has no reviewed whisper.cpp build, so the gates are measured here, on
    # Homebrew's build. It drives the engine library, not the installed binary.
    Checkpoint("p07_asr_gates", "p07_asr_qualification", True,
               "the T-04 accuracy, timing and memory gates of the recognizer, in process "
               "through the engine library (NOT the installed binary: a gate on the tools)",
               package="vsift-infrastructure", systems=("macos",)),
)

# Checkpoints that exist and do not run here, with the reason, so that nothing is
# skipped silently. The managed-install checkpoints are RQ-06's.
NOT_RUN_ELSEWHERE = {
    "ubuntu": [
        ("p13_install_e2e / p13_managed_install_real",
         "run by `P13 managed smoke` (RQ-06) against the same published binary; the tools "
         "here are installed by its `setup install` once"),
        ("p07_asr_qualification (the T-04 gates)",
         "run weekly by `P07 local ASR`, in process through the engine library, with the "
         "pinned tools"),
    ],
    "windows": [
        ("p13_install_e2e / p13_managed_install_real",
         "not applicable: managed installation exists on Ubuntu 24.04 x64 only "
         "(ADR 0023 decision E)"),
        ("p07_asr_qualification (the T-04 gates)",
         "run weekly by `P07 local ASR`, in process through the engine library, with the "
         "pinned tools"),
    ],
    "macos": [
        ("p13_install_e2e / p13_managed_install_real",
         "not applicable: managed installation exists on Ubuntu 24.04 x64 only "
         "(ADR 0023 decision E)"),
    ],
}

SYSTEM_LABELS = {
    "ubuntu": "Ubuntu 24.04 x64: tools installed by the published binary's own `setup install`",
    "windows": "Windows x64: the repository's pinned FFmpeg (BtbN win64 LGPL 9.0.1), "
               "whisper.cpp v1.9.2 and the base model",
    "macos": "macOS 15 arm64: Homebrew's FFmpeg and whisper.cpp (not reviewed artifacts) "
             "with the pinned base model",
}


# ---------------------------------------------------------------- versions ----

def semver_key(text: str) -> tuple[Any, ...]:
    """A sort key that follows SemVer precedence, or raise `Failure`."""
    found = SEMVER.match(text)
    if found is None:
        raise Failure(f"not a version: {text!r}")
    major, minor, patch, pre_release = found.groups()
    if pre_release is None:
        # A release sorts after every pre-release of the same numbers.
        return (int(major), int(minor), int(patch), 1, ())
    identifiers = tuple(
        (0, int(part), "") if part.isdigit() else (1, 0, part)
        for part in pre_release.split(".")
    )
    return (int(major), int(minor), int(patch), 0, identifiers)


def highest_published(versions: list[str]) -> str:
    """The highest version, never the empty `0.0.0` placeholder that holds `latest`."""
    candidates = [
        version for version in versions
        if SEMVER.match(version) and version != PLACEHOLDER_VERSION
    ]
    if not candidates:
        raise Failure("no published version of vsift-cli other than the placeholder")
    return max(candidates, key=semver_key)


def validated_version(text: str) -> str:
    """A version that is safe to put in a command line, or raise `Failure`."""
    if SEMVER.match(text) is None:
        raise Failure(f"not a version: {text!r}")
    return text


# ------------------------------------------------------------------ helpers ----

def run(argv: list[str], *, cwd: Path | None = None, env: dict[str, str] | None = None,
        timeout: float | None = 600, check: bool = True) -> subprocess.CompletedProcess[str]:
    """Run one command with an explicit argument list and capture its text."""
    result = subprocess.run(
        argv, cwd=cwd, env=env, capture_output=True, text=True, errors="replace",
        timeout=timeout, check=False,
    )
    if check and result.returncode != 0:
        tail = (result.stderr or result.stdout).strip()[-600:]
        raise Failure(f"`{' '.join(argv)[:200]}` failed with exit {result.returncode}: {tail}")
    return result


def git(arguments: list[str], cwd: Path) -> str:
    """One line from `git`, with a clean message when it fails."""
    return run(["git", *arguments], cwd=cwd, timeout=60).stdout.strip()


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise Failure(f"{path.name} could not be read: {error}") from error


def append_output(name: str, value: str) -> None:
    """Write one `name=value` line to the step's output file when there is one."""
    if "\n" in value or "\r" in value:
        raise Failure("a step output must be one line")
    target = os.environ.get("GITHUB_OUTPUT")
    if target:
        with open(target, "a", encoding="utf-8") as handle:
            handle.write(f"{name}={value}\n")
    print(f"{name}={value}")


def append_environment(name: str, value: str) -> None:
    """Write one variable for the later steps of the job."""
    if "\n" in value or "\r" in value:
        raise Failure("an environment value must be one line")
    target = os.environ.get("GITHUB_ENV")
    if not target:
        raise Failure("GITHUB_ENV is not set: this must run inside a workflow step")
    with open(target, "a", encoding="utf-8") as handle:
        handle.write(f"{name}={value}\n")


def append_summary(markdown: str) -> None:
    """Add to the job summary, or print when run by hand."""
    target = os.environ.get("GITHUB_STEP_SUMMARY")
    if target:
        with open(target, "a", encoding="utf-8") as handle:
            handle.write(markdown + "\n")
    else:
        print(markdown)


def current_system() -> str:
    if sys.platform.startswith("linux"):
        return "ubuntu"
    if sys.platform == "win32":
        return "windows"
    if sys.platform == "darwin":
        return "macos"
    raise Failure(f"no journey is defined for {sys.platform}")


def native_package() -> tuple[str, str]:
    """The native package and executable name for this machine."""
    machine = platform.machine().lower()
    arch = {"x86_64": "x64", "amd64": "x64", "arm64": "arm64", "aarch64": "arm64"}.get(machine)
    key = ("win32" if sys.platform == "win32" else
           "darwin" if sys.platform == "darwin" else "linux", arch)
    if key not in NATIVE_PACKAGES:
        raise Failure(f"vsift has no native package for {key[0]} {key[1]}")
    return NATIVE_PACKAGES[key]


def say(text: str) -> None:
    """One progress line, flushed at once: a stalled step must show where it stalled."""
    print(text, flush=True)


KILL_WAIT_SECONDS = 30
DRAIN_WAIT_SECONDS = 30
HEARTBEAT_SECONDS = 300


def kill_tree(process: subprocess.Popen[bytes]) -> bool:
    """Stop a command and everything it started, without ever waiting without a limit.

    Returns whether the command itself ended. (A stopped job leaves children, and on
    Windows a kill that cannot finish must not take the whole job with it.)
    """
    try:
        if sys.platform == "win32":
            subprocess.run(["taskkill", "/T", "/F", "/PID", str(process.pid)],
                           capture_output=True, check=False, timeout=KILL_WAIT_SECONDS)
        else:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                process.kill()
    except subprocess.TimeoutExpired:
        say("the kill command did not answer in time")
    try:
        process.wait(timeout=KILL_WAIT_SECONDS)
    except subprocess.TimeoutExpired:
        return False
    return True


def run_logged(argv: list[str], log: Path, *, cwd: Path, env: dict[str, str],
               timeout: float) -> tuple[int | None, float, bool]:
    """Run a command, copy its output to the job log and to `log`.

    Returns the exit code (None when the deadline stopped it, or the command could not
    be stopped), the seconds taken, and whether its output reached the end. A descendant
    that outlives the command and keeps the output pipe open leaves the last answer
    False: the pipe is then left alone (closing it from here waits for the reader, which
    waits for that descendant) and the caller says so.
    """
    log.parent.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    options: dict[str, Any] = {}
    if sys.platform != "win32":
        options["start_new_session"] = True
    process = subprocess.Popen(
        argv, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, **options,
    )

    def pump() -> None:
        assert process.stdout is not None
        with log.open("w", encoding="utf-8", errors="replace") as handle:
            for raw in iter(process.stdout.readline, b""):
                text = raw.decode("utf-8", errors="replace")
                handle.write(text)
                handle.flush()
                sys.stdout.write(text)
                sys.stdout.flush()

    thread = threading.Thread(target=pump, daemon=True)
    thread.start()
    finished = threading.Event()

    def heartbeat() -> None:
        # A quiet command (a long recognition prints nothing) is not a stalled one.
        while not finished.wait(HEARTBEAT_SECONDS):
            say(f"... still running after {round((time.monotonic() - started) / 60)} minutes: "
                f"{' '.join(argv[:6])}")

    threading.Thread(target=heartbeat, daemon=True).start()
    code: int | None
    try:
        code = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        say(f"deadline of {round(timeout / 60)} minutes reached; stopping the command")
        code = None
        if not kill_tree(process):
            say("the command could not be stopped; leaving it")
    finished.set()
    thread.join(timeout=DRAIN_WAIT_SECONDS)
    drained = not thread.is_alive()
    if drained and process.stdout is not None:
        process.stdout.close()
    elif not drained:
        say("a descendant of the command still holds its output open; not waiting for it")
    return code, time.monotonic() - started, drained


# ------------------------------------------------------------ resolve-version ----

def command_resolve_version(arguments: argparse.Namespace) -> None:
    """Print and export the version to qualify."""
    requested = (arguments.requested or "").strip()
    npm = shutil.which("npm")
    if npm is None:
        raise Failure("npm is not available")
    if requested and requested != "highest":
        version = validated_version(requested)
    else:
        listed = json.loads(run([npm, "view", PACKAGE, "versions", "--json"]).stdout)
        version = highest_published(listed if isinstance(listed, list) else [listed])
    # The published packages of this version must all exist: the launcher and the
    # three native packages it depends on.
    manifest = json.loads(run([npm, "view", f"{PACKAGE}@{version}", "optionalDependencies",
                               "--json"]).stdout)
    if not isinstance(manifest, dict) or not manifest:
        raise Failure(f"{PACKAGE}@{version} lists no native package")
    for name, wanted in sorted(manifest.items()):
        if wanted != version:
            raise Failure(f"{PACKAGE}@{version} depends on {name}@{wanted}")
        run([npm, "view", f"{name}@{version}", "version"])
    tags = json.loads(run([npm, "view", PACKAGE, "dist-tags", "--json"]).stdout)
    append_output("version", version)
    append_output("tag", f"v{version}")
    append_output("dist_tags", json.dumps(tags, sort_keys=True, separators=(",", ":")))


# ---------------------------------------------------------------- select-source ----

def command_select_source(arguments: argparse.Namespace) -> None:
    """Choose the checkout the tests are compiled from and record the tag's commit."""
    version = validated_version(arguments.version)
    work = Path(arguments.work)
    tag_source = Path(arguments.tag_source).resolve()
    workflow_source = Path(arguments.workflow_source).resolve()
    tag_commit = git(["rev-parse", "HEAD"], tag_source)
    pointed = git(["rev-parse", f"refs/tags/v{version}^{{commit}}"], tag_source)
    if tag_commit != pointed or not HEX_COMMIT.match(tag_commit):
        raise Failure(f"the checkout of v{version} is not at that tag's commit")
    workflow_commit = git(["rev-parse", "HEAD"], workflow_source)
    has_override = (tag_source / OVERRIDE_MODULE).is_file()
    wanted = arguments.tests_from
    if wanted == "tag" and not has_override:
        raise Failure(f"v{version} predates the binary override, so its tests cannot run an "
                      "installed binary; use the workflow ref's tests")
    use_tag = wanted == "tag" or (wanted == "auto" and has_override)
    chosen = tag_source if use_tag else workflow_source
    state = {
        "version": version,
        "system": current_system(),
        "tag_commit": tag_commit,
        "workflow_commit": workflow_commit,
        "tag_has_override": has_override,
        "tests_from": "tag" if use_tag else "workflow_ref",
        "tests_root": str(chosen),
        "tag_source": str(tag_source),
    }
    write_json(work / "state.json", state)
    print(json.dumps(state, indent=2, sort_keys=True))
    note = ("the tests are the tag's own" if use_tag else
            f"v{version} predates the binary override, so the tests are compiled from the "
            f"workflow ref ({workflow_commit[:12]}) and run the binary published as "
            f"{version} ({tag_commit[:12]})")
    append_summary(f"### Test source\n\n{note}.\n")


# ---------------------------------------------------------------------- install ----

def load_state(work: Path) -> dict[str, Any]:
    state = read_json(work / "state.json")
    if not isinstance(state, dict) or "version" not in state:
        raise Failure("the work folder holds no selected source: run select-source first")
    return state


def check_version_output(text: str, version: str, commit: str) -> str:
    """The `--version` line of a binary, if it names this version and commit."""
    line = text.strip()
    found = VERSION_LINE.match(line)
    if found is None:
        raise Failure(f"`--version` printed {line!r}, not `vsift <version> (<commit>)`")
    if found.group(1) != version:
        raise Failure(f"the binary is version {found.group(1)}, not {version}")
    if not commit.startswith(found.group(2)):
        raise Failure(f"the binary names commit {found.group(2)}, which is not v{version}'s "
                      f"({commit[:12]})")
    return line


def lock_entry(lock: dict[str, Any], package: str) -> dict[str, str]:
    """The resolved URL and integrity npm recorded for an installed package."""
    entry = lock.get("packages", {}).get(f"node_modules/{package}")
    if not isinstance(entry, dict):
        raise Failure(f"npm installed no {package}")
    resolved, integrity = entry.get("resolved"), entry.get("integrity")
    if not isinstance(resolved, str) or not resolved.startswith("https://registry.npmjs.org/"):
        raise Failure(f"{package} did not come from the real registry: {resolved!r}")
    if not isinstance(integrity, str) or not integrity.startswith("sha512-"):
        raise Failure(f"{package} has no recorded integrity")
    return {"version": str(entry.get("version")), "resolved": resolved, "integrity": integrity}


def command_install(arguments: argparse.Namespace) -> None:
    """Install the published package into a fresh folder and check its executable."""
    work = Path(arguments.work)
    state = load_state(work)
    version = validated_version(state["version"])
    npm = shutil.which("npm")
    if npm is None:
        raise Failure("npm is not available")
    install = work / "install"
    install.mkdir(parents=True, exist_ok=False)
    write_json(install / "package.json",
               {"name": "p14-published", "version": "0.0.0", "private": True})
    # Scripts are never run; the registry is whatever npm's defaults say, and the lock
    # is checked below to name the public one.
    run([npm, "install", "--prefix", str(install), f"{PACKAGE}@{version}", "--ignore-scripts",
         "--no-audit", "--no-fund", "--loglevel=error"], timeout=INSTALL_TIMEOUT_SECONDS)
    package_name, executable_name = native_package()
    launcher_root = install / "node_modules" / PACKAGE
    native_root = install / "node_modules" / Path(package_name)
    binary = native_root / executable_name
    if not binary.is_file():
        raise Failure(f"the installed {package_name} holds no {executable_name}")
    lock = read_json(install / "package-lock.json")
    entries = {name: lock_entry(lock, name) for name in (PACKAGE, package_name)}
    for name, entry in entries.items():
        if entry["version"] != version:
            raise Failure(f"{name} installed as {entry['version']}, not {version}")
    # The digest the launcher package records for this executable (what its own
    # check compares against) must be the executable's.
    digests = read_json(launcher_root / "platform-digests.json")
    recorded = digests.get("packages", {}).get(package_name, {})
    actual = sha256_of(binary)
    if recorded.get("sha256") != actual or recorded.get("size") != binary.stat().st_size:
        raise Failure("the native executable does not match platform-digests.json")
    if sys.platform != "win32" and not os.access(binary, os.X_OK):
        raise Failure("the installed native executable is not executable")
    output = run([str(binary), "--version"], timeout=60).stdout
    version_line = check_version_output(output, version, state["tag_commit"])
    node = run(["node", "--version"], timeout=60).stdout.strip()
    npm_version = run([npm, "--version"], timeout=60).stdout.strip()
    installed = {
        "version": version,
        "native_package": package_name,
        "executable": executable_name,
        "executable_sha256": actual,
        "executable_bytes": binary.stat().st_size,
        "version_line": version_line,
        "recorded_in_platform_digests": True,
        "tag_commit": state["tag_commit"],
        "lock": entries,
        "node": node,
        "npm": npm_version,
    }
    write_json(work / "out" / "installed.json", installed)
    state["binary"] = str(binary)
    state["version_line"] = version_line
    write_json(work / "state.json", state)
    if arguments.export_environment:
        append_environment("VSIFT_E2E_BINARY", str(binary))
        append_environment("VSIFT_E2E_EXPECTED_VERSION", version)
        append_environment("VSIFT_E2E_EXPECTED_COMMIT", state["tag_commit"])
    append_summary(
        f"### Published binary\n\n`{PACKAGE}@{version}` installed from the real npm registry "
        f"with scripts disabled (Node.js {node}, npm {npm_version}).\n\n"
        f"- native package: `{package_name}`, `{executable_name}` {binary.stat().st_size} bytes, "
        f"SHA-256 `{actual}`, equal to the digest in `platform-digests.json`\n"
        f"- `--version`: `{version_line}`; the tag `v{version}` is {state['tag_commit']}\n"
        f"- npm integrity: `{entries[PACKAGE]['integrity']}` (launcher), "
        f"`{entries[package_name]['integrity']}` (native)\n"
    )
    print(f"installed {version_line}")


# ---------------------------------------------------------------- stage-tools ----

def first_line(argv: list[str]) -> str:
    """The first output line of a version command, or an empty string."""
    result = run(argv, timeout=60, check=False)
    text = (result.stdout or result.stderr).strip()
    return text.splitlines()[0] if text else ""


def tool_record(path: Path, version_argv: list[str] | None) -> dict[str, Any]:
    record: dict[str, Any] = {
        "file": path.name,
        "bytes": path.stat().st_size,
        "sha256": sha256_of(path),
    }
    if version_argv is not None:
        record["version_line"] = first_line(version_argv)
    return record


def find_managed_files(managed_root: Path) -> dict[str, Path]:
    """The four reviewed files of a finished managed install, each found exactly once."""
    wanted = {"ffmpeg": "ffmpeg", "ffprobe": "ffprobe", "whisper_cli": "whisper-cli",
              "whisper_model": "ggml-base.bin"}
    found: dict[str, Path] = {}
    for role, name in wanted.items():
        matches = [
            path for path in managed_root.rglob(name)
            if path.is_file() and not any(part.startswith("stage-") for part in path.parts)
        ]
        if len(matches) != 1:
            raise Failure(f"expected one {name} under the managed folder, found {len(matches)}")
        found[role] = matches[0]
    if found["ffmpeg"].parent != found["ffprobe"].parent:
        raise Failure("ffmpeg and ffprobe are not in one folder")
    return found


def stage_ubuntu(work: Path, state: dict[str, Any]) -> dict[str, Any]:
    """Install the tools with the published binary itself: plan, accept, install."""
    binary = state["binary"]
    home = work / "tools-home"
    home.mkdir(parents=True, exist_ok=False)
    env = {
        "PATH": "", "HOME": str(home),
        "XDG_CONFIG_HOME": str(home / "config"), "XDG_DATA_HOME": str(home / "data"),
        "XDG_CACHE_HOME": str(home / "cache"), "XDG_STATE_HOME": str(home / "state"),
    }
    plan = run([binary, "setup", "plan", "--profile", "desktop", "--json"], env=env)
    plan_path = work / "out" / "plan.json"
    plan_path.parent.mkdir(parents=True, exist_ok=True)
    plan_path.write_text(plan.stdout, encoding="utf-8")
    document = json.loads(plan.stdout)
    digest = document["data"]["plan_digest"]
    actions = document["data"]["actions"]
    if len(actions) != 3 or not re.fullmatch(r"[0-9a-f]{64}", digest):
        raise Failure(f"the plan has {len(actions)} actions or an odd digest: {digest!r}")
    started = time.monotonic()
    install = run([binary, "setup", "install", "--plan", str(plan_path), "--accept-plan", digest,
                   "--json"], env=env, timeout=INSTALL_TIMEOUT_SECONDS, check=False)
    result = json.loads(install.stdout)
    statuses = [component["status"] for component in result["data"]["components"]]
    if install.returncode != 0 or statuses != ["activated"] * 3:
        raise Failure(f"setup install ended with exit {install.returncode}: {statuses} "
                      f"{result.get('error')}")
    check = json.loads(run([binary, "setup", "check", "--json"], env=env).stdout)
    lookups = {item["dependency"]: item["lookup"] for item in check["dependencies"]}
    if check["status"] != "ready" or set(lookups.values()) != {"managed_version"}:
        raise Failure(f"setup check after the install: {check['status']} {lookups}")
    found = find_managed_files(home / "data" / "vsift" / "managed-v1")
    return {
        "provenance": "managed: installed by the published binary's own `setup install` from "
                      "the catalogue's reviewed artifacts",
        "ffmpeg_dir": str(found["ffmpeg"].parent),
        "whisper_cli": str(found["whisper_cli"]),
        "whisper_model": str(found["whisper_model"]),
        "install_seconds": round(time.monotonic() - started, 1),
        "catalogue_revision": document["data"].get("catalogue_revision"),
        "tools": {
            "ffmpeg": tool_record(found["ffmpeg"], [str(found["ffmpeg"]), "-version"]),
            "ffprobe": tool_record(found["ffprobe"], [str(found["ffprobe"]), "-version"]),
            "whisper_cli": tool_record(found["whisper_cli"], None),
            "whisper_model": tool_record(found["whisper_model"], None),
        },
    }


def import_pinned_tools(tests_root: Path) -> Any:
    """The repository's own staging module (its pins and download checks)."""
    tools_directory = str(tests_root / "tools")
    if tools_directory not in sys.path:
        sys.path.insert(0, tools_directory)
    import p07_local_asr_tools  # noqa: PLC0415  (found through the path just added)
    return p07_local_asr_tools


def stage_windows(work: Path, state: dict[str, Any]) -> dict[str, Any]:
    """The pinned builds the repository's Windows jobs use, hash-checked."""
    staging = import_pinned_tools(Path(state["tests_root"]))
    pinned = work / "pinned"
    pinned.mkdir(parents=True, exist_ok=False)
    ffmpeg_bin, whisper = staging.stage_windows(pinned)
    models = staging.stage_models(pinned)
    return {
        "provenance": "pinned: the repository's reviewed Windows builds, each checked against "
                      "its recorded size and SHA-256 (tools/p07_local_asr_tools.py)",
        "ffmpeg_dir": str(ffmpeg_bin),
        "whisper_cli": str(whisper),
        "whisper_model": str(models["base"]),
        "whisper_model_q5_1": str(models["base_q5_1"]),
        "tools": {
            "ffmpeg": tool_record(ffmpeg_bin / "ffmpeg.exe", [str(ffmpeg_bin / "ffmpeg.exe"), "-version"]),
            "ffprobe": tool_record(ffmpeg_bin / "ffprobe.exe", [str(ffmpeg_bin / "ffprobe.exe"), "-version"]),
            "whisper_cli": tool_record(whisper, None),
            "whisper_model": tool_record(models["base"], None),
        },
    }


def stage_macos(work: Path, state: dict[str, Any]) -> dict[str, Any]:
    """Homebrew's FFmpeg and whisper.cpp, with the repository's pinned base model."""
    brew = shutil.which("brew")
    if brew is None:
        raise Failure("Homebrew is not installed on this runner")
    env = dict(os.environ)
    env.update({"HOMEBREW_NO_AUTO_UPDATE": "1", "HOMEBREW_NO_ANALYTICS": "1",
                "HOMEBREW_NO_INSTALL_CLEANUP": "1", "HOMEBREW_NO_ENV_HINTS": "1"})
    # The formula is `whisper-cpp` in the tap snapshot of some runner images and
    # `whisper.cpp` in newer ones, and the image's tap is not updated here (an update
    # would be one more thing that drifts), so ask which name this Homebrew knows.
    whisper_formula = next((
        name for name in WHISPER_FORMULAE
        if run([brew, "info", "--formula", name], env=env, check=False).returncode == 0
    ), None)
    if whisper_formula is None:
        raise Failure(f"Homebrew knows none of the formulae {', '.join(WHISPER_FORMULAE)}")
    run([brew, "install", "ffmpeg", whisper_formula], env=env, timeout=INSTALL_TIMEOUT_SECONDS)
    versions = run([brew, "list", "--versions", "ffmpeg", whisper_formula], env=env).stdout.strip()
    prefix = Path(run([brew, "--prefix"], env=env).stdout.strip())
    found = {}
    for name in ("ffmpeg", "ffprobe", "whisper-cli"):
        located = shutil.which(name, path=str(prefix / "bin"))
        if located is None:
            raise Failure(f"Homebrew installed no {name} under {prefix / 'bin'}")
        found[name] = Path(located).resolve()
    staging = import_pinned_tools(Path(state["tests_root"]))
    pinned = work / "pinned"
    pinned.mkdir(parents=True, exist_ok=False)
    models = staging.stage_models(pinned)
    model = models["base"]
    return {
        "provenance": "homebrew: formulae `ffmpeg` and `whisper.cpp` as Homebrew's bottles "
                      "provide them (not reviewed artifacts: recorded, not endorsed); the "
                      "base model is the repository's pinned, hash-checked file",
        "homebrew": versions,
        "ffmpeg_dir": str(found["ffmpeg"].parent),
        "whisper_cli": str(found["whisper-cli"]),
        "whisper_model": str(model),
        "whisper_model_q5_1": str(models["base_q5_1"]),
        "tools": {
            "ffmpeg": tool_record(found["ffmpeg"], [str(found["ffmpeg"]), "-version"]),
            "ffprobe": tool_record(found["ffprobe"], [str(found["ffprobe"]), "-version"]),
            "whisper_cli": tool_record(found["whisper-cli"], None),
            "whisper_model": tool_record(model, None),
        },
    }


def command_stage_tools(arguments: argparse.Namespace) -> None:
    work = Path(arguments.work)
    state = load_state(work)
    system = state["system"]
    stager = {"ubuntu": stage_ubuntu, "windows": stage_windows, "macos": stage_macos}[system]
    tools = stager(work, state)
    tools["system"] = system
    write_json(work / "out" / "tools.json", tools)
    state["tools"] = {key: tools[key] for key in
                      ("ffmpeg_dir", "whisper_cli", "whisper_model", "whisper_model_q5_1")
                      if key in tools}
    write_json(work / "state.json", state)
    lines = [f"### Tools ({system})\n", f"{tools['provenance']}.\n"]
    if "homebrew" in tools:
        lines.append(f"Homebrew: `{tools['homebrew']}`\n")
    for role, record in tools["tools"].items():
        version = f", `{record['version_line']}`" if record.get("version_line") else ""
        lines.append(f"- `{role}`: {record['bytes']} bytes, SHA-256 `{record['sha256']}`{version}")
    append_summary("\n".join(lines) + "\n")


# ------------------------------------------------------------------------ run ----

def checkpoint_environment(state: dict[str, Any], checkpoint: Checkpoint) -> dict[str, str]:
    """The environment of one checkpoint: the override, the tools, nothing else added."""
    env = dict(os.environ)
    tools = state["tools"]
    env.update({
        "VSIFT_E2E_BINARY": state["binary"],
        "VSIFT_E2E_EXPECTED_VERSION": state["version"],
        "VSIFT_E2E_EXPECTED_COMMIT": state["tag_commit"],
        "VSIFT_TEST_WHISPER_CLI": tools["whisper_cli"],
        "VSIFT_TEST_WHISPER_MODEL": tools["whisper_model"],
        "PATH": tools["ffmpeg_dir"] + os.pathsep + env.get("PATH", ""),
        "CARGO_TERM_COLOR": "never",
        "RUST_BACKTRACE": "0",
    })
    if tools.get("whisper_model_q5_1"):
        env["VSIFT_TEST_WHISPER_MODEL_Q5_1"] = tools["whisper_model_q5_1"]
    env.update(checkpoint.environment)
    return env


def new_reports(root: Path, before: set[str]) -> list[Path]:
    """The report files a checkpoint wrote, oldest first."""
    if not root.is_dir():
        return []
    fresh = [entry for entry in root.iterdir() if entry.name not in before
             and (entry / "report.json").is_file()]
    return [entry / "report.json" for entry in sorted(fresh, key=lambda entry: entry.stat().st_mtime)]


def stage_rows(report: Any) -> list[dict[str, str]]:
    """The name, status and one line of detail of every stage of a report."""
    rows = []
    for stage in report.get("stages", []) if isinstance(report, dict) else []:
        if not isinstance(stage, dict):
            continue
        detail = stage.get("diagnostic") or stage.get("remediation") or ""
        rows.append({"name": str(stage.get("name")), "status": str(stage.get("status")),
                     "detail": str(detail)[:300]})
    return rows


def classify(code: int | None, rows: list[dict[str, str]]) -> str:
    """passed, failed, blocked or timed_out for one checkpoint."""
    if code is None:
        return "timed_out"
    statuses = {row["status"] for row in rows}
    if code == 0:
        return "passed"
    if "failed" in statuses or not statuses:
        return "failed"
    return "blocked" if "blocked" in statuses else "failed"


def command_run(arguments: argparse.Namespace) -> None:
    work = Path(arguments.work)
    state = load_state(work)
    if "binary" not in state or "tools" not in state:
        raise Failure("install and stage-tools must run before the checkpoints")
    tests_root = Path(state["tests_root"])
    selected = [checkpoint for checkpoint in CHECKPOINTS
                if state["system"] in checkpoint.systems
                and checkpoint.key not in arguments.skip
                and (not arguments.only or checkpoint.key in arguments.only)]
    out = work / "out"
    results: dict[str, Any] = {
        "format": RESULTS_FORMAT, "system": state["system"], "version": state["version"],
        "tag_commit": state["tag_commit"], "workflow_commit": state["workflow_commit"],
        "tests_from": state["tests_from"], "version_line": state["version_line"],
        "runner": {"image_os": os.environ.get("ImageOS", ""),
                   "image_version": os.environ.get("ImageVersion", ""),
                   "platform": platform.platform(), "machine": platform.machine(),
                   "cpus": os.cpu_count()},
        "checkpoints": [],
        "not_run": [{"name": name, "reason": reason}
                    for name, reason in NOT_RUN_ELSEWHERE[state["system"]]],
    }
    # A checkpoint left out on request is reported, never dropped silently.
    for checkpoint in CHECKPOINTS:
        if state["system"] in checkpoint.systems and checkpoint.key in arguments.skip:
            results["not_run"].append({
                "name": f"{checkpoint.test} ({checkpoint.key})",
                "reason": "skipped on this run by request: "
                          "a slow gate that weekly and pull-request runs leave to a dispatch "
                          "(input asr_gates)"})
    base_env = checkpoint_environment(state, CHECKPOINTS[0])
    targets: list[str] = []
    for checkpoint in selected:
        if checkpoint.package == "vsift-cli":
            targets += ["--test", checkpoint.test]
    say(f"::group::build the checkpoint tests ({state['tests_from']} source)")
    code, seconds, _ = run_logged(
        ["cargo", "test", "--release", "--locked", "-p", "vsift-cli", "--no-run", *targets],
        out / "logs" / "build.log", cwd=tests_root, env=base_env, timeout=BUILD_TIMEOUT_SECONDS)
    say("::endgroup::")
    results["build"] = {"status": "passed" if code == 0 else "failed", "seconds": round(seconds)}
    results["complete"] = False
    write_json(out / "results.json", results)
    reports_root = tests_root / ".vsift" / "e2e-runs"
    for checkpoint in selected:
        entry: dict[str, Any] = {"key": checkpoint.key, "test": checkpoint.test,
                                 "what": checkpoint.what, "log": f"logs/{checkpoint.key}.log"}
        if code != 0:
            entry.update(status="failed", seconds=0, stages=[],
                         diagnostic="the checkpoint tests did not compile")
            results["checkpoints"].append(entry)
            continue
        before = {item.name for item in reports_root.iterdir()} if reports_root.is_dir() else set()
        say(f"::group::{checkpoint.key}: {checkpoint.what}")
        exit_code, seconds, drained = run_logged(
            ["cargo", "test", "--release", "--locked", "-p", checkpoint.package, "--test",
             checkpoint.test, "--", "--ignored", "--nocapture"],
            out / "logs" / f"{checkpoint.key}.log", cwd=tests_root,
            env=checkpoint_environment(state, checkpoint),
            timeout=(SLOW_CHECKPOINT_TIMEOUT_SECONDS if checkpoint.package != "vsift-cli"
                     else CHECKPOINT_TIMEOUT_SECONDS))
        say("::endgroup::")
        reports = new_reports(reports_root, before)
        rows: list[dict[str, str]] = []
        binaries = []
        for index, report_path in enumerate(reports):
            report = read_json(report_path)
            rows += stage_rows(report)
            if isinstance(report, dict) and report.get("binary_under_test"):
                binaries.append(report["binary_under_test"])
            copy = out / "reports" / f"{checkpoint.key}-{index}.json"
            copy.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(report_path, copy)
        entry.update(status=classify(exit_code, rows), seconds=round(seconds), stages=rows,
                     exit_code=exit_code, binary_under_test=binaries[0] if binaries else None)
        if not drained:
            entry["lingering_output"] = True
        if entry["status"] != "passed" and not rows:
            entry["diagnostic"] = "no report was written; see the log"
        results["checkpoints"].append(entry)
        # Written after every checkpoint: a job that is cancelled still reports what ended.
        write_json(out / "results.json", results)
        say(f"{checkpoint.key}: {entry['status']} in {entry['seconds']} s")
    results["overall"] = ("passed" if results["build"]["status"] == "passed" and all(
        item["status"] == "passed" for item in results["checkpoints"]) else "failed")
    results["complete"] = True
    write_json(out / "results.json", results)
    say(f"checkpoints: {results['overall']}")


# ------------------------------------------------------------------- summarize ----

ICONS = {"passed": "passed", "failed": "**FAILED**", "blocked": "**BLOCKED**",
         "timed_out": "**TIMED OUT**", "not_run": "not run"}


def render_results(results: dict[str, Any]) -> str:
    """The markdown of one system's results, failures first and in full."""
    system = results["system"]
    overall = "PASSED" if results["overall"] == "passed" else "FAILED"
    lines = [
        f"## P14 journeys: {SYSTEM_LABELS[system]}",
        "",
        f"**{overall}** on `{results['version_line']}` (version {results['version']}, tag "
        f"commit `{results['tag_commit'][:12]}`). Tests compiled from "
        f"{'the tag' if results['tests_from'] == 'tag' else 'the workflow ref ' + results['workflow_commit'][:12]}.",
    ]
    runner = results.get("runner")
    if runner:
        lines.append(f"Runner image `{runner.get('image_os')}` {runner.get('image_version')}: "
                     f"{runner.get('platform')}, {runner.get('cpus')} CPUs.")
    lines += [
        "",
        "| Checkpoint | What | Result | Stages passed | Seconds |",
        "| --- | --- | --- | --- | --- |",
    ]
    for item in results["checkpoints"]:
        stages = item.get("stages", [])
        passed = sum(1 for stage in stages if stage["status"] == "passed")
        lines.append(f"| `{item['key']}` | {item['what']} | {ICONS.get(item['status'], item['status'])} "
                     f"| {passed} of {len(stages)} | {item.get('seconds', 0)} |")
    problems = [(item, stage) for item in results["checkpoints"]
                for stage in item.get("stages", []) if stage["status"] != "passed"]
    if problems or any(item["status"] != "passed" for item in results["checkpoints"]):
        lines += ["", "### Stages that did not pass", ""]
        for item, stage in problems:
            lines.append(f"- `{item['key']}` / `{stage['name']}`: {stage['status']}: {stage['detail']}")
        for item in results["checkpoints"]:
            if item["status"] != "passed" and not item.get("stages"):
                lines.append(f"- `{item['key']}`: {item['status']}: {item.get('diagnostic', 'see the log')}")
    lines += ["", "### Not run here, and why", ""]
    for entry in results["not_run"]:
        lines.append(f"- {entry['name']}: {entry['reason']}")
    lines += ["", "In-process stages: P06's tool verification, P08's `p08_candidates_budget` and "
              "P11's session inspection use the engine library compiled from the tests' source, "
              "not the installed binary; every other stage drives the installed binary."]
    return "\n".join(lines) + "\n"


def command_summarize(arguments: argparse.Namespace) -> None:
    work = Path(arguments.work)
    results_path = work / "out" / "results.json"
    if not results_path.is_file():
        # An earlier step failed (the install, the tools, the build): say so where the
        # reader looks, instead of leaving an empty summary.
        append_summary("## P14 journeys\n\n**FAILED before the checkpoints ran** (see the "
                       "failed step above): there are no results for this system.\n")
        raise Failure("no results were written")
    results = read_json(results_path)
    if not results.get("complete"):
        results["overall"] = "failed"
        results.setdefault("checkpoints", [])
        results["checkpoints"].append({
            "key": "(stopped)", "test": "", "what": "the run did not finish: the job was "
            "cancelled or its step timed out after the checkpoints above ended",
            "status": "failed", "seconds": 0, "stages": [],
            "diagnostic": "no result after the last checkpoint listed; see the job log"})
    markdown = render_results(results)
    append_summary(markdown)
    if results["overall"] != "passed":
        raise Failure(f"the journeys on {results['system']} did not pass")


def command_report(arguments: argparse.Namespace) -> None:
    """The one table that says, for every system, whether the journeys passed."""
    found: dict[str, Any] = {}
    for path in sorted(Path(arguments.results).rglob("results.json")):
        document = read_json(path)
        found[document["system"]] = document
    lines = ["# P14 journeys (RQ-05): the published binary", "",
             f"Requested version: `{arguments.version}`. A system with no results below did not "
             "finish: that is a failure, not a skip.", "",
             "| System | Result | Checkpoints passed |", "| --- | --- | --- |"]
    failed = False
    for system in SYSTEMS:
        document = found.get(system)
        if document is None:
            lines.append(f"| {system} | **NO RESULTS** | - |")
            failed = True
            continue
        passed = sum(1 for item in document["checkpoints"] if item["status"] == "passed")
        total = len(document["checkpoints"])
        verdict = "passed" if document["overall"] == "passed" else "**FAILED**"
        failed = failed or document["overall"] != "passed"
        lines.append(f"| {system} | {verdict} | {passed} of {total} |")
    if failed:
        lines += ["", "**This run found something.** Each failure is a finding: reproduce it, "
                  "record the evidence, open an issue (governance rule 14) and do not rerun until "
                  "it is tracked. See the per-system summaries and uploaded logs."]
    append_summary("\n".join(lines) + "\n")
    if failed:
        raise Failure("at least one system did not pass")


# ------------------------------------------------------------------------ main ----

def workspace_default(name: str) -> str:
    """A folder under the job's workspace: the default of each path option."""
    workspace = os.environ.get("GITHUB_WORKSPACE")
    return str(Path(workspace) / name) if workspace else name


def add_work_option(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--work", default=workspace_default("p14-work"),
                        help="the work folder shared by the steps of one job")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)

    resolve = commands.add_parser("resolve-version")
    resolve.add_argument("--requested", default="")
    resolve.set_defaults(handler=command_resolve_version)

    select = commands.add_parser("select-source")
    add_work_option(select)
    select.add_argument("--version", required=True)
    select.add_argument("--tag-source", default=workspace_default("tag-source"))
    select.add_argument("--workflow-source", default=workspace_default("workflow-source"))
    select.add_argument("--tests-from", choices=("auto", "tag", "workflow-ref"), default="auto")
    select.set_defaults(handler=command_select_source)

    install = commands.add_parser("install")
    add_work_option(install)
    install.add_argument("--export-environment", action="store_true",
                         help="write the override variables to GITHUB_ENV for later steps")
    install.set_defaults(handler=command_install)

    stage = commands.add_parser("stage-tools")
    add_work_option(stage)
    stage.set_defaults(handler=command_stage_tools)

    checkpoints = commands.add_parser("run")
    add_work_option(checkpoints)
    checkpoints.add_argument("--only", action="append", default=[],
                             choices=[checkpoint.key for checkpoint in CHECKPOINTS])
    checkpoints.add_argument("--skip", action="append", default=[],
                             choices=[checkpoint.key for checkpoint in CHECKPOINTS],
                             help="leave a checkpoint out, and say so in the results")
    checkpoints.set_defaults(handler=command_run)

    summarize = commands.add_parser("summarize")
    add_work_option(summarize)
    summarize.set_defaults(handler=command_summarize)

    report = commands.add_parser("report")
    report.add_argument("--results", required=True)
    report.add_argument("--version", default="")
    report.set_defaults(handler=command_report)
    return parser


def main(argv: list[str]) -> int:
    arguments = build_parser().parse_args(argv)
    try:
        arguments.handler(arguments)
    except Failure as failure:
        print(f"P14 journeys failed: {failure}", file=sys.stderr)
        return 1
    except subprocess.TimeoutExpired as expired:
        print(f"P14 journeys failed: `{expired.cmd[0]}` timed out", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
