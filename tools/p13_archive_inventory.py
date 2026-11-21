"""Check a pinned bzip2 tar archive's bounds against the managed catalogue's pins.

The speaker group's tool comes from sherpa-onnx's own release archive (ADR 0031,
ADR 0032), whose only published digest is GitHub's. The catalogue also pins the
archive's bounds: the most entries it may hold, the sum of its entries' declared
sizes, and a cap on its decompressed tar stream. Those numbers are read on a
disposable GitHub Actions runner, where this script fetches the one pinned archive
(HTTPS from `github.com`, at most the pinned size, SHA-256 checked) and walks it
without extracting anything to disk; nothing in it is run. It fails if the archive
is not the pinned bytes, if it holds more entries or a longer stream than the
catalogue allows, or if the sum of its declared sizes is not the pinned one, so a
pin that does not describe the archive shows here and not in a user's install.

The inputs are environment variables, so a workflow step cannot pass an argument
that reaches a shell: ``ARCHIVE_URL``, ``ARCHIVE_BYTES``, ``ARCHIVE_SHA256``,
``PINNED_ENTRIES``, ``PINNED_EXPANDED`` and ``PINNED_STREAM``. The request names no
person and carries only a generic client name.
"""

from __future__ import annotations

import bz2
import hashlib
import os
import sys
import tarfile
import tempfile
import urllib.parse
import urllib.request
from pathlib import Path

CLIENT_NAME = "vsift-archive-inventory"
CHUNK_BYTES = 1024 * 1024
# A hard ceiling whatever the pins say: a hostile archive cannot make this loop
# or fill a disk without end.
MAX_TAR_STREAM_BYTES = 1024 * 1024 * 1024
MAX_ENTRIES = 4096


class InventoryError(Exception):
    """The archive is not the one the catalogue pins, or exceeds its bounds."""


def download(url: str, expected_bytes: int, expected_sha256: str, destination: Path) -> None:
    """Fetches `url` to `destination` and checks its size and SHA-256 exactly."""
    parts = urllib.parse.urlsplit(url)
    if parts.scheme != "https" or parts.hostname != "github.com" or parts.query or parts.fragment:
        raise InventoryError("the archive is not a plain HTTPS github.com address")
    request = urllib.request.Request(url, headers={"User-Agent": CLIENT_NAME})
    digest = hashlib.sha256()
    read = 0
    with urllib.request.urlopen(request, timeout=300) as response, destination.open("wb") as sink:
        while chunk := response.read(CHUNK_BYTES):
            read += len(chunk)
            if read > expected_bytes:
                raise InventoryError("the archive is longer than its pinned size")
            digest.update(chunk)
            sink.write(chunk)
    if read != expected_bytes:
        raise InventoryError(f"the archive is {read} bytes, not the pinned {expected_bytes}")
    if digest.hexdigest() != expected_sha256:
        raise InventoryError("the archive's SHA-256 is not the pinned one")


def tar_stream(archive: Path, destination: Path) -> int:
    """Decompresses the one bzip2 stream of `archive` to `destination`; returns its length.

    A second stream or any byte after the first is refused, as the catalogue's
    reader refuses it.
    """
    decoder = bz2.BZ2Decompressor()
    length = 0
    with archive.open("rb") as source, destination.open("wb") as sink:
        while chunk := source.read(CHUNK_BYTES):
            if decoder.eof:
                raise InventoryError("bytes follow the archive's one bzip2 stream")
            data = decoder.decompress(chunk)
            length += len(data)
            if length > MAX_TAR_STREAM_BYTES:
                raise InventoryError("the archive's tar stream is longer than the hard ceiling")
            sink.write(data)
            if decoder.eof and decoder.unused_data:
                raise InventoryError("bytes follow the archive's one bzip2 stream")
    if not decoder.eof:
        raise InventoryError("the archive's bzip2 stream is cut short")
    return length


def read_tar(path: Path) -> dict:
    """Entries, declared sizes and kinds of the tar at `path`; nothing is extracted."""
    entries = 0
    declared = 0
    kinds: dict[str, int] = {}
    with tarfile.open(path, mode="r:") as tar:
        for member in tar:
            entries += 1
            if entries > MAX_ENTRIES:
                raise InventoryError("the archive holds more entries than the hard ceiling")
            declared += member.size
            kind = "directory" if member.isdir() else "file" if member.isreg() else "other"
            kinds[kind] = kinds.get(kind, 0) + 1
    return {"entries": entries, "expanded_bytes": declared, "kinds": kinds}


def inventory(url: str, bytes_: int, sha256: str, work: Path) -> dict:
    """Downloads, verifies and measures one archive."""
    archive = work / "archive.tar.bz2"
    download(url, bytes_, sha256, archive)
    tar_path = work / "archive.tar"
    stream = tar_stream(archive, tar_path)
    measured = read_tar(tar_path)
    measured["tar_stream_bytes"] = stream
    return measured


def compare(measured: dict, pinned_entries: int, pinned_expanded: int, pinned_stream: int) -> list[str]:
    """What the pins get wrong, as sentences; empty when they describe the archive."""
    problems = []
    if measured["entries"] > pinned_entries:
        problems.append(f"the archive holds {measured['entries']} entries; the pin allows {pinned_entries}")
    if measured["expanded_bytes"] != pinned_expanded:
        problems.append(
            f"the archive's declared sizes add up to {measured['expanded_bytes']} bytes; the pin says {pinned_expanded}"
        )
    if measured["tar_stream_bytes"] > pinned_stream:
        problems.append(
            f"the archive's tar stream is {measured['tar_stream_bytes']} bytes; the pin allows {pinned_stream}"
        )
    return problems


def summary(url: str, measured: dict, pins: tuple[int, int, int], problems: list[str]) -> str:
    """The job summary: the measured numbers beside the pins."""
    entries, expanded, stream = pins
    lines = [
        "### sherpa-onnx archive against the catalogue's pins",
        "",
        f"`{url.rsplit('/', 1)[-1]}`",
        "",
        "| | measured | pinned |",
        "| --- | ---: | ---: |",
        f"| entries (at most) | {measured['entries']} | {entries} |",
        f"| declared sizes (exact) | {measured['expanded_bytes']} | {expanded} |",
        f"| tar stream (at most) | {measured['tar_stream_bytes']} | {stream} |",
        "",
        f"Entry kinds: {measured['kinds']}.",
        "",
    ]
    lines += [f"**{problem}**" for problem in problems] or ["The pins describe the archive."]
    return "\n".join(lines) + "\n"


def main() -> int:
    """Reads the environment, measures the archive and fails on a wrong pin."""
    try:
        url = os.environ["ARCHIVE_URL"]
        bytes_ = int(os.environ["ARCHIVE_BYTES"])
        sha256 = os.environ["ARCHIVE_SHA256"]
        pins = (
            int(os.environ["PINNED_ENTRIES"]),
            int(os.environ["PINNED_EXPANDED"]),
            int(os.environ["PINNED_STREAM"]),
        )
    except (KeyError, ValueError) as error:
        print(f"missing or malformed input: {error!r}", file=sys.stderr)
        return 2
    with tempfile.TemporaryDirectory() as folder:
        try:
            measured = inventory(url, bytes_, sha256, Path(folder))
        except (InventoryError, tarfile.TarError, OSError, EOFError) as error:
            print(f"the archive could not be measured: {error}", file=sys.stderr)
            return 1
    problems = compare(measured, *pins)
    text = summary(url, measured, pins, problems)
    print(text)
    target = os.environ.get("GITHUB_STEP_SUMMARY")
    if target:
        with open(target, "a", encoding="utf-8") as sink:
            sink.write(text)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
