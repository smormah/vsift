"""The online-video component review parses sums strictly and fails closed."""

from __future__ import annotations

import hashlib
from pathlib import Path
import stat
import tempfile
import unittest
from unittest import mock
import zipfile

import online_video_component_review as review
from p06_windows_candidate_smoke import CandidateRejected


DIGEST = "58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a"


class SumsTests(unittest.TestCase):
    def test_reads_names_and_digests(self) -> None:
        sums = review.parse_sums(f"{DIGEST}  yt-dlp_linux\n{'0' * 64}  *yt-dlp.exe\n\n")
        self.assertEqual(sums, {"yt-dlp_linux": DIGEST, "yt-dlp.exe": "0" * 64})

    def test_refuses_malformed_duplicate_and_path_lines(self) -> None:
        for text in (
            f"{DIGEST} yt-dlp_linux",
            f"{DIGEST.upper()}  yt-dlp_linux",
            f"{DIGEST[:-1]}  yt-dlp_linux",
            f"{DIGEST}  ",
            f"{DIGEST}  dir/yt-dlp_linux",
            f"{DIGEST}  yt-dlp_linux\n{'0' * 64}  yt-dlp_linux",
        ):
            with self.subTest(text=text), self.assertRaises(CandidateRejected):
                review.parse_sums(text)

    def test_the_pins_are_the_ones_the_catalogue_uses(self) -> None:
        self.assertEqual(review.YT_DLP_EXECUTABLES[review.YT_DLP_MANAGED], (40_446_224, DIGEST))
        self.assertEqual(review.DENO_ZIP[1:], (
            41_596_794, "c6527f24f4b16031d3ae4fa9f658d5f11534c8d84ce7dc8502420280919c3490"))


class GitBlobTests(unittest.TestCase):
    def test_matches_git_hash_object(self) -> None:
        # `printf 'hello\n' | git hash-object --stdin`
        self.assertEqual(review.git_blob_identity(b"hello\n"),
                         "ce013625030ba8dba906f756967f9e9ca394464a")


def write_zip(path: Path, entries: list[tuple[str, bytes, int]]) -> None:
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, content, mode in entries:
            info = zipfile.ZipInfo(name)
            info.external_attr = mode << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, content)


class ZipInventoryTests(unittest.TestCase):
    def test_lists_the_one_entry_with_its_digest_and_mode(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "runtime.zip"
            write_zip(path, [("deno", b"runtime bytes", stat.S_IFREG | 0o755)])
            result = review.inventory_one_entry_zip(path)
        self.assertEqual(result["entries"], 1)
        member = result["members"][0]
        self.assertEqual(member["path"], "deno")
        self.assertEqual(member["kind"], "file")
        self.assertEqual(member["bytes"], len(b"runtime bytes"))
        self.assertEqual(member["mode"], oct(stat.S_IFREG | 0o755))
        self.assertEqual(member["sha256"], hashlib.sha256(b"runtime bytes").hexdigest())

    def test_a_link_is_listed_as_other_and_never_read(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "runtime.zip"
            write_zip(path, [("deno", b"/etc/passwd", stat.S_IFLNK | 0o777)])
            result = review.inventory_one_entry_zip(path)
        self.assertEqual(result["members"][0]["kind"], "other")

    def test_refuses_a_traversing_path_and_too_many_entries(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "runtime.zip"
            write_zip(path, [("../deno", b"x", stat.S_IFREG | 0o755)])
            with self.assertRaises(CandidateRejected):
                review.inventory_one_entry_zip(path)
            crowded = Path(temporary) / "crowded.zip"
            write_zip(crowded, [(f"file{index}", b"x", stat.S_IFREG | 0o644)
                                for index in range(review.DENO_MAX_ENTRIES + 1)])
            with self.assertRaises(CandidateRejected):
                review.inventory_one_entry_zip(crowded)


class RunnerOnlyTests(unittest.TestCase):
    def test_refuses_to_fetch_outside_an_actions_runner(self) -> None:
        with mock.patch.dict("os.environ", {}, clear=True), \
                self.assertRaises(CandidateRejected):
            review.main([])


if __name__ == "__main__":
    unittest.main()
