"""The online-video component review parses sums strictly and fails closed."""

from __future__ import annotations

import hashlib
from pathlib import Path
import stat
import tarfile
import tempfile
import unittest
from unittest import mock
import zipfile

import online_video_component_review as review
from p06_windows_candidate_smoke import CandidateRejected


DIGEST = "58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a"


class PhraseCheckTests(unittest.TestCase):
    def test_phrases_come_from_the_table_with_their_escapes_read(self) -> None:
        source = (
            'pub const YTDLP_PHRASES: [(SourceFetchReason, &[&str]); 2] = [\n'
            '    (SourceFetchReason::Drm, &["DRM protected"]),\n'
            '    (SourceFetchReason::BotCheck, &["you\\u{2019}re not a bot"]),\n'
            '];\n'
            'const OTHER: &str = "not a phrase";\n'
        )
        self.assertEqual(review.reviewed_phrases(source),
                         ["DRM protected", "you\u2019re not a bot"])

    def test_both_lists_are_read_and_comments_are_not_phrases(self) -> None:
        source = (
            'pub const SITE_PHRASES: [(SourceFetchReason, &[&str]); 1] = [\n'
            '    // From the "site" itself.\n'
            '    (SourceFetchReason::Drm, &["a site phrase"]),\n'
            '];\n'
        )
        self.assertEqual(review.reviewed_phrases(source, "SITE_PHRASES"), ["a site phrase"])

    def test_the_lists_are_held_to_their_rules(self) -> None:
        self.assertFalse(any(review.phrase_failures(["ours"], ["theirs"], "ours").values()))
        failures = review.phrase_failures(["gone", "both"], ["both", "here"], "here")
        self.assertEqual(failures["phrases"], ["gone", "both"])
        self.assertEqual(failures["in_both_lists"], ["both"])
        self.assertEqual(failures["site_phrases_in_source"], ["here"])

    def test_the_real_lists_hold_six_site_phrases(self) -> None:
        refusals = review.REFUSAL_SOURCE.read_text(encoding="utf-8")
        self.assertEqual(len(review.reviewed_phrases(refusals, "SITE_PHRASES")), 6)

    def test_origins_are_bounded_and_name_the_line(self) -> None:
        origins = review.fragment_origins({"a.py": "x\nnot a bot here\n"}, ("not a bot", "none"))
        self.assertEqual(origins, {"not a bot": ["a.py:2: not a bot here"], "none": []})

    def test_the_real_table_and_flags_are_read(self) -> None:
        phrases = review.reviewed_phrases(review.REFUSAL_SOURCE.read_text(encoding="utf-8"))
        flags = review.reviewed_flags(review.ACQUISITION_SOURCE.read_text(encoding="utf-8"))
        site = review.reviewed_phrases(review.REFUSAL_SOURCE.read_text(encoding="utf-8"),
                                       "SITE_PHRASES")
        self.assertIn("Sign in to confirm you\u2019re not a bot", site)
        self.assertIn("407 Proxy Authentication Required", site)
        self.assertIn("HTTP Error 403", phrases)
        for flag in ("--ignore-config", "--no-js-runtimes", "--match-filters", "--simulate"):
            self.assertIn(flag, flags)

    def test_the_probe_video_has_the_streams_the_selection_names(self) -> None:
        info = review.probe_info(8443, "/audio.m4a")
        self.assertEqual([item["format_id"] for item in info["formats"]], ["137", "140"])
        self.assertTrue(review.PROBE_BODY[4:8] == b"ftyp")
        self.assertTrue(all(item["url"].startswith("https://127.0.0.1:8443/")
                            for item in info["formats"]))

    def test_one_check_failing_does_not_hide_the_other(self) -> None:
        def failing(text: str) -> dict:
            raise CandidateRejected(text)

        with self.assertRaisesRegex(CandidateRejected, "first; second"):
            review.both_checks(lambda: failing("first"), lambda: failing("second"))
        self.assertEqual(review.both_checks(lambda: {"a": 1}, lambda: {"b": 2}),
                         ({"a": 1}, {"b": 2}))

    def test_a_missing_phrase_is_named(self) -> None:
        self.assertEqual(review.missing_from(["a", "b"], "only a here"), ["b"])

    def test_the_source_archive_is_read_in_place(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            folder = Path(temporary)
            (folder / "extractor.py").write_text("Private video", encoding="utf-8")
            (folder / "ignored.bin").write_text("ignored", encoding="utf-8")
            archive = folder / "source.tar.gz"
            with tarfile.open(archive, "w:gz") as tar:
                tar.add(folder / "extractor.py", arcname="yt-dlp/yt_dlp/extractor.py")
                tar.add(folder / "ignored.bin", arcname="yt-dlp/ignored.bin")
            files = review.source_archive_files(archive)
        self.assertEqual(files, {"yt-dlp/yt_dlp/extractor.py": "Private video"})


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
