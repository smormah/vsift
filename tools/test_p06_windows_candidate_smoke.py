"""Negative tests for the opt-in candidate-only archive qualification tool."""

import hashlib
import io
from pathlib import Path
import stat
import tempfile
import unittest
import urllib.request
import zipfile

from p06_windows_candidate_smoke import (
    CandidateRejected,
    HttpsRedirectsOnly,
    selected_members,
    validate_archive_path,
    verified_extract,
)


def archive_with_entries(entries: list[tuple[str, bytes, int]]) -> bytes:
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        for name, payload, file_type in entries:
            info = zipfile.ZipInfo(name)
            info.create_system = 3
            info.external_attr = (file_type | 0o644) << 16
            archive.writestr(info, payload)
    return buffer.getvalue()


class CandidateArchiveTests(unittest.TestCase):
    def test_rejects_ambiguous_paths(self):
        for name in ("../escape", "/absolute", "C:/drive", "a\\b", "a/./b", "a/../b"):
            with self.subTest(name=name), self.assertRaises(CandidateRejected):
                validate_archive_path(name)

    def test_rejects_duplicate_casefolded_entry(self):
        data = archive_with_entries([
            ("Release/tool.exe", b"good", stat.S_IFREG),
            ("release/TOOL.exe", b"evil", stat.S_IFREG),
        ])
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            with self.assertRaisesRegex(CandidateRejected, "duplicate"):
                selected_members(archive, "none", {"tool.exe": (4, "unused")})

    def test_rejects_symlink_even_if_not_selected(self):
        data = archive_with_entries([
            ("Release/tool.exe", b"good", stat.S_IFREG),
            ("Release/link", b"target", stat.S_IFLNK),
        ])
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            with self.assertRaisesRegex(CandidateRejected, "special"):
                selected_members(archive, "none", {"tool.exe": (4, "unused")})

    def test_rejects_missing_and_oversized_selected_file(self):
        data = archive_with_entries([("Release/tool.exe", b"good", stat.S_IFREG)])
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            with self.assertRaisesRegex(CandidateRejected, "missing"):
                selected_members(archive, "none", {"missing.exe": (4, "unused")})
            with self.assertRaisesRegex(CandidateRejected, "size"):
                selected_members(archive, "none", {"tool.exe": (3, "unused")})

    def test_extracts_only_reviewed_hash_and_rejects_mismatch(self):
        data = archive_with_entries([
            ("Release/tool.exe", b"good", stat.S_IFREG),
            ("Release/unrelated.exe", b"unrelated", stat.S_IFREG),
        ])
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive_path = root / "candidate.zip"
            archive_path.write_bytes(data)
            expected = {"tool.exe": (4, hashlib.sha256(b"good").hexdigest())}
            verified_extract(archive_path, root / "accepted", "none", expected)
            self.assertEqual((root / "accepted/tool.exe").read_bytes(), b"good")
            self.assertFalse((root / "accepted/unrelated.exe").exists())
            with self.assertRaisesRegex(CandidateRejected, "integrity"):
                verified_extract(archive_path, root / "rejected", "none",
                                 {"tool.exe": (4, hashlib.sha256(b"evil").hexdigest())})

    def test_https_redirect_rejects_downgrade_or_credentials(self):
        handler = HttpsRedirectsOnly()
        request = urllib.request.Request("https://example.org")
        for url in ("http://example.org/file", "https://user:pass@example.org/file"):
            with self.subTest(url=url), self.assertRaises(CandidateRejected):
                handler.redirect_request(request, None, 302, "Found", {}, url)


if __name__ == "__main__":
    unittest.main()
