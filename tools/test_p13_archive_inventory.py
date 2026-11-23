"""Tests of the sherpa-onnx archive bounds check (no network)."""

from __future__ import annotations

import bz2
import io
import tarfile
import tempfile
import unittest
from pathlib import Path

import p13_archive_inventory as tool


def make_tar(files: dict[str, bytes]) -> bytes:
    """A tar with a root folder, a subfolder and `files`, as a release archive has."""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as tar:
        for folder in ("root", "root/bin"):
            info = tarfile.TarInfo(folder)
            info.type = tarfile.DIRTYPE
            tar.addfile(info)
        for name, content in files.items():
            info = tarfile.TarInfo(f"root/bin/{name}")
            info.size = len(content)
            tar.addfile(info, io.BytesIO(content))
    return buffer.getvalue()


class MeasureTests(unittest.TestCase):
    def measure(self, archive: bytes) -> dict:
        with tempfile.TemporaryDirectory() as folder:
            work = Path(folder)
            compressed = work / "archive.tar.bz2"
            compressed.write_bytes(archive)
            tar_path = work / "archive.tar"
            stream = tool.tar_stream(compressed, tar_path)
            measured = tool.read_tar(tar_path)
            measured["tar_stream_bytes"] = stream
            return measured

    def test_counts_entries_declared_sizes_and_the_tar_stream(self) -> None:
        raw = make_tar({"tool": b"a" * 1000, "library": b"b" * 2500})
        measured = self.measure(bz2.compress(raw))
        self.assertEqual(measured["entries"], 4)
        self.assertEqual(measured["expanded_bytes"], 3500)
        self.assertEqual(measured["kinds"], {"directory": 2, "file": 2})
        self.assertEqual(measured["tar_stream_bytes"], len(raw))

    def test_a_second_stream_or_trailing_bytes_are_refused(self) -> None:
        raw = make_tar({"tool": b"a"})
        for archive in (bz2.compress(raw) + bz2.compress(raw), bz2.compress(raw) + b"trailing"):
            with self.assertRaises(tool.InventoryError):
                self.measure(archive)

    def test_a_cut_stream_is_refused(self) -> None:
        compressed = bz2.compress(make_tar({"tool": b"a" * 5000}))
        with self.assertRaises(tool.InventoryError):
            self.measure(compressed[: len(compressed) // 2])


class CompareTests(unittest.TestCase):
    MEASURED = {"entries": 4, "expanded_bytes": 3500, "tar_stream_bytes": 10240, "kinds": {}}

    def test_pins_that_describe_the_archive_report_nothing(self) -> None:
        self.assertEqual(tool.compare(self.MEASURED, 4, 3500, 10240), [])
        self.assertEqual(tool.compare(self.MEASURED, 50, 3500, 20000), [])

    def test_each_wrong_pin_is_named(self) -> None:
        self.assertEqual(len(tool.compare(self.MEASURED, 3, 3500, 10240)), 1)
        self.assertEqual(len(tool.compare(self.MEASURED, 4, 3499, 10240)), 1)
        self.assertEqual(len(tool.compare(self.MEASURED, 4, 3501, 10240)), 1)
        self.assertEqual(len(tool.compare(self.MEASURED, 4, 3500, 10239)), 1)
        self.assertEqual(len(tool.compare(self.MEASURED, 1, 1, 1)), 3)


class DownloadTests(unittest.TestCase):
    def test_only_a_plain_https_github_address_is_fetched(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            target = Path(folder) / "archive"
            for url in (
                "http://github.com/k2-fsa/sherpa-onnx/a.tar.bz2",
                "https://example.invalid/a.tar.bz2",
                "https://github.com/k2-fsa/sherpa-onnx/a.tar.bz2?token=1",
                "https://github.com/k2-fsa/sherpa-onnx/a.tar.bz2#fragment",
            ):
                with self.assertRaises(tool.InventoryError, msg=url):
                    tool.download(url, 1, "0" * 64, target)
            self.assertFalse(target.exists())


if __name__ == "__main__":
    unittest.main()
