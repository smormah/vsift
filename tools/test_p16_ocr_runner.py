"""Guardrails for the OCR runner's hosted checks: staging, the protocol, parity and the end-to-end tally."""

from __future__ import annotations

import base64
import gzip
import io
import json
import stat
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

import p16_ocr_runner as runner
from p16_ocr_evaluation import Line, Recognition
from p16_ocr_runner import RunnerCheckError


def zipped(files: dict[str, bytes], links: dict[str, str] | None = None) -> bytes:
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        for name, content in files.items():
            archive.writestr(name, content)
        for name, target in (links or {}).items():
            info = zipfile.ZipInfo(name)
            info.external_attr = (stat.S_IFLNK | 0o777) << 16
            archive.writestr(info, target)
    return buffer.getvalue()


def line(text: str, box: tuple[int, int, int, int]) -> Line:
    return Line(text, box, 0.9)


class ArchiveTests(unittest.TestCase):
    def test_a_member_is_read_by_its_exact_name_and_a_link_is_refused(self) -> None:
        archive = zipped({"runtimes/linux-x64/native/libonnxruntime.so": b"library", "LICENSE": b"MIT", "dir/": b""},
                         {"runtimes/linux-x64/native/libonnxruntime.so.1": "libonnxruntime.so"})
        self.assertEqual(runner.archive_member(archive, "runtimes/linux-x64/native/libonnxruntime.so"), b"library")
        for name in ("runtimes/linux-x64/native/libonnxruntime.so.1", "dir/", "runtimes/linux-x64/native/missing",
                     "runtimes/../LICENSE"):
            with self.assertRaises(RunnerCheckError, msg=name):
                runner.archive_member(archive, name)
        with self.assertRaises(RunnerCheckError):
            runner.archive_member(archive, "runtimes/linux-x64/native/libonnxruntime.so", limit=3)
        self.assertEqual(runner.package_listing(archive, "linux-x64"),
                         [("runtimes/linux-x64/native/libonnxruntime.so", 7, "file"),
                          ("runtimes/linux-x64/native/libonnxruntime.so.1", 17, "link")])
        self.assertEqual(runner.package_listing(archive, "win-x64"), [])

    def test_a_name_the_package_repeats_is_refused(self) -> None:
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w") as handle:
            handle.writestr("LICENSE", b"first")
            with self.assertWarns(UserWarning):
                handle.writestr("LICENSE", b"second")
        with self.assertRaises(RunnerCheckError):
            runner.archive_member(buffer.getvalue(), "LICENSE")

    def test_a_member_is_held_to_its_pinned_size_and_digest(self) -> None:
        archive = zipped({"LICENSE": b"MIT licence"})
        digest = runner.sha256_bytes(b"MIT licence")
        self.assertEqual(runner.checked_member(archive, "LICENSE", 11, digest), b"MIT licence")
        for size, wanted in ((10, digest), (12, digest), (11, "0" * 64)):
            with self.assertRaises(RunnerCheckError):
                runner.checked_member(archive, "LICENSE", size, wanted)

    def test_a_download_must_match_its_size_and_digest(self) -> None:
        content = b"model bytes"
        digest = runner.sha256_bytes(content)
        self.assertEqual(runner.checked_download("https://h/m", len(content), digest, 100, lambda _u, _l: content),
                         content)
        for size, wanted in ((len(content) + 1, digest), (len(content), "0" * 64)):
            with self.assertRaises(RunnerCheckError):
                runner.checked_download("https://h/m", size, wanted, 100, lambda _u, _l: content)

    def test_a_download_with_a_pinned_sha512_must_match_it_too(self) -> None:
        content = b"package"
        digest = runner.sha256_bytes(content)
        self.assertEqual(runner.checked_download("https://h/p", len(content), digest, 100, lambda _u, _l: content,
                                                 sha512=runner.sha512_bytes(content)), content)
        with self.assertRaises(RunnerCheckError):
            runner.checked_download("https://h/p", len(content), digest, 100, lambda _u, _l: content, sha512="0" * 128)


class ArchiveLimitTests(unittest.TestCase):
    """The 128 MiB cap holds for every source; one pinned package is allowed its own exact size."""

    def test_an_ordinary_source_keeps_the_general_cap(self) -> None:
        self.assertEqual(runner.archive_limit({"asset": "a", "bytes": 11_408_003}), runner.MAX_RUNTIME_ARCHIVE_BYTES)
        self.assertEqual(runner.archive_limit({"asset": "a", "bytes": runner.MAX_RUNTIME_ARCHIVE_BYTES}),
                         runner.MAX_RUNTIME_ARCHIVE_BYTES)

    def test_a_source_over_the_cap_is_refused_unless_its_own_allowance_is_its_exact_size(self) -> None:
        over = runner.MAX_RUNTIME_ARCHIVE_BYTES + 1
        with self.assertRaises(RunnerCheckError):
            runner.archive_limit({"asset": "a", "bytes": over})
        for allowance in (over - 1, over + 1, None):
            with self.assertRaises(RunnerCheckError):
                runner.archive_limit({"asset": "a", "bytes": over, "max_bytes": allowance})
        self.assertEqual(runner.archive_limit({"asset": "a", "bytes": over, "max_bytes": over}), over)

    def test_the_pinned_package_is_the_only_source_over_the_cap_and_its_allowance_is_its_size(self) -> None:
        package = runner.RUNTIME_PACKAGE
        self.assertGreater(package["bytes"], runner.MAX_RUNTIME_ARCHIVE_BYTES)
        self.assertEqual(runner.archive_limit(package), package["bytes"])
        self.assertEqual(package["max_bytes"], package["bytes"])
        self.assertLessEqual(runner.MAX_MODEL_BYTES, runner.MAX_RUNTIME_ARCHIVE_BYTES)

    def test_the_package_is_asked_for_with_its_own_allowance_and_refused_if_it_is_larger(self) -> None:
        wanted = runner.RUNTIME_PACKAGE
        asked: list[int] = []

        def get(_url: str, limit: int) -> bytes:
            asked.append(limit)
            return b"x" * (wanted["bytes"] + 1)

        with self.assertRaises(RunnerCheckError):
            runner.fetch_runtime_package(get)
        self.assertEqual(asked, [wanted["bytes"]])


class SourceTests(unittest.TestCase):
    def test_only_nuget_org_is_fetched_by_the_package_getter(self) -> None:
        self.assertTrue(runner.is_nuget_host("api.nuget.org"))
        self.assertTrue(runner.is_nuget_host("nuget.org"))
        for host in ("evil-nuget.org", "nuget.org.evil.example", "github.com", "", None):
            self.assertFalse(runner.is_nuget_host(host), host)
        for url in ("https://github.com/microsoft/onnxruntime/x", "http://api.nuget.org/x", "https://nuget.org.evil.example/x"):
            with self.assertRaises(RunnerCheckError):
                runner.nuget_get(url, 10)

    def test_the_pinned_addresses_are_nuget_org_and_the_pin_is_complete(self) -> None:
        package = runner.RUNTIME_PACKAGE
        self.assertTrue(package["url"].startswith("https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/"))
        self.assertTrue(package["registration"].startswith("https://api.nuget.org/v3/registration"))
        self.assertTrue(package["url"].endswith(package["asset"]))
        self.assertRegex(package["sha256"], r"^[0-9a-f]{64}$")
        self.assertRegex(package["sha512"], r"^[0-9a-f]{128}$")
        self.assertEqual(sorted(runner.RUNTIMES), ["linux-x64", "osx-arm64", "win-x64"])
        for platform, pin in runner.RUNTIMES.items():
            self.assertTrue(pin["member"].startswith(f"runtimes/{platform}/native/"), platform)
            self.assertRegex(pin["sha256"], r"^[0-9a-f]{64}$")
        self.assertEqual(sorted(runner.RUNTIME_LICENCES), ["LICENSE", "ThirdPartyNotices.txt"])

    def test_the_published_sha512_is_read_from_the_catalogue_entry(self) -> None:
        digest = bytes(range(64))
        entry = {"packageHashAlgorithm": "SHA512", "packageHash": base64.b64encode(digest).decode()}
        leaf = {"catalogEntry": "https://api.nuget.org/v3/catalog0/data/x.json"}

        def get(url: str, _limit: int) -> bytes:
            return gzip.compress(json.dumps(entry if "catalog0" in url else leaf).encode())

        self.assertEqual(runner.published_sha512(get), digest.hex())
        entry["packageHashAlgorithm"] = "SHA256"
        with self.assertRaises(RunnerCheckError):
            runner.published_sha512(get)

    def fake_package(self) -> tuple[bytes, dict]:
        files = {"runtimes/linux-x64/native/libonnxruntime.so": b"linux library", "LICENSE": b"MIT",
                 "ThirdPartyNotices.txt": b"notices", "README.md": b"readme"}
        content = zipped(files)
        return content, {"asset": "p.nupkg", "url": "https://api.nuget.org/p.nupkg", "registration": "https://api.nuget.org/r.json",
                         "bytes": len(content), "max_bytes": len(content), "sha256": runner.sha256_bytes(content),
                         "sha512": runner.sha512_bytes(content)}

    def test_staging_writes_the_pinned_members_and_the_inventory(self) -> None:
        content, package = self.fake_package()
        runtimes = {"linux-x64": {"member": "runtimes/linux-x64/native/libonnxruntime.so", "bytes": 13,
                                  "sha256": runner.sha256_bytes(b"linux library"), "library": "libonnxruntime.so.1.31.0"}}
        licences = {"LICENSE": (3, runner.sha256_bytes(b"MIT")), "ThirdPartyNotices.txt": (7, runner.sha256_bytes(b"notices"))}
        models = {"m.onnx": ("https://models/m.onnx", 5, runner.sha256_bytes(b"model"))}

        def get(url: str, _limit: int) -> bytes:
            return content if url.endswith("p.nupkg") else b"model"

        with tempfile.TemporaryDirectory() as folder, \
                mock.patch.object(runner, "RUNTIME_PACKAGE", package), mock.patch.object(runner, "RUNTIMES", runtimes), \
                mock.patch.object(runner, "RUNTIME_LICENCES", licences), mock.patch.object(runner, "MODELS", models):
            base = Path(folder)
            (base / "runner").write_bytes(b"runner")
            inventory = runner.stage(base / "out", base / "runner", "linux-x64", get)
            self.assertEqual((base / "out/onnxruntime/libonnxruntime.so.1.31.0").read_bytes(), b"linux library")
            self.assertEqual((base / "out/LICENSES/onnxruntime-LICENSE").read_bytes(), b"MIT")
            paths = [entry["path"] for entry in inventory["files"]]
            self.assertIn("models/m.onnx", paths)
            self.assertEqual(inventory["runtime_native_listing"],
                             [("runtimes/linux-x64/native/libonnxruntime.so", 13, "file")])
            runtimes["linux-x64"]["sha256"] = "0" * 64
            with self.assertRaises(RunnerCheckError) as raised:
                runner.stage(base / "second", base / "runner", "linux-x64", get)
            self.assertIn("native folder holds", str(raised.exception))

    def test_provenance_reports_every_difference_after_measuring(self) -> None:
        content, package = self.fake_package()
        entry = {"packageHashAlgorithm": "SHA512", "packageHash": base64.b64encode(bytes.fromhex(package["sha512"])).decode()}

        def get(url: str, _limit: int) -> bytes:
            if url.endswith("p.nupkg"):
                return content
            return json.dumps(entry if "catalog0" in url else {"catalogEntry": "https://api.nuget.org/catalog0/x"}).encode()

        with tempfile.TemporaryDirectory() as folder, mock.patch.object(runner, "RUNTIME_PACKAGE", package):
            report = runner.provenance(Path(folder), get)
            self.assertEqual(report["differs"], [])
            self.assertEqual((Path(folder) / "p.nupkg").read_bytes(), content)
            self.assertEqual({member["name"] for member in report["members"]},
                             {"runtimes/linux-x64/native/libonnxruntime.so", "LICENSE", "ThirdPartyNotices.txt", "README.md"})
            entry["packageHash"] = base64.b64encode(bytes(64)).decode()
            package["sha256"] = "0" * 64
            report = runner.provenance(Path(folder), get)
            self.assertEqual(report["differs"], ["sha256", "sha512 against nuget.org's"])


class ProtocolTests(unittest.TestCase):
    def describe(self, **changes: object) -> str:
        record = {"record": "describe", "protocol": 1, "runner_version": "0.1.0", "runtime_version": "1.31.0",
                  "runtime_sha256": "a" * 64, "profile": "ppocrv5-latin-v1",
                  "detector_sha256": runner.MODELS[runner.DETECTOR][2],
                  "recognizer_sha256": runner.MODELS[runner.RECOGNIZER][2], **changes}
        return json.dumps(record) + "\n"

    def greeting(self, **changes: object) -> str:
        record = {"record": "runner", "protocol": 1, "runner_version": "0.1.0", "runtime_version": "1.31.0",
                  "runtime_sha256": "a" * 64, "detector_sha256": runner.MODELS[runner.DETECTOR][2],
                  "recognizer_sha256": runner.MODELS[runner.RECOGNIZER][2], **changes}
        return json.dumps(record, separators=(",", ":"))

    def test_describe_is_held_to_the_pins(self) -> None:
        self.assertEqual(runner.check_description(0, self.describe())["runtime_version"], "1.31.0")
        for changes in ({"runtime_version": "1.30.0"}, {"detector_sha256": "0" * 64}, {"protocol": 2},
                        {"runtime_sha256": "A"}):
            with self.assertRaises(RunnerCheckError):
                runner.check_description(0, self.describe(**changes))
        with self.assertRaises(RunnerCheckError):
            runner.check_description(3, '{"record":"error","image":null,"code":"runtime_unavailable"}\n')

    def test_a_read_gives_each_image_its_lines_or_none(self) -> None:
        stdout = "\n".join([
            self.greeting(),
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
        other_model = stdout.replace(self.greeting(), self.greeting(detector_sha256="0" * 64))
        with self.assertRaises(RunnerCheckError):
            runner.parse_read(other_model, 2)


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


class CrosscheckTests(unittest.TestCase):
    def test_the_reference_recogniser_on_our_boxes_is_counted_against_both_readings(self) -> None:
        import tempfile  # noqa: PLC0415
        from pathlib import Path  # noqa: PLC0415

        with tempfile.TemporaryDirectory() as folder:
            frames = Path(folder)
            (frames / f"ocr-{runner.REFERENCE_ENGINE}.jsonl").write_text(json.dumps(
                {"file": "f.png", "seconds": 0.1, "lines": [
                    {"text": "Total 4407", "box": [10, 10, 100, 20], "confidence": 0.9}]}) + "\n", encoding="utf-8")
            (frames / f"ocr-{runner.ENGINE_NAME}.jsonl").write_text(json.dumps(
                {"file": "f.png", "seconds": 0.1, "lines": [
                    {"text": "Total 4401", "box": [10, 10, 100, 20], "confidence": 0.9}]}) + "\n", encoding="utf-8")
            counts = runner.crosscheck_frames(frames, lambda _crop: "Total 4407", lambda _path: None,
                                              lambda _image, _box: None)
        self.assertEqual(counts["same_as_runner"], 0)
        self.assertEqual(counts["same_as_reference_pipeline"], 1)
        self.assertEqual(counts["kinds_against_runner"], {"near": 1})
        self.assertEqual((counts["reference_boxes"], counts["reference_boxes_same_text"]), (1, 1))


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
