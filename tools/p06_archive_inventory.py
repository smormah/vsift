"""List every entry of a pinned candidate archive, for the review of a re-pin.

A re-pin of a reviewed tool needs the size and SHA-256 of each file it selects
from the publisher's archive, and the archive's entry count and expanded size
for the managed catalogue's bounds. The publisher publishes only the archive's
own digest. This tool reads the rest on a disposable GitHub Actions runner, so
that nothing is downloaded to a developer machine: it fetches one pinned
archive with the same bounded, HTTPS-only, size-and-SHA-256-checked download
the candidate smokes use, then walks the archive index and streams each regular
file through SHA-256. Nothing is extracted to disk and nothing in the archive
is run. The result is printed as one JSON document (and as a table in the job
summary); the pins are then copied from it by hand and reviewed.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import stat
import sys
import tarfile
import tempfile
import zipfile

import p06_ubuntu_candidate_smoke as ubuntu
import p06_windows_candidate_smoke as windows
from p06_windows_candidate_smoke import CandidateRejected, validate_archive_path, verified_download


# Generous bounds: an inventory is read before any per-file pin exists, so it
# must accept a release that grew, but a hostile index still cannot make it
# hash without end.
MAX_ENTRIES = 1_024
MAX_EXPANDED_BYTES = 4_000_000_000
CHUNK_BYTES = 1024 * 1024

PINS = {
    "ubuntu-whisper": ("tar", ubuntu.ARCHIVES["whisper"]),
    "windows-whisper": ("zip", windows.ARCHIVES["whisper"]),
}


def stream_digest(source, expected_size: int) -> str:
    """SHA-256 of exactly `expected_size` bytes from `source`, failing on any other length."""
    digest = hashlib.sha256()
    read = 0
    while chunk := source.read(CHUNK_BYTES):
        read += len(chunk)
        if read > expected_size:
            raise CandidateRejected("archive entry is longer than its index says")
        digest.update(chunk)
    if read != expected_size:
        raise CandidateRejected("archive entry is shorter than its index says")
    return digest.hexdigest()


def check_bounds(entries: int, expanded: int) -> None:
    if entries > MAX_ENTRIES or expanded > MAX_EXPANDED_BYTES:
        raise CandidateRejected("archive entry or expanded-size limit exceeded")


def inventory_tar(archive_path: Path) -> dict:
    """Every member of a tar archive: path, kind, size, SHA-256 and mode or link target."""
    entries = []
    with tarfile.open(archive_path, "r:*") as archive:
        members = archive.getmembers()
        check_bounds(len(members), sum(member.size for member in members))
        for member in members:
            path = str(validate_archive_path(member.name))
            if member.isfile():
                source = archive.extractfile(member)
                if source is None:
                    raise CandidateRejected("archive file is unreadable")
                with source:
                    digest = stream_digest(source, member.size)
                entries.append({
                    "path": path,
                    "kind": "file",
                    "bytes": member.size,
                    "sha256": digest,
                    "executable": bool(member.mode & stat.S_IXUSR),
                })
            elif member.isdir():
                entries.append({"path": path, "kind": "directory"})
            elif member.issym():
                entries.append({"path": path, "kind": "symlink", "target": member.linkname})
            elif member.islnk():
                entries.append({"path": path, "kind": "hardlink", "target": member.linkname})
            else:
                entries.append({"path": path, "kind": "special"})
    return summarise(entries)


def inventory_zip(archive_path: Path) -> dict:
    """Every entry of a ZIP archive: path, kind, size and SHA-256."""
    entries = []
    with zipfile.ZipFile(archive_path) as archive:
        infos = archive.infolist()
        check_bounds(len(infos), sum(info.file_size for info in infos))
        for info in infos:
            path = str(validate_archive_path(info.filename))
            mode = (info.external_attr >> 16) & 0o170000
            if info.is_dir():
                entries.append({"path": path, "kind": "directory"})
            elif mode not in {0, stat.S_IFREG}:
                entries.append({"path": path, "kind": "special"})
            else:
                with archive.open(info) as source:
                    digest = stream_digest(source, info.file_size)
                entries.append({
                    "path": path,
                    "kind": "file",
                    "bytes": info.file_size,
                    "sha256": digest,
                })
    return summarise(entries)


def summarise(entries: list[dict]) -> dict:
    """The entries in path order with the totals the managed catalogue's bounds use."""
    ordered = sorted(entries, key=lambda entry: entry["path"])
    return {
        "entries": len(ordered),
        "expanded_bytes": sum(entry.get("bytes", 0) for entry in ordered),
        "members": ordered,
    }


def summary_table(name: str, url: str, inventory: dict) -> str:
    """A Markdown table of the inventory for the job summary."""
    lines = [
        f"### Inventory of `{name}`",
        "",
        f"`{url}`: {inventory['entries']} entries, {inventory['expanded_bytes']} expanded bytes.",
        "",
        "| Path | Kind | Bytes | SHA-256 or target |",
        "| --- | --- | ---: | --- |",
    ]
    for entry in inventory["members"]:
        detail = entry.get("sha256") or entry.get("target") or ""
        lines.append(f"| `{entry['path']}` | {entry['kind']} | {entry.get('bytes', '')} | `{detail}` |")
    return "\n".join(lines) + "\n"


def main(arguments: list[str]) -> None:
    if not os.environ.get("GITHUB_ACTIONS"):
        raise CandidateRejected("pinned archives may be fetched only on an Actions runner")
    if len(arguments) != 1 or arguments[0] not in PINS:
        raise CandidateRejected(f"usage: p06_archive_inventory.py <{'|'.join(sorted(PINS))}>")
    name = arguments[0]
    kind, (url, size, digest) = PINS[name]
    with tempfile.TemporaryDirectory(prefix="vsift-p06-inventory-") as temporary:
        archive_path = Path(temporary) / "candidate.archive"
        verified_download(url, archive_path, size, digest)
        inventory = inventory_tar(archive_path) if kind == "tar" else inventory_zip(archive_path)
    document = {"pin": name, "url": url, "bytes": size, "sha256": digest, **inventory}
    print(json.dumps(document, indent=2, sort_keys=True))
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as summary_file:
            summary_file.write(summary_table(name, url, inventory))


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except (CandidateRejected, OSError, ValueError, tarfile.TarError, zipfile.BadZipFile) as error:
        print(f"P06 archive inventory failed: {error}", file=sys.stderr)
        sys.exit(1)
