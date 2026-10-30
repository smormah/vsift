"""Guardrails for the second clock evidence run: what counts as a persisting misread, and the scenes."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import p16_clock_tracks as tracks
from p16_taskbar_clocks import Verdict


def verdicts(*reads: str | None) -> list[Verdict]:
    result = []
    for read in reads:
        if read is None:
            result.append(Verdict("not_found", None, None))
        elif read == "true":
            result.append(Verdict("parsed_exact", 0.99, "14:03"))
        else:
            result.append(Verdict("parsed_wrong", 0.95, read))
    return result


class Persistence(unittest.TestCase):
    def test_a_misread_filling_a_minute_is_told_from_one_that_came_and_went(self) -> None:
        values = [
            tracks.tally_value("14:03", verdicts("14:08", "14:08", "14:08", "14:08", "14:08")),
            tracks.tally_value("14:04", verdicts("true", "14:09", "true", "true", "true")),
            tracks.tally_value("14:05", verdicts("14:06", "14:09", "14:06", "14:09", "true")),
            tracks.tally_value("14:06", verdicts("true", "true", "true", None, "true")),
        ]
        report = tracks.persistence_report(values)
        self.assertEqual(report["values"], 4)
        self.assertEqual(report["reads"], 20)
        self.assertEqual(report["reads_wrong"], 10)
        self.assertEqual(report["reads_not_found"], 1)
        self.assertEqual(report["values_misread"], 3)
        self.assertEqual(report["all_reads_same_wrong"], 1)
        self.assertEqual(report["some_reads_wrong"], 2)
        self.assertEqual(report["wrong_reads_per_misread_value"], {"1": 1, "2": 0, "3": 0, "4": 1, "5": 1})
        self.assertEqual(report["still_wrong_settled"], 1)
        self.assertEqual(report["by_time_after_change"]["0.1s"]["parsed_wrong"], 2)
        self.assertEqual(report["by_time_after_change"]["30s"]["parsed_wrong"], 1)

    def test_no_misread_is_an_empty_answer_not_a_failure(self) -> None:
        report = tracks.persistence_report([tracks.tally_value("14:03", verdicts("true", "true"))])
        self.assertEqual(report["values_misread"], 0)
        self.assertEqual(report["samples"], [])

    def test_values_are_consecutive_minutes_past_midnight(self) -> None:
        self.assertEqual(tracks.consecutive_minutes("24h", 23 * 3_600 + 59 * 60, 3), ["23:59", "00:00", "00:01"])
        self.assertEqual(tracks.consecutive_minutes("12h", 12 * 3_600 + 59 * 60, 2), ["12:59 PM", "1:00 PM"])

    def test_every_read_falls_inside_its_own_held_minute(self) -> None:
        self.assertEqual(tracks.READS_PER_VALUE, 5)
        self.assertLess(max(tracks.FRAMES_AFTER_CHANGE), tracks.HOLD_S * tracks.FRAME_RATE)
        self.assertGreaterEqual(min(tracks.FRAMES_AFTER_CHANGE), 1, "never the frame of the change itself")
        self.assertIn("eq(mod(n", tracks.frame_select())


class Scenes(unittest.TestCase):
    def test_a_cut_jumps_by_a_length_no_quarter_hour_explains(self) -> None:
        scene = next(scene for scene in tracks.scenes() if scene.name == "a-cut")
        values = scene.clocks[0]["values"]
        self.assertEqual((values[2][2], values[3][2]), ("14:05", "14:13"))
        self.assertEqual(scene.expected_gaps, 1)

    def test_every_scene_is_one_timeline_with_no_overlap_in_a_clock(self) -> None:
        for scene in tracks.scenes():
            for clock in scene.clocks:
                self.assertEqual([value[0] for value in clock["values"]], [value[1] - 60 for value in clock["values"]])

    def test_the_static_scene_holds_times_that_never_move(self) -> None:
        scene = next(scene for scene in tracks.scenes() if scene.name == "static-times")
        self.assertIn("Message sent at 10:32", scene.body)
        self.assertEqual(scene.expected_clocks, 1, "only the real clock counts")


class Turns(unittest.TestCase):
    def test_a_presenter_who_hands_back_is_expected_to_keep_their_clock(self) -> None:
        scene = next(scene for scene in tracks.scenes() if scene.name == "turns")
        self.assertEqual((scene.expected_clocks, scene.expected_gaps), (2, 0))
        first, second = scene.clocks
        self.assertEqual([value[0] for value in first["values"]], [0, 60, 240, 300])
        self.assertEqual([value[0] for value in second["values"]], [120, 180])


class Report(unittest.TestCase):
    def test_the_numbers_reach_a_file_as_well_as_the_summary(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            report, summary = Path(folder, "sub", "report.json"), Path(folder, "summary.md")
            value = tracks.tally_value("14:03", verdicts("14:08", "14:08", "14:08", "14:08"))
            tracks.write({"persistence": tracks.persistence_report([value])}, report, summary)
            self.assertEqual(json.loads(report.read_text(encoding="utf-8"))["persistence"]["all_reads_same_wrong"], 1)
            self.assertIn("every read the same wrong value: 1", summary.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
