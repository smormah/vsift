"""Check that what a release branch changed since its cut is also on `main`.

A release branch (`release/<MAJOR>.<MINOR>.x`, docs/operations/release.md 6.8 and 6.18)
takes window fixes by pull request, and each fix is brought to `main` by a second pull
request with the same change. Nothing else compares the two, so a fix could ship in one
release and be missing from the next cut (L-173). This script is that comparison. It
reports, for every commit on a release branch since the cut, whether its change is on
`main`, and exits 1 if any is not.

**How "its change is on main" is decided, and why not otherwise.** Both branches take
squash merges, so a commit of the release branch is never an ancestor of `main`, and the
counterpart on `main` is a different commit from a different pull request (its own number,
its own date, often its own context). Three ways to match them were weighed:

- *By ancestry* (`git branch --contains`, `git cherry` by commit id): never true with squash
  merges, so it would report every fix as missing.
- *By pull request number* (the `(#123)` of a squash subject): the counterpart is a
  different pull request, so the numbers never match; it would need a naming convention
  that nothing enforces.
- *By patch-id* (`git patch-id`, `git cherry`): matches only when the diff is the same
  text, context included. The counterpart on `main` is made against a `main` that has moved
  on, so its context differs, and one squash can combine two fixes or split one; a patch-id
  match would be wrong in both directions.

What the maintainer wants to know is whether `main` has the fix, so the comparison is made on
content: for each file a release commit changed, every significant line it added must be in
the file on `main` now, every significant line it removed must be gone from `main`'s file
(unless the release branch's own head still has it), and a file it deleted must be gone and a
binary file it changed must be byte-identical. That survives `main` having moved on, a
counterpart made of several commits, and a counterpart that adapted the fix. It errs toward
over-reporting: a fix that `main` later reworked is reported as missing, and the pull request
that finds it records why it is fine. It does not prove the counterpart is *right*, only that
the lines are there.

**Version strings differ on purpose** (the release branch carries its candidates' versions,
`main` the next cut's), so in the version-string files the semantic version inside a
version key or a `vsift` line is replaced by `<V>` before lines are compared; every other line
of those files, including a third-party dependency's `checksum` or `version` in a manifest,
is compared as written.

**The work record** (`CHANGELOG.md`, `memory/`, `docs/decisions|history|planning|guide/`) and the
two shipped documents reach `main` in the records change after the publish, not as window fixes.
They are reported separately and fail the check only with `--records`, which the records change
itself passes (6.18 step 5).

**What is excused, and why none of it can hide a missing code fix (L-173, #458).** The records change
does not copy the release branch's work record: it rewrites it as current state. A line comparison
would then report every rewritten line, so three excuses exist, and each is narrower than the
file it applies to:

1. *A superseded line* (any file). A line a window commit added that the release branch's own
   head no longer has was replaced by a later window commit. That later commit is compared on its
   own: its added lines must be on `main` and the line it removed must be gone. So the net change of
   the branch is still held to `main`; only the intermediate text is not.
2. *A current-state file* (`CURRENT_STATE_FILES`: `memory/TODO.md` and
   `memory/project_current_status.md`). They state where the work is and are rewritten in full by
   every change, so a line of them is not a fact that must survive. They are satisfied when the last
   change to the file on `main` is not older than the release commit (a rewrite newer than the
   branch's); if `main` has not rewritten them since, they are compared line by line as before. They
   hold no code, configuration or shipped text, and a window fix has its own durable record in the
   changelog and the register, which are still compared.
3. *A line rewritten in place on both sides* (`CHANGELOG.md`, the `.md` pages directly in
   `docs/planning/`, the register among them, and the `.md` pages of the trial records,
   `docs/planning/p14-agent-trials/` and its subfolders outside their `records/` folders; #481, run
   39237738669). Either the release commit replaced a line of the cut (a hunk with removed and added
   lines) and `main` no longer has the line it replaced, or `main` has a line that is not in the cut and
   begins as the added line does (the same sentence, reworded), or contains all the added line's words in
   order (the same sentence with words added, such as a link to a file `main` moved). A new line with
   no such counterpart still fails, and so does a replaced line that `main` kept as the cut had it:
   `main` has then not taken the change. So the register's date and counts lines, which both sides
   rewrite, are not reported, while a new entry (its heading and its table row are new lines) that
   `main` lacks is.

The trial pages are in excuse 3 because they are prose work records that name the trial freeze a
candidate was run on: a later freeze makes `main` rewrite those lines in its own words (#468 rewrote the
readings and the README for the new freeze, while the release branch still names the old one). The
`.md` suffix is what keeps the trial's `freeze.json` and `state` and `summary` files, and everything in a
`records/` folder, strict: they are data, and a changed digest there is a different fact, not a reworded one.

Code, configuration, JSON (the ledger, the trials' `freeze.json` and `records/`), version files, shipped
documents (`docs/operations/install.md`, `npm/vsift-cli/README.md`) and every other file are compared
exactly as before: excuse 1 is the only one that reaches them, and it cannot excuse a line the branch
still has. Every excuse is
named in the report, so a reader sees what was not compared.

The script runs `git` with explicit arguments (no shell), reads local objects only, writes
nothing and contacts no network. File contents reach the output only as indented, truncated
quotations, so a line of a file cannot form a workflow command.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
from typing import Callable, Sequence

# The files in which the release branch's version differs from `main`'s by design (the
# lists of docs/operations/release.md 6.8, "version-string files").
VERSION_FILES = frozenset(
    {
        "Cargo.toml",
        "Cargo.lock",
        "fuzz/Cargo.toml",
        "fuzz/Cargo.lock",
        "npm/vsift-cli/package.json",
    }
)

# The work record and the shipped documents: they reach `main` in the records change.
RECORD_PREFIXES = (
    "memory/",
    "docs/decisions/",
    "docs/history/",
    "docs/planning/",
    "docs/guide/",
)
RECORD_FILES = frozenset(
    {
        "CHANGELOG.md",
        "docs/operations/install.md",
        "npm/vsift-cli/README.md",
    }
)

# The files that every change rewrites as current state (the repository's own rule for the two
# memory files). They are satisfied by a newer rewrite on `main` (the module docstring, excuse 2);
# the reason is printed with the excuse.
CURRENT_STATE_FILES = {
    "memory/TODO.md": "the handoff, rewritten as current state by every change",
    "memory/project_current_status.md": "the status, rewritten as current state by every change",
}
# The prose pages of the work record in which a line can be rewritten in place on both sides
# (excuse 3): the changelog, the pages directly in `docs/planning/` (the register among them), and the
# Markdown pages of the trial records, which name the trial freeze that a later freeze changes.
REWRITTEN_RECORD_FILES = frozenset({"CHANGELOG.md"})
REWRITTEN_RECORD_FOLDER = "docs/planning/"
REWRITTEN_TRIAL_FOLDER = "docs/planning/p14-agent-trials/"
# A folder of raw run records inside the trial folder: data, compared exactly whatever its suffix.
TRIAL_DATA_FOLDER = "records"
# How much of a line's beginning says that it is the same sentence reworded.
LEAD_LENGTH = 60
# A line with fewer words than this is too short for "all its words, in order" to say it is the same sentence.
MINIMUM_WORDS_FOR_EXTENSION = 6
MARKDOWN_LINK_TARGET = re.compile(r"\]\([^)]*\)")
# A line shorter than this (a closing brace, a bare `end`) says nothing about a change.
MINIMUM_SIGNIFICANT_LENGTH = 3
# How much of a line is quoted in the output.
QUOTE_LENGTH = 100
# How many lines of one file are quoted.
QUOTES_PER_FILE = 3

SEMVER = re.compile(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?(?:\+[0-9A-Za-z.]+)?")
VERSION_KEY_LINE = re.compile(r'^\s*"?version"?\s*[=:]\s*"')

EXIT_OK = 0
EXIT_MISSING = 1
EXIT_ERROR = 2


class GitError(Exception):
    """A git command failed or the repository is not shaped as this script expects."""


def run_git(repository: Path, *arguments: str) -> bytes:
    """Run `git -C <repository> <arguments>` and return its output, or raise `GitError`."""
    try:
        completed = subprocess.run(
            ["git", "-C", str(repository), "-c", "core.quotepath=off", *arguments],
            check=False,
            capture_output=True,
        )
    except OSError as error:
        raise GitError(f"git could not be started: {error}") from error
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", "replace").strip().splitlines()
        raise GitError(f"git {arguments[0]} failed: {detail[0] if detail else 'no message'}")
    return completed.stdout


def text(output: bytes) -> str:
    return output.decode("utf-8", "replace")


def kind_of(path: str) -> str:
    """`record` for the work record and shipped documents, `fix` for everything else."""
    posix = PurePosixPath(path).as_posix()
    if posix in RECORD_FILES or posix.startswith(RECORD_PREFIXES):
        return "record"
    return "fix"


def normalise(path: str, line: str) -> str:
    """The line as compared: stripped, and in a version-string file without the version.

    Only a version key's line, or a line naming `vsift`, loses its version: a third-party
    dependency's version in a manifest stays as written.
    """
    stripped = line.strip()
    if path in VERSION_FILES and (VERSION_KEY_LINE.match(stripped) or "vsift" in stripped):
        return SEMVER.sub("<V>", stripped)
    return stripped


def is_rewritten_record(path: str) -> bool:
    """Whether a line of this page can be excused by `main` having rewritten it in place."""
    posix = PurePosixPath(path).as_posix()
    if posix in REWRITTEN_RECORD_FILES:
        return True
    if not posix.endswith(".md"):
        return False
    if posix.startswith(REWRITTEN_TRIAL_FOLDER):
        folders = PurePosixPath(posix[len(REWRITTEN_TRIAL_FOLDER) :]).parts[:-1]
        return TRIAL_DATA_FOLDER not in folders
    return posix.startswith(REWRITTEN_RECORD_FOLDER) and "/" not in posix[len(REWRITTEN_RECORD_FOLDER) :]


def lead(line: str) -> str:
    """The opening of a normalised line without list or table markers: its identity as a sentence."""
    return line.lstrip("-*#| ").strip()[:LEAD_LENGTH]


def words(line: str) -> list[str]:
    """The words of a line without Markdown link targets, emphasis, code marks and edge punctuation."""
    plain = MARKDOWN_LINK_TARGET.sub("", line)
    for mark in "[]`*_":
        plain = plain.replace(mark, "")
    tokens = (token.strip(",.;:()\"'|") for token in plain.split())
    return [token for token in tokens if token]


def is_extension_of(line: str, candidates: Sequence[list[str]]) -> bool:
    """Whether a line of `main` carries all the words of `line`, in order, with words added.

    This is the same sentence with an insertion (a link to a file that `main` moved, a date), which
    the 60-character lead cannot see when the insertion comes early. A line too short to be a
    sentence never matches.
    """
    wanted = words(line)
    if len(wanted) < MINIMUM_WORDS_FOR_EXTENSION:
        return False
    wanted_set = set(wanted)
    for candidate in candidates:
        if len(candidate) < len(wanted) or not wanted_set.issubset(candidate):
            continue
        remaining = iter(candidate)
        if all(word in remaining for word in wanted):
            return True
    return False


def significant(line: str) -> bool:
    return len(line) >= MINIMUM_SIGNIFICANT_LENGTH


def quote(line: str) -> str:
    cut = line if len(line) <= QUOTE_LENGTH else line[: QUOTE_LENGTH - 3] + "..."
    return "    > " + cut


@dataclass(frozen=True)
class Hunk:
    """The lines one hunk of a no-context diff added and removed."""

    added: tuple[str, ...]
    removed: tuple[str, ...]


@dataclass(frozen=True)
class Change:
    """One file of one release commit: its status, and the lines it added and removed."""

    path: str
    status: str
    binary: bool
    added: tuple[str, ...]
    removed: tuple[str, ...]
    hunks: tuple[Hunk, ...] = ()


@dataclass
class Finding:
    """What is missing from `main` for one file of one commit."""

    path: str
    reason: str
    kind: str
    quotes: list[str] = field(default_factory=list)


@dataclass(frozen=True)
class Comparison:
    """The result for one file: what is missing, how many lines were compared, what was excused."""

    finding: Finding | None
    compared: int
    notes: tuple[str, ...] = ()


@dataclass
class CommitReport:
    commit: str
    subject: str
    kind: str
    findings: list[Finding]
    compared: int
    notes: list[str] = field(default_factory=list)

    def failing(self, include_records: bool) -> list[Finding]:
        """The findings that fail the check: a record file counts only with `--records`."""
        return [
            finding
            for finding in self.findings
            if include_records or finding.kind == "fix"
        ]


@dataclass
class BranchReport:
    branch: str
    cut: str
    commits: list[CommitReport]

    def missing(self, include_records: bool) -> list[CommitReport]:
        return [commit for commit in self.commits if commit.failing(include_records)]


def parse_hunk_groups(diff: str) -> list[Hunk]:
    """The hunks of a one-file diff made with no context lines, each with its own lines.

    Everything before the first hunk header is file metadata, so a content line that
    happens to start with `+++` or `---` is not mistaken for a header.
    """
    hunks: list[Hunk] = []
    added: list[str] = []
    removed: list[str] = []
    in_hunk = False
    for line in diff.splitlines():
        if line.startswith("@@"):
            if in_hunk:
                hunks.append(Hunk(tuple(added), tuple(removed)))
            added, removed = [], []
            in_hunk = True
        elif in_hunk and line.startswith("+"):
            added.append(line[1:])
        elif in_hunk and line.startswith("-"):
            removed.append(line[1:])
    if in_hunk:
        hunks.append(Hunk(tuple(added), tuple(removed)))
    return hunks


def parse_hunks(diff: str) -> tuple[list[str], list[str]]:
    """All the added and removed lines of a one-file diff made with no context lines."""
    hunks = parse_hunk_groups(diff)
    return (
        [line for hunk in hunks for line in hunk.added],
        [line for hunk in hunks for line in hunk.removed],
    )


def commit_changes(repository: Path, commit: str) -> list[Change]:
    """The files `commit` changed against its first parent."""
    raw = run_git(
        repository,
        "diff-tree",
        "--no-commit-id",
        "-r",
        "-z",
        "--raw",
        "--no-renames",
        commit,
    )
    fields = raw.split(b"\0")
    changes: list[Change] = []
    index = 0
    while index + 1 < len(fields) and fields[index]:
        status = fields[index].decode("ascii", "replace").split()[-1][0]
        path = fields[index + 1].decode("utf-8", "replace")
        index += 2
        diff = text(
            run_git(
                repository,
                "diff",
                "-U0",
                "--no-renames",
                "--no-ext-diff",
                "--no-color",
                f"{commit}^",
                commit,
                "--",
                path,
            )
        )
        binary = "Binary files" in diff or "GIT binary patch" in diff
        hunks = parse_hunk_groups(diff)
        added, removed = parse_hunks(diff)
        changes.append(Change(path, status, binary, tuple(added), tuple(removed), tuple(hunks)))
    return changes


# One run's reads of a file at a ref, so that a file many commits touched is read once.
FileCache = dict[tuple[str, str], list[str] | None]


def file_lines(
    repository: Path, ref: str, path: str, cache: FileCache
) -> list[str] | None:
    """The file's lines at `ref`, or None if the path does not exist there."""
    key = (ref, path)
    if key not in cache:
        try:
            cache[key] = text(
                run_git(repository, "cat-file", "blob", f"{ref}:{path}")
            ).splitlines()
        except GitError:
            cache[key] = None
    return cache[key]


def blob_id(repository: Path, ref: str, path: str) -> str | None:
    try:
        return text(run_git(repository, "rev-parse", f"{ref}:{path}")).strip()
    except GitError:
        return None


def last_change_time(repository: Path, ref: str, path: str) -> int | None:
    """When the last commit that changed `path` on `ref` was made (committer time), or None."""
    try:
        listed = text(
            run_git(repository, "log", "-1", "--format=%ct", ref, "--", path)
        ).strip()
    except GitError:
        return None
    return int(listed) if listed.isdigit() else None


def commit_time(repository: Path, commit: str) -> int:
    """The committer time of `commit`."""
    return int(text(run_git(repository, "show", "-s", "--format=%ct", commit)).strip())


def day(seconds: int) -> str:
    return datetime.fromtimestamp(seconds, timezone.utc).strftime("%Y-%m-%d")


def rewritten_since(
    repository: Path, change: Change, main: str, commit: str
) -> Comparison | None:
    """Excuse 2: a current-state file that `main` rewrote after `commit` was made.

    None when the file is not a current-state file, or `main` has not rewritten it since (it is
    then compared line by line like any other file).
    """
    reason = CURRENT_STATE_FILES.get(PurePosixPath(change.path).as_posix())
    if reason is None:
        return None
    on_main = last_change_time(repository, main, change.path)
    if on_main is None or on_main < commit_time(repository, commit):
        return None
    note = (
        f"{change.path}: {reason}; main rewrote it on {day(on_main)}, after this commit, "
        "so its lines are not compared"
    )
    return Comparison(None, 0, (note,))


def compare_change(
    repository: Path,
    change: Change,
    main: str,
    head: str,
    cut: str,
    commit: str,
    cache: FileCache,
) -> Comparison:
    """Whether `main` has this file's change, how many lines were compared, what was excused."""
    on_main = file_lines(repository, main, change.path, cache)
    kind = kind_of(change.path)
    if change.status == "D":
        if on_main is None:
            return Comparison(None, 0)
        return Comparison(
            Finding(change.path, "deleted on the release branch, still on main", kind), 0
        )
    if on_main is None:
        return Comparison(
            Finding(change.path, "changed on the release branch, not a file on main", kind), 0
        )
    if change.binary:
        if blob_id(repository, main, change.path) == blob_id(repository, head, change.path):
            return Comparison(None, 0)
        return Comparison(
            Finding(change.path, "binary file differs from the release branch's", kind), 0
        )

    current_state = rewritten_since(repository, change, main, commit)
    if current_state is not None:
        return current_state

    on_head = file_lines(repository, head, change.path, cache) or []
    main_lines = {normalise(change.path, line) for line in on_main}
    head_lines = {normalise(change.path, line) for line in on_head}
    # Excuse 3 needs the cut's file: a line of `main` that the cut does not have is `main`'s own.
    rewritable = is_rewritten_record(change.path)
    main_new_leads: set[str] = set()
    main_new_words: list[list[str]] = []
    if rewritable:
        on_cut = file_lines(repository, cut, change.path, cache) or []
        cut_lines = {normalise(change.path, line) for line in on_cut}
        main_new_leads = {lead(line) for line in main_lines - cut_lines if lead(line)}
        main_new_words = [words(line) for line in main_lines - cut_lines]

    absent: list[str] = []
    superseded = 0
    rewritten = 0
    for hunk in change.hunks or (Hunk(change.added, change.removed),):
        replaced = [
            normalise(change.path, line)
            for line in hunk.removed
            if significant(normalise(change.path, line))
        ]
        # The hunk replaced lines of the cut, and `main` has none of them any more.
        replaced_on_main = bool(replaced) and all(line not in main_lines for line in replaced)
        for line in hunk.added:
            compared = normalise(change.path, line)
            if not significant(compared) or compared in main_lines or compared in absent:
                continue
            if compared not in head_lines:
                superseded += 1
            elif rewritable and (
                replaced_on_main
                or lead(compared) in main_new_leads
                or is_extension_of(compared, main_new_words)
            ):
                rewritten += 1
            else:
                absent.append(compared)
    remaining: list[str] = []
    for line in change.removed:
        compared = normalise(change.path, line)
        if (
            significant(compared)
            and compared in main_lines
            and compared not in head_lines
            and compared not in remaining
        ):
            remaining.append(compared)

    compared_count = sum(
        1 for line in (*change.added, *change.removed) if significant(normalise(change.path, line))
    )
    excused: list[str] = []
    if superseded:
        excused.append(f"{superseded} added line(s) replaced by a later window commit")
    if rewritten:
        excused.append(f"{rewritten} added line(s) rewritten on main")
    notes = (f"{change.path}: {', '.join(excused)}",) if excused else ()
    if not absent and not remaining:
        return Comparison(None, compared_count, notes)
    reasons = []
    if absent:
        reasons.append(f"{len(absent)} added line(s) not on main")
    if remaining:
        reasons.append(f"{len(remaining)} removed line(s) still on main")
    finding = Finding(change.path, ", ".join(reasons), kind)
    finding.quotes = [quote("+ " + line) for line in absent[:QUOTES_PER_FILE]]
    finding.quotes += [quote("- " + line) for line in remaining[:QUOTES_PER_FILE]]
    return Comparison(finding, compared_count, notes)


def check_release_branch(repository: Path, main: str, release: str) -> BranchReport:
    """Compare every commit of `release` since its cut from `main` with `main`."""
    try:
        cut = text(run_git(repository, "merge-base", main, release)).strip()
    except GitError as error:
        raise GitError(f"{release} and {main} share no history: {error}") from error
    listed = text(
        run_git(repository, "rev-list", "--reverse", "--no-merges", f"{cut}..{release}")
    ).split()
    head = text(run_git(repository, "rev-parse", release)).strip()
    reports: list[CommitReport] = []
    cache: FileCache = {}
    for commit in listed:
        subject = text(run_git(repository, "show", "-s", "--format=%s", commit)).strip()
        findings: list[Finding] = []
        notes: list[str] = []
        compared = 0
        kinds = set()
        for change in commit_changes(repository, commit):
            kinds.add(kind_of(change.path))
            comparison = compare_change(repository, change, main, head, cut, commit, cache)
            compared += comparison.compared
            notes.extend(comparison.notes)
            if comparison.finding is not None:
                findings.append(comparison.finding)
        # A commit that changes any code or configuration is a fix, whatever else it changes.
        kind = "fix" if "fix" in kinds else "record"
        reports.append(CommitReport(commit, subject, kind, findings, compared, notes))
    return BranchReport(release, cut, reports)


def release_branches(repository: Path, pattern: str) -> list[str]:
    """The refs matching `pattern` (a `for-each-ref` pattern), by name."""
    listed = text(
        run_git(repository, "for-each-ref", "--format=%(refname)", pattern)
    ).split()
    return sorted(listed)


def render(report: BranchReport, include_records: bool) -> tuple[str, bool]:
    """The report for one branch as text, and whether the branch passes."""
    lines = [
        f"{report.branch}: {len(report.commits)} commit(s) since the cut "
        f"{report.cut[:12]}"
    ]
    failed = False
    for commit in report.commits:
        if commit.failing(include_records):
            state = "NOT ON MAIN"
            failed = True
        elif commit.findings:
            state = "fix on main; work record still to be brought by the records change"
        elif commit.compared == 0:
            state = "on main (nothing significant to compare)"
        else:
            state = "on main"
        lines.append(f"  {commit.commit[:12]} [{commit.kind}] {state}: {commit.subject}")
        lines.extend(f"      excused, {note}" for note in commit.notes)
        for finding in commit.findings:
            lines.append(f"      {finding.path} ({finding.kind}): {finding.reason}")
            lines.extend("  " + quote_line for quote_line in finding.quotes)
    return "\n".join(lines), not failed


def run(
    repository: Path,
    main: str,
    releases: Sequence[str],
    include_records: bool,
    write: Callable[[str], None],
) -> int:
    """Check each release ref; print the reports; return the exit code."""
    if not releases:
        write("No release branch to check (no ref matches release/*): nothing to compare.")
        return EXIT_OK
    exit_code = EXIT_OK
    for release in releases:
        report = check_release_branch(repository, main, release)
        rendered, passed = render(report, include_records)
        write(rendered)
        if not passed:
            exit_code = EXIT_MISSING
    if exit_code == EXIT_MISSING:
        write(
            "\nA window fix is missing from main. Open its pull request against main "
            "(docs/operations/release.md 6.18 step 4). If main deliberately differs "
            "(it reworked the fix, or the change was only needed on the branch), say so in "
            "the pull request that finds this; the check is a report, not a ruleset."
        )
    else:
        write("\nEvery window fix on the release branch is on main.")
    return exit_code


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--repository", type=Path, default=Path.cwd())
    parser.add_argument(
        "--main",
        default="origin/main",
        help="the ref that must carry the fixes (the merge ref of a pull request to main, "
        "so that the pull request itself counts)",
    )
    parser.add_argument(
        "--release",
        action="append",
        default=[],
        help="a release branch ref; repeatable. Default: every refs/remotes/origin/release/*",
    )
    parser.add_argument(
        "--records",
        action="store_true",
        help="also require the work record and the shipped documents to be on main "
        "(the records change after a publish)",
    )
    arguments = parser.parse_args(argv)
    # File contents are quoted in the report; the console's own code page must not fail on them.
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    try:
        releases = arguments.release or release_branches(
            arguments.repository, "refs/remotes/origin/release/*"
        )
        return run(arguments.repository, arguments.main, releases, arguments.records, print)
    except GitError as error:
        print(f"release_window: {error}", file=sys.stderr)
        return EXIT_ERROR


if __name__ == "__main__":
    sys.exit(main())
