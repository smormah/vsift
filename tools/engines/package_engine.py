"""Package a VSift-built engine deterministically and describe it for the catalogue.

ADR 0032 section E: an engine VSift builds itself (whisper.cpp's command line
and an LGPL FFmpeg for macOS arm64, later the OCR runner) is published on this
repository's releases and pinned by the managed catalogue like any publisher's
artifact. This script turns one built tree into:

- `<root>.tar.gz`: a tar of the tree under one root folder, with every entry
  in path order, owner and group 0 with no names, a fixed modification time
  (the upstream commit's, `SOURCE_DATE_EPOCH`), mode 0755 for an executable
  and 0644 otherwise, regular files and folders only, then gzip with no file
  name and the same time. The same tree gives the same bytes, so a second
  build compares byte for byte.
- `<root>.inventory.json`: the archive's size and SHA-256, its entry count
  and expanded bytes (the catalogue's `archive_limits`), and each file's path,
  size, SHA-256 and whether it is executable: the numbers a catalogue entry
  pins, read from the bytes that are published.

It reads only the folder it is given and writes only the two named outputs.
Nothing is downloaded and nothing in the tree is run.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import stat
import sys
import tarfile

# Bounds of the tree a recipe may package, far above the engines planned.
MAX_ENTRIES = 512
MAX_EXPANDED_BYTES = 1_073_741_824
CHUNK_BYTES = 1024 * 1024
SAFE_NAME_BYTES = frozenset(
    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-"
)


class PackageRefused(Exception):
    """The tree cannot be packaged under the engine archive rules."""


def safe_segment(name: str) -> bool:
    raw = name.encode("utf-8", "surrogateescape")
    return (
        0 < len(raw) <= 128
        and name not in {".", ".."}
        and all(byte in SAFE_NAME_BYTES for byte in raw)
    )


def collect(tree: Path) -> list[tuple[str, Path, bool]]:
    """Every folder and regular file under `tree` as (archive path, source,
    is a folder), in path order; links and special files are refused."""
    found: list[tuple[str, Path, bool]] = []
    for current, folders, files in os.walk(tree, followlinks=False):
        folders.sort()
        base = Path(current)
        for name in sorted(folders) + sorted(files):
            path = base / name
            relative = path.relative_to(tree).as_posix()
            if not all(safe_segment(part) for part in relative.split("/")):
                raise PackageRefused(f"unsafe name in the tree: {relative!r}")
            info = path.lstat()
            if stat.S_ISLNK(info.st_mode):
                raise PackageRefused(f"a link in the tree: {relative}")
            if stat.S_ISDIR(info.st_mode):
                found.append((relative, path, True))
            elif stat.S_ISREG(info.st_mode):
                found.append((relative, path, False))
            else:
                raise PackageRefused(f"a special file in the tree: {relative}")
    found.sort(key=lambda entry: entry[0])
    if not found:
        raise PackageRefused("the tree is empty")
    if len(found) + 1 > MAX_ENTRIES:
        raise PackageRefused("the tree has too many entries")
    return found


def is_executable(path: Path) -> bool:
    return bool(path.lstat().st_mode & stat.S_IXUSR)


def digest_file(path: Path) -> tuple[int, str]:
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as source:
        while chunk := source.read(CHUNK_BYTES):
            size += len(chunk)
            digest.update(chunk)
    return size, digest.hexdigest()


def package(tree: Path, root: str, mtime: int, archive_path: Path, inventory_path: Path) -> dict:
    """Writes the archive and its inventory and returns the inventory."""
    if not safe_segment(root):
        raise PackageRefused(f"unsafe root folder name: {root!r}")
    if mtime < 0:
        raise PackageRefused("the modification time is negative")
    entries = collect(tree)
    files = []
    expanded = 0
    tar_bytes = io.BytesIO()
    with tarfile.open(fileobj=tar_bytes, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        folder = tarfile.TarInfo(root)
        folder.type = tarfile.DIRTYPE
        folder.mode = 0o755
        folder.mtime = mtime
        archive.addfile(folder)
        for relative, source, is_folder in entries:
            member = tarfile.TarInfo(f"{root}/{relative}")
            member.mtime = mtime
            member.uid = member.gid = 0
            member.uname = member.gname = ""
            if is_folder:
                member.type = tarfile.DIRTYPE
                member.mode = 0o755
                archive.addfile(member)
                continue
            # Windows keeps no executable bit; a Windows program is its .exe.
            executable = is_executable(source) or relative.lower().endswith(".exe")
            size, sha256 = digest_file(source)
            expanded += size
            if expanded > MAX_EXPANDED_BYTES:
                raise PackageRefused("the tree is too large")
            member.mode = 0o755 if executable else 0o644
            member.size = size
            with source.open("rb") as content:
                archive.addfile(member, content)
            files.append({
                "path": f"{root}/{relative}",
                "bytes": size,
                "sha256": sha256,
                "executable": executable,
            })
    stream = tar_bytes.getvalue()
    compressed = io.BytesIO()
    with gzip.GzipFile(filename="", mode="wb", fileobj=compressed, mtime=mtime, compresslevel=9) as gz:
        gz.write(stream)
    archive_bytes = compressed.getvalue()
    archive_path.write_bytes(archive_bytes)
    inventory = {
        "archive": archive_path.name,
        "bytes": len(archive_bytes),
        "sha256": hashlib.sha256(archive_bytes).hexdigest(),
        "format": "tar_gz",
        "root": root,
        "entries": len(entries) + 1,
        "expanded_bytes": expanded,
        "tar_stream_bytes": len(stream),
        "files": files,
    }
    inventory_path.write_text(json.dumps(inventory, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return inventory


def main(arguments: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--tree", required=True, type=Path)
    parser.add_argument("--root", required=True)
    parser.add_argument("--mtime", required=True, type=int)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--inventory", required=True, type=Path)
    options = parser.parse_args(arguments)
    try:
        package(options.tree, options.root, options.mtime, options.archive, options.inventory)
    except (PackageRefused, OSError) as error:
        print(f"Engine packaging refused: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
