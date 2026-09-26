"""Tests of the independent P04 fixture verifier's frame-timestamp lists (P09 V-01 truth)."""

from __future__ import annotations

from decimal import Decimal
import json
from pathlib import Path
import shutil
import unittest

import verify_p04_fixtures as verifier


GENERATED = Path(__file__).resolve().parents[1] / "fixtures" / "corpus" / "generated"


class NormalizedMicrosecondsTests(unittest.TestCase):
    def test_times_are_made_relative_to_the_origin_exactly(self) -> None:
        times = [Decimal("2.000000"), Decimal("2.350000"), Decimal("5.250000")]
        self.assertEqual(verifier.normalized_microseconds(times, Decimal("2.000000")), [0, 350_000, 3_250_000])

    def test_inexact_negative_or_unordered_times_are_rejected(self) -> None:
        for times, origin in [
            ([Decimal("0.0000005")], Decimal("0")),
            ([Decimal("1.5")], Decimal("2")),
            ([Decimal("0.1"), Decimal("0.1")], Decimal("0")),
            ([Decimal("0.2"), Decimal("0.1")], Decimal("0")),
        ]:
            with self.assertRaises(AssertionError):
                verifier.normalized_microseconds(times, origin)


class RecordedListTests(unittest.TestCase):
    """The committed verifier report holds the lists the adapter tests compare with."""

    def setUp(self) -> None:
        self.report = json.loads((GENERATED / "verification.json").read_text(encoding="utf-8"))

    def test_the_report_lists_f01_f09_and_the_rotation_variant(self) -> None:
        lists = self.report["frame_timestamps_us"]
        self.assertEqual(set(lists), {"F01", "F09", "F01-rotation-90"})
        self.assertEqual(self.report["verifier"], "tools/verify_p04_fixtures.py v2")

    def test_f01_and_its_rotation_follow_the_twenty_hertz_generation_grid(self) -> None:
        grid = [frame * 50_000 for frame in range(120)]
        self.assertEqual(self.report["frame_timestamps_us"]["F01"], grid)
        self.assertEqual(self.report["frame_timestamps_us"]["F01-rotation-90"], grid)

    def test_f09_keeps_its_variable_frames_and_frozen_boundaries(self) -> None:
        # The generator keeps frames 0, 7, 14, ... and 65, 145, 239 of a
        # 20 Hz grid (tools/generate_p04_fixtures.py).
        kept = sorted({frame for frame in range(240) if frame % 7 == 0} | {65, 145, 239})
        self.assertEqual(self.report["frame_timestamps_us"]["F09"], [frame * 50_000 for frame in kept])
        self.assertIn(3_250_000, self.report["frame_timestamps_us"]["F09"])
        self.assertIn(7_250_000, self.report["frame_timestamps_us"]["F09"])


@unittest.skipUnless(shutil.which("ffprobe"), "ffprobe is not on PATH")
class LiveFrameTimesTests(unittest.TestCase):
    def test_ffprobe_frame_times_match_the_committed_lists(self) -> None:
        ffprobe = shutil.which("ffprobe") or ""
        report = json.loads((GENERATED / "verification.json").read_text(encoding="utf-8"))
        for fixture, file, origin in [
            ("F01", "F01.mp4", Decimal("0")),
            ("F09", "F09.mkv", Decimal("2.000000")),
            ("F01-rotation-90", "F01-rotation-90.mp4", Decimal("0")),
        ]:
            times = verifier.frame_times(ffprobe, GENERATED / file)
            self.assertEqual(verifier.normalized_microseconds(times, origin), report["frame_timestamps_us"][fixture])


if __name__ == "__main__":
    unittest.main()
