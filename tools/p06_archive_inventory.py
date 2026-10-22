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
    "windows-ffmpeg": ("zip", windows.ARCHIVES["ffmpeg"]),
}

# ADR 0032: the PE import reading below reads at most this many DLL names of
# one executable, each at most this long, so a hostile file cannot make it
# loop or allocate without end.
MAX_IMPORTED_NAMES = 256
MAX_IMPORTED_NAME_BYTES = 255


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


def pe_imports(content: bytes) -> list[str] | None:
    """The DLL names a PE executable imports, directly or delay-loaded.

    ADR 0032, the supervisor's decision D2: a Windows build that imports the
    Microsoft C runtime (`vcruntime140.dll`, `msvcp140.dll`) or OpenMP
    (`vcomp140.dll`) needs a redistributable a clean Windows may lack. This
    reads the import and delay-import directories of a PE32 or PE32+ file
    from its bytes; nothing is run. `None` when the bytes are not a PE file.
    """
    def u16(offset: int) -> int:
        if offset < 0 or offset + 2 > len(content):
            raise CandidateRejected("PE header runs past the file")
        return int.from_bytes(content[offset:offset + 2], "little")

    def u32(offset: int) -> int:
        if offset < 0 or offset + 4 > len(content):
            raise CandidateRejected("PE header runs past the file")
        return int.from_bytes(content[offset:offset + 4], "little")

    if content[:2] != b"MZ" or len(content) < 0x40:
        return None
    header = u32(0x3C)
    if content[header:header + 4] != b"PE\0\0":
        return None
    sections = u16(header + 6)
    optional_size = u16(header + 20)
    optional = header + 24
    magic = u16(optional)
    if magic == 0x10B:
        directories = optional + 96
    elif magic == 0x20B:
        directories = optional + 112
    else:
        raise CandidateRejected("PE optional header has an unknown magic")
    directory_count = u32(directories - 4)
    table = optional + optional_size
    mapped = []
    for index in range(min(sections, 96)):
        entry = table + index * 40
        virtual_size, virtual_address = u32(entry + 8), u32(entry + 12)
        raw_size, raw_offset = u32(entry + 16), u32(entry + 20)
        mapped.append((virtual_address, max(virtual_size, raw_size), raw_offset))

    def offset_of(rva: int) -> int:
        for virtual_address, size, raw_offset in mapped:
            if virtual_address <= rva < virtual_address + size:
                return raw_offset + (rva - virtual_address)
        raise CandidateRejected("PE address is outside every section")

    def name_at(rva: int) -> str:
        start = offset_of(rva)
        end = content.find(b"\0", start, start + MAX_IMPORTED_NAME_BYTES + 1)
        if end < 0:
            raise CandidateRejected("PE import name is unterminated or too long")
        raw = content[start:end]
        if not raw or not all(0x20 < byte < 0x7F for byte in raw):
            raise CandidateRejected("PE import name is not plain ASCII")
        return raw.decode("ascii").lower()

    names: set[str] = set()
    # (data directory, descriptor size, offset of the DLL name's address):
    # the import table, then the delay-load table.
    for index, size, name_field in ((1, 20, 12), (13, 32, 4)):
        if index >= directory_count:
            continue
        rva = u32(directories + index * 8)
        if rva == 0:
            continue
        position = offset_of(rva)
        for _ in range(MAX_IMPORTED_NAMES + 1):
            if position + size > len(content):
                raise CandidateRejected("PE import table runs past the file")
            if not any(content[position:position + size]):
                break
            names.add(name_at(u32(position + name_field)))
            if len(names) > MAX_IMPORTED_NAMES:
                raise CandidateRejected("PE file imports too many libraries")
            position += size
    return sorted(names)


def has_extra_tag(extra: bytes, wanted: int) -> bool:
    """Whether a zip extra field holds a block with the tag `wanted`."""
    position = 0
    while position + 4 <= len(extra):
        tag = int.from_bytes(extra[position:position + 2], "little")
        length = int.from_bytes(extra[position + 2:position + 4], "little")
        if tag == wanted:
            return True
        position += 4 + length
    return False


def zip_features(archive: zipfile.ZipFile) -> dict:
    """What the archive's writer used, against the managed zip reader's policy.

    ADR 0032: the reader refuses Zip64, comments, methods other than stored
    (0) and deflated (8), backslash separators and made-by hosts other than
    MS-DOS (0), Unix (3), NTFS (10) and macOS (19), so each pin's inventory
    says whether its archive uses any of them.
    """
    infos = archive.infolist()
    return {
        "archive_comment_bytes": len(archive.comment),
        "methods": sorted({info.compress_type for info in infos}),
        "made_by_hosts": sorted({info.create_system for info in infos}),
        "flag_bits": sorted({info.flag_bits for info in infos}),
        "entry_comments": sum(1 for info in infos if info.comment),
        "backslash_names": sum(1 for info in infos if "\\" in info.orig_filename),
        "zip64_extras": sum(1 for info in infos if has_extra_tag(info.extra, 0x0001)),
    }


def inventory_zip(archive_path: Path) -> dict:
    """Every entry of a ZIP archive: path, kind, size, SHA-256 and, for a PE
    executable or library, the DLLs it imports; and the writer's features."""
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
                entry = {
                    "path": path,
                    "kind": "file",
                    "bytes": info.file_size,
                    "sha256": digest,
                }
                if path.lower().endswith((".exe", ".dll")):
                    with archive.open(info) as source:
                        imports = pe_imports(source.read(info.file_size + 1))
                    if imports is not None:
                        entry["pe_imports"] = imports
                entries.append(entry)
        features = zip_features(archive)
    summary = summarise(entries)
    summary["zip_features"] = features
    return summary


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
    imported = [entry for entry in inventory["members"] if "pe_imports" in entry]
    if imported:
        lines += ["", "| PE file | Imports |", "| --- | --- |"]
        for entry in imported:
            lines.append(f"| `{entry['path']}` | {', '.join(entry['pe_imports'])} |")
    if "zip_features" in inventory:
        lines += ["", f"Zip features: `{json.dumps(inventory['zip_features'], sort_keys=True)}`"]
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
