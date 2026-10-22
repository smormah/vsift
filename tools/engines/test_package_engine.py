"""The engine packager makes the same bytes from the same tree and refuses what
the managed installer would refuse."""

from __future__ import annotations

import gzip
import hashlib
import json
import os
from pathlib import Path
import stat
import tarfile
import tempfile
import unittest

import package_engine


def make_tree(base: Path) -> Path:
    tree = base / "tree"
    (tree / "bin").mkdir(parents=True)
    (tree / "bin" / "tool").write_bytes(b"\xcf\xfa\xed\xfe reviewed tool")
    os.chmod(tree / "bin" / "tool", 0o755)
    (tree / "LICENSE").write_bytes(b"licence text")
    os.chmod(tree / "LICENSE", 0o600)
    return tree


class PackageTests(unittest.TestCase):
    def test_the_same_tree_gives_the_same_bytes_and_an_exact_inventory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            tree = make_tree(base)
            first = package_engine.package(
                tree, "tool-1.0-r1-macos-arm64", 1_760_000_000,
                base / "one.tar.gz", base / "one.json",
            )
            # The second tree differs only in what the archive must not carry:
            # its owner's file times and the order the folder lists them in.
            os.utime(tree / "LICENSE", (1, 1))
            second = package_engine.package(
                tree, "tool-1.0-r1-macos-arm64", 1_760_000_000,
                base / "two.tar.gz", base / "two.json",
            )
            self.assertEqual((base / "one.tar.gz").read_bytes(), (base / "two.tar.gz").read_bytes())
            self.assertEqual(first["sha256"], second["sha256"])
            data = (base / "one.tar.gz").read_bytes()
            self.assertEqual(first["sha256"], hashlib.sha256(data).hexdigest())
            self.assertEqual(first["bytes"], len(data))
            self.assertEqual(first["entries"], 4)
            self.assertEqual(first["expanded_bytes"], len(b"\xcf\xfa\xed\xfe reviewed tool") + len(b"licence text"))
            self.assertEqual(
                [(entry["path"], entry["executable"]) for entry in first["files"]],
                # Windows has no executable bit; the packager runs on macOS.
                [("tool-1.0-r1-macos-arm64/LICENSE", False),
                 ("tool-1.0-r1-macos-arm64/bin/tool", os.name != "nt")],
            )
            self.assertEqual(json.loads((base / "one.json").read_text(encoding="utf-8")), first)
            with gzip.open(base / "one.tar.gz") as stream:
                self.assertEqual(len(stream.read()), first["tar_stream_bytes"])
            with tarfile.open(base / "one.tar.gz") as archive:
                members = archive.getmembers()
            self.assertEqual(
                [member.name for member in members],
                [
                    "tool-1.0-r1-macos-arm64",
                    "tool-1.0-r1-macos-arm64/LICENSE",
                    "tool-1.0-r1-macos-arm64/bin",
                    "tool-1.0-r1-macos-arm64/bin/tool",
                ],
            )
            for member in members:
                self.assertEqual((member.uid, member.gid, member.uname, member.gname), (0, 0, "", ""))
                self.assertEqual(member.mtime, 1_760_000_000)
                self.assertTrue(member.isfile() or member.isdir())
            modes = {member.name: stat.S_IMODE(member.mode) for member in members}
            self.assertEqual(modes["tool-1.0-r1-macos-arm64/bin/tool"], 0o644 if os.name == "nt" else 0o755)
            self.assertEqual(modes["tool-1.0-r1-macos-arm64/LICENSE"], 0o644)

    @unittest.skipIf(os.name == "nt", "creating a link needs a privilege on Windows")
    def test_a_link_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            tree = make_tree(base)
            (tree / "bin" / "alias").symlink_to("tool")
            with self.assertRaises(package_engine.PackageRefused):
                package_engine.package(tree, "root", 0, base / "a.tar.gz", base / "a.json")

    def test_unsafe_names_and_an_empty_tree_are_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            tree = make_tree(base)
            (tree / "has space").write_bytes(b"x")
            with self.assertRaises(package_engine.PackageRefused):
                package_engine.package(tree, "root", 0, base / "a.tar.gz", base / "a.json")
            empty = base / "empty"
            empty.mkdir()
            with self.assertRaises(package_engine.PackageRefused):
                package_engine.package(empty, "root", 0, base / "b.tar.gz", base / "b.json")
            clean = make_tree(base / "clean")
            for root in ["", "..", "a/b", "with space"]:
                with self.assertRaises(package_engine.PackageRefused):
                    package_engine.package(clean, root, 0, base / "c.tar.gz", base / "c.json")

    def test_a_negative_time_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            with self.assertRaises(package_engine.PackageRefused):
                package_engine.package(make_tree(base), "root", -1, base / "a.tar.gz", base / "a.json")


if __name__ == "__main__":
    unittest.main()
