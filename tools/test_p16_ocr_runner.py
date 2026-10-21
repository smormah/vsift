"""Guardrails for the OCR runner's hosted checks: staging, the protocol, parity and the end-to-end tally."""

from __future__ import annotations

import io
import json
import tarfile
import unittest
import zipfile

import p16_ocr_runner as runner
from p16_ocr_evaluation import Line, Recognition
from p16_ocr_runner import RunnerCheckError


def tar_gz(files: dict[str, bytes], links: dict[str, str] | None = None) -> bytes:
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
        for name, content in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(content)
            archive.addfile(info, io.BytesIO(content))
        for name, target in (links or {}).items():
            info = tarfile.TarInfo(name)
            info.type = tarfile.SYMTYPE
            info.linkname = target
            archive.addfile(info)
    return buffer.getvalue()


def zipped(files: dict[str, bytes]) -> bytes:
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        for name, content in files.items():
            archive.writestr(name, content)
    return buffer.getvalue()


def line(text: str, box: tuple[int, int, int, int]) -> Line:
    return Line(text, box, 0.9)


class ArchiveTests(unittest.TestCase):
    def test_a_member_is_read_by_its_exact_name_and_a_link_is_refused(self) -> None:
        root = "onnxruntime-linux-x64-1.31.0"
        archive = tar_gz({f"{root}/lib/libonnxruntime.so.1.31.0": b"library"},
                         {f"{root}/lib/libonnxruntime.so": "libonnxruntime.so.1.31.0"})
        asset = f"{root}.tgz"
        self.assertEqual(runner.archive_member(archive, asset, f"{root}/lib/libonnxruntime.so.1.31.0"), b"library")
        with self.assertRaises(RunnerCheckError):
            runner.archive_member(archive, asset, f"{root}/lib/libonnxruntime.so")
        with self.assertRaises(RunnerCheckError):
            runner.archive_member(archive, asset, f"{root}/lib/missing")
        with self.assertRaises(RunnerCheckError):
            runner.archive_member(archive, asset, f"{root}/lib/libonnxruntime.so.1.31.0", limit=3)
        self.assertEqual(runner.archive_lib_listing(archive, asset),
                         [(f"{root}/lib/libonnxruntime.so", 0, "link"),
                          (f"{root}/lib/libonnxruntime.so.1.31.0", 7, "file")])

    def test_a_member_named_with_a_leading_dot_folder_is_found_as_macos_names_it(self) -> None:
        archive = tar_gz({"./r/lib/libonnxruntime.1.31.0.dylib": b"dylib"})
        self.assertEqual(runner.archive_member(archive, "r.tgz", "r/lib/libonnxruntime.1.31.0.dylib"), b"dylib")

    def test_a_zip_member_is_read_and_a_folder_is_refused(self) -> None:
        archive = zipped({"r/lib/onnxruntime.dll": b"dll", "r/lib/": b""})
        self.assertEqual(runner.archive_member(archive, "r.zip", "r/lib/onnxruntime.dll"), b"dll")
        with self.assertRaises(RunnerCheckError):
            runner.archive_member(archive, "r.zip", "r/lib/")

    def test_a_download_must_match_its_size_and_digest(self) -> None:
        content = b"model bytes"
        digest = runner.sha256_bytes(content)
        self.assertEqual(runner.checked_download("https://h/m", len(content), digest, 100, lambda _u, _l: content),
                         content)
        for size, wanted in ((len(content) + 1, digest), (len(content), "0" * 64)):
            with self.assertRaises(RunnerCheckError):
                runner.checked_download("https://h/m", size, wanted, 100, lambda _u, _l: content)

    def test_the_runtime_root_is_the_asset_without_its_extension(self) -> None:
        self.assertEqual(runner.runtime_root("onnxruntime-win-x64-1.31.0.zip"), "onnxruntime-win-x64-1.31.0")
        self.assertEqual(runner.runtime_root("onnxruntime-osx-arm64-1.31.0.tgz"), "onnxruntime-osx-arm64-1.31.0")


class ProtocolTests(unittest.TestCase):
    def describe(self, **changes: object) -> str:
        record = {"record": "describe", "protocol": 1, "runner_version": "0.1.0", "runtime_version": "1.31.0",
                  "profile": "ppocrv5-latin-v1", "detector_sha256": runner.MODELS[runner.DETECTOR][2],
                  "recognizer_sha256": runner.MODELS[runner.RECOGNIZER][2], **changes}
        return json.dumps(record) + "\n"

    def test_describe_is_held_to_the_pins(self) -> None:
        self.assertEqual(runner.check_description(0, self.describe())["runtime_version"], "1.31.0")
        for changes in ({"runtime_version": "1.30.0"}, {"detector_sha256": "0" * 64}, {"protocol": 2}):
            with self.assertRaises(RunnerCheckError):
                runner.check_description(0, self.describe(**changes))
        with self.assertRaises(RunnerCheckError):
            runner.check_description(3, '{"record":"error","image":null,"code":"runtime_unavailable"}\n')

    def test_a_read_gives_each_image_its_lines_or_none(self) -> None:
        stdout = "\n".join([
            '{"record":"runner","protocol":1,"runner_version":"0.1.0"}',
            '{"record":"line","image":0,"x":1,"y":2,"width":30,"height":10,"text":"Total","text_cut":false,"confidence":0.98}',
            '{"record":"image","image":0,"width":64,"height":32,"lines":1,"truncated":false}',
            '{"record":"error","image":1,"code":"image_invalid"}',
        ]) + "\n"
        readings = runner.parse_read(stdout, 2)
        self.assertEqual(readings[0], [Line("Total", (1, 2, 30, 10), 0.98)])
        self.assertIsNone(readings[1])
        with self.assertRaises(RunnerCheckError):
            runner.parse_read(stdout, 3)
        with self.assertRaises(RunnerCheckError):
            runner.parse_read(stdout.split("\n", 1)[1], 2)
        misnumbered = stdout.replace('"image":1,"code"', '"image":5,"code"')
        with self.assertRaises(RunnerCheckError):
            runner.parse_read(misnumbered, 2)


class ParityTests(unittest.TestCase):
    def test_lines_pair_by_overlap_and_agree_by_text(self) -> None:
        reference = [Recognition("f.png", [line("Invoice 4407", (10, 10, 100, 20)),
                                           line("Total", (10, 50, 60, 20)),
                                           line("Missed", (300, 300, 50, 20))], 0.1)]
        own = [Recognition("f.png", [line("Invoice 4407", (11, 9, 100, 21)),
                                     line("TotaI", (10, 51, 60, 20)),
                                     line("Extra", (600, 600, 40, 20))], 0.1)]
        result = runner.parity(reference, own)
        self.assertEqual((result["paired"], result["text_equal"], result["unpaired_runner_lines"]), (2, 1, 1))
        self.assertEqual(result["text_agreement"], round(1 / 3, 4))
        self.assertEqual(result["max_box_offset_px"], 1)

    def test_a_missing_frame_is_an_error_not_a_silent_skip(self) -> None:
        with self.assertRaises(RunnerCheckError):
            runner.parity([Recognition("a.png", [], 0.1)], [Recognition("b.png", [], 0.1)])

    def test_iou_of_disjoint_and_equal_boxes(self) -> None:
        self.assertEqual(runner.iou((0, 0, 10, 10), (20, 20, 10, 10)), 0.0)
        self.assertEqual(runner.iou((0, 0, 10, 10), (0, 0, 10, 10)), 1.0)

    def test_platforms_agree_line_by_line(self) -> None:
        linux = [Recognition("f.png", [line("A", (0, 0, 1, 1)), line("B", (0, 2, 1, 1))], 0.1)]
        other = [Recognition("f.png", [line("A", (0, 0, 1, 1))], 0.1)]
        self.assertEqual(runner.agreement(linux, other), {"lines": 2, "equal": 1, "agreement": 0.5})


class DiagnoseTests(unittest.TestCase):
    def test_differences_are_named_by_kind_never_by_text(self) -> None:
        self.assertEqual(runner.text_difference("Total 4407", "Total 4407"), "equal")
        self.assertEqual(runner.text_difference("Total 4407", "total  4407"), "case_or_space")
        self.assertEqual(runner.text_difference("Invoice total", "nvoice total"), "shorter")
        self.assertEqual(runner.text_difference("Invoice", "Invoice:"), "longer")
        self.assertEqual(runner.text_difference("Invoice 4407", "Invoice 4401"), "near")
        self.assertEqual(runner.text_difference("Invoice 4407", "Settings"), "far")

    def test_a_frame_tally_counts_pairs_misses_and_extras(self) -> None:
        reference = [Recognition("f.png", [Line("Invoice 4407", (10, 10, 100, 20), 0.99),
                                           Line("Gone", (300, 300, 50, 20), 0.6)], 0.1)]
        own = [Recognition("f.png", [Line("Invoice 4401", (10, 10, 90, 20), 0.9),
                                     Line("Extra", (600, 600, 40, 20), 0.9)], 0.1)]
        tally = runner.diagnose(reference, own)
        self.assertEqual(tally["outcomes"], {"near": 1, "unpaired_missed": 1, "runner_only": 1})
        self.assertEqual(tally["width_ratio"]["differing"], {"0.8-0.95": 1})
        self.assertEqual(tally["reference_confidence"]["unpaired_missed/low"], 1)


class EndToEndTests(unittest.TestCase):
    def test_a_number_counts_as_found_only_within_a_screen_that_showed_it(self) -> None:
        truth = {"hold_us": 3_000_000, "screens": [
            {"lines": [{"text": "Invoice 4407"}]},
            {"lines": [{"text": "Total 15.2%"}]},
        ]}
        hits = {"4407": [{"frame": {"time_us": 1_000_000}}],
                "15.2%": [{"frame": {"time_us": 1_000_000}}]}
        self.assertEqual(runner.synthetic_found(truth, hits), {"numbers": 2, "found": 1, "found_share": 0.5})


if __name__ == "__main__":
    unittest.main()
