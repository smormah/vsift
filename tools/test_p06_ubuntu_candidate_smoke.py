"""Archive guardrails for the opt-in Ubuntu candidate experiment."""

from __future__ import annotations

import hashlib
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

from p06_ubuntu_candidate_smoke import CandidateRejected, extract_candidate


def regular(name: str, content: bytes) -> tuple[tarfile.TarInfo, io.BytesIO]:
    member = tarfile.TarInfo(name)
    member.size = len(content)
    member.mode = 0o755
    return member, io.BytesIO(content)


def link(name: str, target: str) -> tuple[tarfile.TarInfo, None]:
    member = tarfile.TarInfo(name)
    member.type = tarfile.SYMTYPE
    member.linkname = target
    return member, None


class CandidateExtractionTests(unittest.TestCase):
    def make_archive(self, path: Path, members: list[tuple[tarfile.TarInfo, io.BytesIO | None]]) -> None:
        with tarfile.open(path, "w:gz") as archive:
            for member, content in members:
                archive.addfile(member, content)

    def test_selected_file_and_reviewed_alias_become_regular_files(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "candidate.tar.gz"
            self.make_archive(archive, [
                regular("release/libname.so.1", b"library"),
                link("release/libname.so", "libname.so.1"),
            ])
            expected = {"libname.so.1": (7, hashlib.sha256(b"library").hexdigest())}
            extract_candidate(archive, root / "out", expected,
                              {"libname.so": "libname.so.1"})
            alias = root / "out/libname.so"
            self.assertTrue(alias.is_file())
            self.assertFalse(alias.is_symlink())
            self.assertEqual(alias.read_bytes(), b"library")

    def test_rejects_traversal_duplicate_and_unreviewed_link(self) -> None:
        cases = [
            [regular("release/../escape", b"bad")],
            [regular("release/file", b"one"), regular("release/file", b"two")],
            [link("release/file", "../../outside")],
        ]
        for members in cases:
            with self.subTest(members=[member.name for member, _ in members]):
                with tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    archive = root / "candidate.tar.gz"
                    self.make_archive(archive, members)
                    with self.assertRaises(CandidateRejected):
                        extract_candidate(archive, root / "out", {}, {})

    def test_rejects_changed_file_and_missing_alias(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "candidate.tar.gz"
            self.make_archive(archive, [regular("release/file", b"changed")])
            expected = {"file": (7, hashlib.sha256(b"trusted").hexdigest())}
            with self.assertRaises(CandidateRejected):
                extract_candidate(archive, root / "out", expected, {})

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "candidate.tar.gz"
            self.make_archive(archive, [regular("release/file", b"trusted")])
            expected = {"file": (7, hashlib.sha256(b"trusted").hexdigest())}
            with self.assertRaises(CandidateRejected):
                extract_candidate(archive, root / "out", expected,
                                  {"alias": "file"})

    def test_does_not_extract_unselected_executable(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "candidate.tar.gz"
            self.make_archive(archive, [
                regular("release/selected", b"trusted"),
                regular("release/unwanted-tool", b"untrusted"),
            ])
            expected = {"selected": (7, hashlib.sha256(b"trusted").hexdigest())}
            extract_candidate(archive, root / "out", expected, {})
            self.assertFalse((root / "out/unwanted-tool").exists())


if __name__ == "__main__":
    unittest.main()
