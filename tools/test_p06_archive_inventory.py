"""The re-pin inventory reads every entry and fails closed on a hostile index."""

from __future__ import annotations

import hashlib
import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest import mock
import zipfile

import p06_archive_inventory as inventory
from p06_windows_candidate_smoke import CandidateRejected


def add_tar_file(archive: tarfile.TarFile, name: str, content: bytes, mode: int) -> None:
    member = tarfile.TarInfo(name)
    member.size = len(content)
    member.mode = mode
    archive.addfile(member, io.BytesIO(content))


class TarInventoryTests(unittest.TestCase):
    def test_lists_files_links_and_directories_with_digests_and_totals(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "candidate.tar.gz"
            with tarfile.open(path, "w:gz") as archive:
                directory = tarfile.TarInfo("release")
                directory.type = tarfile.DIRTYPE
                archive.addfile(directory)
                add_tar_file(archive, "release/tool", b"binary", 0o755)
                add_tar_file(archive, "release/libname.so.1.2.3", b"library", 0o644)
                link = tarfile.TarInfo("release/libname.so.1")
                link.type = tarfile.SYMTYPE
                link.linkname = "libname.so.1.2.3"
                archive.addfile(link)
            result = inventory.inventory_tar(path)
        self.assertEqual(result["entries"], 4)
        self.assertEqual(result["expanded_bytes"], len(b"binary") + len(b"library"))
        by_path = {entry["path"]: entry for entry in result["members"]}
        self.assertEqual(by_path["release"], {"path": "release", "kind": "directory"})
        self.assertEqual(by_path["release/tool"]["sha256"], hashlib.sha256(b"binary").hexdigest())
        self.assertTrue(by_path["release/tool"]["executable"])
        self.assertFalse(by_path["release/libname.so.1.2.3"]["executable"])
        self.assertEqual(by_path["release/libname.so.1"],
                         {"path": "release/libname.so.1", "kind": "symlink",
                          "target": "libname.so.1.2.3"})
        self.assertEqual([entry["path"] for entry in result["members"]],
                         sorted(entry["path"] for entry in result["members"]))

    def test_rejects_a_traversing_path(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "candidate.tar.gz"
            with tarfile.open(path, "w:gz") as archive:
                add_tar_file(archive, "release/../escape", b"bad", 0o644)
            with self.assertRaises(CandidateRejected):
                inventory.inventory_tar(path)

    def test_rejects_an_index_over_the_entry_bound(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "candidate.tar.gz"
            with tarfile.open(path, "w:gz") as archive:
                for index in range(3):
                    add_tar_file(archive, f"release/file-{index}", b"x", 0o644)
            with mock.patch.object(inventory, "MAX_ENTRIES", 2):
                with self.assertRaises(CandidateRejected):
                    inventory.inventory_tar(path)


class ZipInventoryTests(unittest.TestCase):
    def test_lists_files_and_directories_with_digests(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "candidate.zip"
            with zipfile.ZipFile(path, "w") as archive:
                archive.writestr("Release/", b"")
                archive.writestr("Release/tool.exe", b"program")
            result = inventory.inventory_zip(path)
        self.assertEqual(result["entries"], 2)
        self.assertEqual(result["expanded_bytes"], len(b"program"))
        self.assertEqual(result["members"][1],
                         {"path": "Release/tool.exe", "kind": "file", "bytes": 7,
                          "sha256": hashlib.sha256(b"program").hexdigest()})

    def test_a_link_entry_is_listed_as_special_and_not_read(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "candidate.zip"
            with zipfile.ZipFile(path, "w") as archive:
                info = zipfile.ZipInfo("Release/link")
                info.external_attr = 0o120777 << 16
                archive.writestr(info, b"target")
            result = inventory.inventory_zip(path)
        self.assertEqual(result["members"], [{"path": "Release/link", "kind": "special"}])

    def test_rejects_an_index_over_the_size_bound(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "candidate.zip"
            with zipfile.ZipFile(path, "w") as archive:
                archive.writestr("Release/tool.exe", b"program")
            with mock.patch.object(inventory, "MAX_EXPANDED_BYTES", 3):
                with self.assertRaises(CandidateRejected):
                    inventory.inventory_zip(path)


class EntryPointTests(unittest.TestCase):
    def test_refuses_to_fetch_outside_an_actions_runner(self) -> None:
        with mock.patch.dict("os.environ", {}, clear=True):
            with self.assertRaises(CandidateRejected):
                inventory.main(["ubuntu-whisper"])

    def test_refuses_an_unknown_pin(self) -> None:
        with mock.patch.dict("os.environ", {"GITHUB_ACTIONS": "true"}, clear=True):
            with self.assertRaises(CandidateRejected):
                inventory.main(["ffmpeg"])

    def test_a_short_entry_fails_closed(self) -> None:
        with self.assertRaises(CandidateRejected):
            inventory.stream_digest(io.BytesIO(b"abc"), 4)
        with self.assertRaises(CandidateRejected):
            inventory.stream_digest(io.BytesIO(b"abcde"), 4)


def minimal_pe(imports: list[str], delayed: list[str]) -> bytes:
    """A PE32+ image with one section holding an import and a delay-import
    table that name `imports` and `delayed`."""
    section_rva, section_offset = 0x1000, 0x200
    data = bytearray()
    names_at = {}
    for name in imports + delayed:
        names_at[name] = section_rva + 0x400 + len(data)
        data += name.encode("ascii") + b"\0"
    imports_table = bytearray()
    for name in imports:
        descriptor = bytearray(20)
        descriptor[12:16] = names_at[name].to_bytes(4, "little")
        imports_table += descriptor
    imports_table += bytes(20)
    delayed_table = bytearray()
    for name in delayed:
        descriptor = bytearray(32)
        descriptor[0:4] = (1).to_bytes(4, "little")
        descriptor[4:8] = names_at[name].to_bytes(4, "little")
        delayed_table += descriptor
    delayed_table += bytes(32)
    section = bytearray(0x800)
    section[0:len(imports_table)] = imports_table
    section[0x200:0x200 + len(delayed_table)] = delayed_table
    section[0x400:0x400 + len(data)] = data
    image = bytearray(section_offset)
    image[0:2] = b"MZ"
    image[0x3C:0x40] = (0x40).to_bytes(4, "little")
    image[0x40:0x44] = b"PE\0\0"
    image[0x46:0x48] = (1).to_bytes(2, "little")  # sections
    optional_size = 112 + 16 * 8
    image[0x54:0x56] = optional_size.to_bytes(2, "little")
    optional = 0x58
    image[optional:optional + 2] = (0x20B).to_bytes(2, "little")
    directories = optional + 112
    image[directories - 4:directories] = (16).to_bytes(4, "little")
    image[directories + 8:directories + 12] = section_rva.to_bytes(4, "little")
    image[directories + 13 * 8:directories + 13 * 8 + 4] = (section_rva + 0x200).to_bytes(4, "little")
    table = optional + optional_size
    image[table:table + 8] = b".rdata\0\0"
    image[table + 8:table + 12] = len(section).to_bytes(4, "little")
    image[table + 12:table + 16] = section_rva.to_bytes(4, "little")
    image[table + 16:table + 20] = len(section).to_bytes(4, "little")
    image[table + 20:table + 24] = section_offset.to_bytes(4, "little")
    return bytes(image + section)


class PeImportTests(unittest.TestCase):
    def test_reads_direct_and_delay_loaded_imports(self) -> None:
        image = minimal_pe(["KERNEL32.dll", "VCRUNTIME140.dll"], ["VCOMP140.DLL"])
        self.assertEqual(inventory.pe_imports(image),
                         ["kernel32.dll", "vcomp140.dll", "vcruntime140.dll"])

    def test_a_non_pe_file_has_no_imports(self) -> None:
        self.assertIsNone(inventory.pe_imports(b"#!/bin/sh\n"))
        self.assertIsNone(inventory.pe_imports(b"MZ" + bytes(0x40)))

    def test_a_truncated_or_hostile_header_fails_closed(self) -> None:
        image = minimal_pe(["KERNEL32.dll"], [])
        for cut in (0x48, 0x60, 0x180, 0x210):
            with self.assertRaises(CandidateRejected):
                inventory.pe_imports(image[:cut])
        unterminated = bytearray(image)
        start = unterminated.index(b"KERNEL32.dll")
        unterminated[start:] = b"A" * (len(unterminated) - start)
        with self.assertRaises(CandidateRejected):
            inventory.pe_imports(bytes(unterminated))

    def test_a_zip_inventory_lists_pe_imports_and_the_writers_features(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "candidate.zip"
            with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
                archive.writestr("Release/tool.exe", minimal_pe(["MSVCP140.dll"], []))
                archive.writestr("Release/notes.txt", b"text")
            result = inventory.inventory_zip(path)
        by_path = {entry["path"]: entry for entry in result["members"]}
        self.assertEqual(by_path["Release/tool.exe"]["pe_imports"], ["msvcp140.dll"])
        self.assertNotIn("pe_imports", by_path["Release/notes.txt"])
        features = result["zip_features"]
        self.assertEqual(features["methods"], [zipfile.ZIP_DEFLATED])
        self.assertEqual(features["backslash_names"], 0)
        self.assertEqual(features["zip64_extras"], 0)
        self.assertEqual(features["archive_comment_bytes"], 0)


if __name__ == "__main__":
    unittest.main()
