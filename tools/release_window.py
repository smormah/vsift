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

A version bump on `main` (the next candidate's pull request, #516, #518) moves and rewrites the previous
cut's work record, so four more excuses exist. They are what the bump does by design, and each is narrower
than the file it applies to:

4. *A moved file* (`find_moves`). A work-record file of the branch's head that `main` no longer holds at that
   path, but holds under another path with the same blob id (the bump moves the previous candidate's trial
   records into a folder named for it, content unchanged), counts as present, and the report says where it
   moved to. Only work-record files (`kind_of` is `record`) are matched, only to a path the branch's head does
   not have, and an empty file never matches. A moved file whose content changed has another blob id and is
   still reported.
5. *A line naming a moved path*. A branch line that is not on `main` counts as present when `main` has the same
   line with the old path replaced by the new one. Only the paths excuse 4 found are replaced (the whole path,
   or its tail from a folder name on, as a relative link writes it), so a line that differs in anything else
   is still reported.
6. *A version line of a shipped document* (`VERSION_DOCUMENTS`: the guide's `index.md`, `docs/operations/install.md`
   and `npm/vsift-cli/README.md`). A branch line that names a version (`X.Y.Z` or `X.Y.Z-rc.N`) counts as present
   when `main` has it with only the version strings changed or, as the bump words it, when `main` rewrote it in
   place (the same test as excuse 3). It applies only while `main`'s workspace version is newer than the branch
   head's: that is what says the line was rewritten for a later cut and not lost.
7. *A waiver kept as a record in the evidence ledger* (`ledger_kept_as_records`). The ledger is JSON and the
   release evidence, so it is not excused by a line test. The bump sets a waived item back to planned and
   keeps what the waiver said as entries of the item's `prior`. A text of the branch's item (its decision's
   statement and its reason, or an evidence note) counts as present only when `main`'s same item has a `prior`
   entry with the same reference (a moved path as `main` has it) and the same date, whose note begins with
   exactly `NOT COUNTED for <main's workspace version>, kept as a record: ` and contains that text unchanged
   (for the statement and the reason, both). The version must be `main`'s workspace version and newer than the
   branch's. Each item so excused is named in the report. Because the text must be there word for word, under
   that prefix and with the reference and date it had, a reworded decision, a changed date, a missing entry or
   the prefix of another candidate is reported as before, and a weakened waiver cannot pass as a kept one.

None of the four reaches code. Excuse 4 needs a work-record file, excuse 5 only rewrites paths of files that
excuse 4 found, excuse 6 is limited to three prose documents, and excuse 7 reads one JSON file: a version-looking change in a source file,
a workflow, a manifest or any page that is not one of the three is compared exactly as before.

Code, configuration, JSON (the ledger, the trials' `freeze.json` and `records/`), version files, shipped
documents (`docs/operations/install.md`, `npm/vsift-cli/README.md`) and every other file are compared
exactly as before, except by the excuses above: excuse 1 reaches them all and cannot excuse a line the branch
still has; excuses 4 to 7 reach the work-record files, the three documents and the ledger only. Every excuse is
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
import json
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
# The shipped documents whose version lines a bump rewrites for the next cut (excuse 6; the pages of
# docs/operations/release.md 6.8 that carry the version in prose). Nothing else gets that excuse.
VERSION_DOCUMENTS = frozenset(
    {
        "docs/guide/index.md",
        "docs/operations/install.md",
        "npm/vsift-cli/README.md",
    }
)
# The evidence ledger, whose waived or superseded entries a bump keeps as records in `prior` (excuse 7).
LEDGER_PATH = "docs/planning/p14-evidence-ledger.json"
# Git's id of the empty blob: every empty file has it, so it says nothing about a move.
EMPTY_BLOB = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
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

# A version ends at its last identifier, so the full stop that ends a sentence is not part of it.
SEMVER = re.compile(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?(?:\+[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?")
# The workspace version is the first `version = "..."` line of the root manifest.
MANIFEST_VERSION = re.compile(r'^version\s*=\s*"([^"]+)"', re.MULTILINE)
VERSION_PARTS = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.]+))?")
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
    moved: tuple[tuple[str, str], ...] = ()


@dataclass(frozen=True)
class BranchContext:
    """What is true of the whole branch, read once: the files `main` moved, and whether its version is newer."""

    moves: dict[str, str] = field(default_factory=dict)
    # Each spelling of a moved path (whole, or its tail from a folder name on) and where it moved to.
    spellings: dict[str, str] = field(default_factory=dict)
    pattern: re.Pattern[str] | None = None
    main_version: str | None = None
    head_version: str | None = None
    main_is_newer: bool = False

    def substitute(self, line: str) -> str:
        """The line with every moved path replaced by its new place (unchanged if it names none)."""
        if self.pattern is None:
            return line
        return self.pattern.sub(lambda found: self.spellings[found.group(0)], line)


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


def version_order(version: str) -> tuple[int, int, int, int, tuple[tuple[int, int, str], ...]] | None:
    """A sortable key for `X.Y.Z` or `X.Y.Z-rc.N` (a release sorts after its candidates), or None."""
    parts = VERSION_PARTS.match(version)
    if parts is None:
        return None
    major, minor, patch, pre = parts.groups()
    identifiers = tuple(
        (0, int(item), "") if item.isdigit() else (1, 0, item) for item in (pre or "").split(".") if item
    )
    return int(major), int(minor), int(patch), 0 if pre else 1, identifiers


def workspace_version(repository: Path, ref: str) -> str | None:
    """The version in the root `Cargo.toml` at `ref`, or None if there is none to read."""
    try:
        manifest = text(run_git(repository, "cat-file", "blob", f"{ref}:Cargo.toml"))
    except GitError:
        return None
    found = MANIFEST_VERSION.search(manifest)
    return found.group(1) if found else None


def tree_blobs(repository: Path, ref: str) -> dict[str, str]:
    """Every file of the tree at `ref` with its blob id."""
    listed = run_git(repository, "ls-tree", "-r", "-z", ref).split(b"\0")
    blobs: dict[str, str] = {}
    for entry in listed:
        meta, separator, path = entry.partition(b"\t")
        fields = meta.split()
        if separator and len(fields) == 3 and fields[1] == b"blob":
            blobs[path.decode("utf-8", "replace")] = fields[2].decode("ascii", "replace")
    return blobs


def find_moves(repository: Path, main: str, head: str) -> dict[str, str]:
    """The work-record files of `head` that `main` holds, with the same content, under another path.

    Excuse 4. A file counts as moved only if `main` has no file at its path, the branch's head has no
    file at the new path (otherwise it would be a copy), both paths are work record, and the file is not
    empty. When several files of `main` have the content, the one with the same file name is taken.
    """
    on_main = tree_blobs(repository, main)
    on_head = tree_blobs(repository, head)
    by_blob: dict[str, list[str]] = {}
    for path, blob in on_main.items():
        if path not in on_head and kind_of(path) == "record":
            by_blob.setdefault(blob, []).append(path)
    moves: dict[str, str] = {}
    for path, blob in on_head.items():
        if path in on_main or blob == EMPTY_BLOB or kind_of(path) != "record":
            continue
        places = sorted(by_blob.get(blob, []))
        if places:
            same_name = [place for place in places if PurePosixPath(place).name == PurePosixPath(path).name]
            moves[path] = (same_name or places)[0]
    return moves


def spellings_of(moves: dict[str, str]) -> dict[str, str]:
    """Each way a document can write a moved path (whole, or from a shared folder on, as a relative link does).

    `a/b/c.md` moved to `a/b2/c.md` is also written `b/c.md` in a link relative to `a/`. Every spelling
    starts at or before the first folder that differs, so the bare file name (which does not say which
    file is meant) and a tail that skips the moved folder are never replaced.
    """
    spellings: dict[str, str] = {}
    for old, new in moves.items():
        old_parts = PurePosixPath(old).parts
        new_parts = PurePosixPath(new).parts
        shared = 0
        while shared < min(len(old_parts), len(new_parts)) - 1 and old_parts[shared] == new_parts[shared]:
            shared += 1
        for start in range(shared + 1):
            spellings["/".join(old_parts[start:])] = "/".join(new_parts[start:])
    return spellings


def make_context(repository: Path, main: str, head: str) -> BranchContext:
    """Read once what the excuses 4 to 7 need to know about the whole branch."""
    moves = find_moves(repository, main, head)
    spellings = spellings_of(moves)
    pattern = None
    if spellings:
        alternatives = "|".join(re.escape(spelling) for spelling in sorted(spellings, key=len, reverse=True))
        # A path is not part of a longer name: `xa/b.md` is not `a/b.md`.
        pattern = re.compile(rf"(?<![\w-])(?:{alternatives})")
    main_version = workspace_version(repository, main)
    head_version = workspace_version(repository, head)
    main_key = version_order(main_version) if main_version else None
    head_key = version_order(head_version) if head_version else None
    newer = main_key is not None and head_key is not None and main_key > head_key
    return BranchContext(moves, spellings, pattern, main_version, head_version, newer)


def describe_moves(moved: Sequence[tuple[str, str]]) -> list[str]:
    """Notes for the moved files of one commit: one per pair of folders, so 100 records are not 100 lines."""
    groups: dict[tuple[str, str], list[str]] = {}
    for old, new in moved:
        key = (PurePosixPath(old).parent.as_posix(), PurePosixPath(new).parent.as_posix())
        groups.setdefault(key, []).append(PurePosixPath(old).name)
    notes = []
    for (old_folder, new_folder), names in sorted(groups.items()):
        if len(names) == 1:
            notes.append(f"{old_folder}/{names[0]}: moved to {new_folder}/{names[0]}, same content")
        else:
            notes.append(
                f"{len(names)} files of {old_folder}/ moved to {new_folder}/, same content "
                f"(for example {sorted(names)[0]})"
            )
    return notes


def record_reference(reference: object, moves: dict[str, str]) -> object:
    """A ledger reference with the path of a file that `main` moved given as `main` has it."""
    if isinstance(reference, dict) and reference.get("type") == "record":
        path = reference.get("path")
        if isinstance(path, str) and path in moves:
            return {**reference, "path": moves[path]}
    return reference


def ledger_kept_as_records(
    head_lines: Sequence[str], main_lines: Sequence[str], candidate: str, moves: dict[str, str]
) -> dict[str, str]:
    """Excuse 7: the texts of the ledger that `main` kept, unchanged, as records in an item's `prior`.

    A bump sets a waived item back to planned and keeps what its status said as entries of `prior`,
    each with the same reference and date and a note that begins `NOT COUNTED for <candidate>, kept as
    a record: `. A text of the branch's item counts only if `main`'s same item has such an entry:
    - an evidence note: an entry with the same reference and date, whose note begins with that prefix
      and contains the branch's note unchanged;
    - the decision's statement and the item's reason: an entry with the decision's reference and date,
      whose note begins with that prefix and contains both texts unchanged.
    Anything else (a word changed, another date, a missing entry, another candidate's prefix) is no
    excuse. Returns each excused text with the id of its item; empty if either file is not the ledger.
    """
    try:
        on_head = json.loads("\n".join(head_lines))
        on_main = json.loads("\n".join(main_lines))
        head_items = {item["id"]: item for item in on_head["items"]}
        main_items = {item["id"]: item for item in on_main["items"]}
    except (ValueError, KeyError, TypeError):
        return {}
    prefix = f"NOT COUNTED for {candidate}, kept as a record: "
    kept: dict[str, str] = {}

    def held(item_id: str, reference: object, date: object, texts: Sequence[str]) -> bool:
        prior = main_items[item_id].get("prior")
        if not isinstance(prior, list):
            return False
        wanted = record_reference(reference, moves)
        return any(
            isinstance(entry, dict)
            and entry.get("reference") == wanted
            and entry.get("date") == date
            and isinstance(entry.get("note"), str)
            and entry["note"].startswith(prefix)
            and all(text in entry["note"][len(prefix) :] for text in texts)
            for entry in prior
        )

    for item_id, item in head_items.items():
        if item_id not in main_items or not isinstance(item, dict):
            continue
        for entry in item.get("evidence") or []:
            if isinstance(entry, dict) and isinstance(entry.get("note"), str):
                if held(item_id, entry.get("reference"), entry.get("date"), [entry["note"]]):
                    kept[entry["note"]] = item_id
        decision = item.get("decision")
        reason = item.get("reason")
        if isinstance(decision, dict) and isinstance(decision.get("statement"), str) and isinstance(reason, str):
            if held(item_id, decision.get("reference"), decision.get("date"), [decision["statement"], reason]):
                kept[decision["statement"]] = item_id
                kept[reason] = item_id
    return kept


def json_string_value(line: str) -> str | None:
    """The string of a `"key": "value",` line of a JSON file written one entry per line, else None."""
    try:
        parsed = json.loads("{" + line.strip().rstrip(",") + "}")
    except ValueError:
        return None
    values = list(parsed.values()) if len(parsed) == 1 else []
    return values[0] if len(values) == 1 and isinstance(values[0], str) else None


def compare_change(
    repository: Path,
    change: Change,
    main: str,
    head: str,
    cut: str,
    commit: str,
    cache: FileCache,
    context: BranchContext = BranchContext(),
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
        if change.path in context.moves:
            return Comparison(None, 0, moved=((change.path, context.moves[change.path]),))
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
    # Excuse 6: a version line of a shipped document, while `main` has the newer version.
    versioned = PurePosixPath(change.path).as_posix() in VERSION_DOCUMENTS and context.main_is_newer
    main_versionless = {SEMVER.sub("<V>", line) for line in main_lines} if versioned else set()
    # Excuse 7: the ledger texts that `main` kept as records, while it is at a newer candidate.
    kept: dict[str, str] = {}
    if PurePosixPath(change.path).as_posix() == LEDGER_PATH and context.main_is_newer and context.main_version:
        kept = ledger_kept_as_records(on_head, on_main, context.main_version, context.moves)
    kept_items: set[str] = set()
    main_new_leads: set[str] = set()
    main_new_words: list[list[str]] = []
    # The same, with every version string blanked, for the sentence that `main` rewrote around a new version.
    versionless_leads: set[str] = set()
    versionless_words: list[list[str]] = []
    if rewritable or versioned:
        on_cut = file_lines(repository, cut, change.path, cache) or []
        cut_lines = {normalise(change.path, line) for line in on_cut}
        main_new_leads = {lead(line) for line in main_lines - cut_lines if lead(line)}
        main_new_words = [words(line) for line in main_lines - cut_lines]
        if versioned:
            blanked = [SEMVER.sub("<V>", line) for line in main_lines - cut_lines]
            versionless_leads = {lead(line) for line in blanked if lead(line)}
            versionless_words = [words(line) for line in blanked]

    absent: list[str] = []
    superseded = 0
    rewritten = 0
    renamed = 0
    version_lines = 0
    kept_lines = 0
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
            same_sentence = lead(compared) in main_new_leads or is_extension_of(compared, main_new_words)
            reworded = replaced_on_main or same_sentence
            # Excuse 5, then the test of excuse 3 on the line as `main` would write it.
            moved_line = context.substitute(compared)
            if moved_line != compared and (
                moved_line in main_lines
                or (
                    rewritable
                    and (lead(moved_line) in main_new_leads or is_extension_of(moved_line, main_new_words))
                )
            ):
                renamed += 1
            elif compared not in head_lines:
                superseded += 1
            elif (
                versioned
                and SEMVER.search(compared)
                and (
                    SEMVER.sub("<V>", compared) in main_versionless
                    or same_sentence
                    or lead(SEMVER.sub("<V>", compared)) in versionless_leads
                    or is_extension_of(SEMVER.sub("<V>", compared), versionless_words)
                    # The line replaced one that named a version, and `main` has none of what it replaced.
                    or (replaced_on_main and any(SEMVER.search(line) for line in replaced))
                )
            ):
                version_lines += 1
            elif (value := json_string_value(line)) is not None and value in kept:
                kept_lines += 1
                kept_items.add(kept[value])
            elif rewritable and reworded:
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
    if renamed:
        excused.append(f"{renamed} added line(s) naming a path that main moved, as main has them")
    if kept_lines:
        excused.append(
            f"{kept_lines} added line(s) of {', '.join(sorted(kept_items))}: kept as a record in prior, "
            f"as the bump to {context.main_version} does"
        )
    if version_lines:
        excused.append(
            f"{version_lines} added line(s) naming a version, rewritten on main for its newer version "
            f"{context.main_version} (the branch is at {context.head_version})"
        )
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
    context = make_context(repository, main, head)
    reports: list[CommitReport] = []
    cache: FileCache = {}
    for commit in listed:
        subject = text(run_git(repository, "show", "-s", "--format=%s", commit)).strip()
        findings: list[Finding] = []
        notes: list[str] = []
        moved: list[tuple[str, str]] = []
        compared = 0
        kinds = set()
        for change in commit_changes(repository, commit):
            kinds.add(kind_of(change.path))
            comparison = compare_change(repository, change, main, head, cut, commit, cache, context)
            compared += comparison.compared
            notes.extend(comparison.notes)
            moved.extend(comparison.moved)
            if comparison.finding is not None:
                findings.append(comparison.finding)
        notes.extend(describe_moves(moved))
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
