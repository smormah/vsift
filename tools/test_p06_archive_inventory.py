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


if __name__ == "__main__":
    unittest.main()
