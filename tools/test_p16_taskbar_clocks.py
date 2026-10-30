"""Guardrails for the taskbar-clock evidence: the time grammar, the verdict and what may leave the runner."""

from __future__ import annotations

import json
from pathlib import Path
import unittest

import p16_taskbar_clocks as clocks
from p16_ocr_evaluation import Line
from p16_taskbar_clocks import Combination


def line(text: str, confidence: float = 0.95, box: tuple[int, int, int, int] = (0, 0, 10, 10)) -> Line:
    return Line(text, box, confidence)


def combination(**changes: object) -> Combination:
    values = {"scale": "1080p", "size": 12, "font": "DejaVuSans", "bar": "dark", "face": "12h", "encoding": "crf23"}
    return Combination(**{**values, **changes})


class TimeTokens(unittest.TestCase):
    def test_every_face_the_set_draws_reads_back_as_itself(self) -> None:
        for face in clocks.FACES:
            for value in clocks.clock_values(combination(face=face)):
                self.assertEqual([token for token, _ in clocks.time_tokens(value)], [value], face)

    def test_faces_a_screen_shows_for_something_else_are_refused(self) -> None:
        for text in ("1:23", "16:9", "10:30:00.0", "1.2.3:45", "25:10", "14:75", "13:03 PM", "v2:03", "12:34:56:78",
                     "10/21/2026", "see 2:3 PM"):
            self.assertEqual(clocks.time_tokens(text), [], text)

    def test_a_longer_line_holding_a_time_counts(self) -> None:
        self.assertEqual(clocks.time_tokens("Tue 2:03 PM"), [("2:03 PM", "12h")])
        self.assertEqual(clocks.time_tokens("Recording 14:03:12 live"), [("14:03:12", "24h_seconds")])

    def test_ways_of_writing_a_meridiem_are_one_token(self) -> None:
        for text in ("2:03 PM", "2:03PM", "2:03 pm", "2:03 p.m."):
            self.assertEqual(clocks.time_tokens(text), [("2:03 PM", "12h")], text)

    def test_line_classes(self) -> None:
        self.assertEqual(clocks.line_classes("14:03"), {"24h"})
        self.assertEqual(clocks.line_classes("2026-10-21 14:03:12"), {"dated", "24h_seconds"})
        self.assertEqual(clocks.line_classes("2026-10-21T14:03:12.250Z"), {"dated", "states_zone"})
        self.assertEqual(clocks.line_classes("2026-10-21 14:03 +01:00"), {"dated", "states_zone", "24h"})
        self.assertEqual(clocks.line_classes("Oct 21, 2026 2:03 PM"), {"dated", "12h"})
        self.assertEqual(clocks.line_classes("21 Oct 2026 14:03 UTC"), {"dated", "states_zone", "24h"})
        self.assertEqual(clocks.line_classes("Nothing here"), set())


class Verdicts(unittest.TestCase):
    def test_the_true_token_alone_is_exact_and_keeps_its_score(self) -> None:
        verdict = clocks.judge_frame("2:03 PM", [line("Open items"), line("Tue 2:03 PM", 0.88)])
        self.assertEqual((verdict.outcome, verdict.score), ("parsed_exact", 0.88))

    def test_another_value_is_wrong_not_missing(self) -> None:
        verdict = clocks.judge_frame("2:08 PM", [line("2:03 PM", 0.91)])
        self.assertEqual((verdict.outcome, verdict.read, verdict.score), ("parsed_wrong", "2:03 PM", 0.91))

    def test_a_second_time_beside_the_truth_is_wrong(self) -> None:
        verdict = clocks.judge_frame("14:03", [line("14:03"), line("14:08", 0.7)])
        self.assertEqual((verdict.outcome, verdict.read), ("parsed_wrong", "14:08"))

    def test_nothing_time_shaped_or_a_failed_frame_is_not_found(self) -> None:
        self.assertEqual(clocks.judge_frame("14:03", [line("1403")]).outcome, "not_found")
        self.assertEqual(clocks.judge_frame("14:03", None).outcome, "not_found")


    def test_a_miss_shows_only_what_was_read_in_the_bar(self) -> None:
        lines = [line("Open items", box=(120, 100, 300, 20)), line("1403", box=(1800, 1040, 50, 14))]
        self.assertEqual(clocks.bar_text(lines), "1403")
        self.assertEqual(clocks.bar_text(None), "")


class Report(unittest.TestCase):
    def frames(self, exact: int, wrong: int, missing: int) -> list:
        chosen = combination()
        return ([(chosen, "2:03 PM", clocks.Verdict("parsed_exact", 0.9, "2:03 PM"), "")] * exact
                + [(chosen, "2:08 PM", clocks.Verdict("parsed_wrong", 0.6, "2:03 PM"), "")] * wrong
                + [(chosen, "2:03 PM", clocks.Verdict("not_found", None, None), "1403")] * missing)

    def test_the_go_is_two_thirds_exact_at_the_judged_scale_and_rate(self) -> None:
        self.assertTrue(clocks.tally_groups(self.frames(2, 1, 0))["goes"])
        self.assertFalse(clocks.tally_groups(self.frames(65, 20, 15))["goes"])

    def test_other_scales_and_rates_never_decide(self) -> None:
        good = clocks.Verdict("parsed_exact", 0.9, "2:03 PM")
        frames = [(combination(scale="1440p-scaled"), "2:03 PM", good, ""), (combination(encoding="1mbit"), "2:03 PM", good, "")]
        report = clocks.tally_groups(frames + self.frames(0, 1, 0))
        self.assertFalse(report["goes"])
        self.assertEqual(report["groups"]["rate/1mbit"]["frames"], 1)
        self.assertEqual(report["groups"]["scale/1440p-scaled"]["frames"], 1)

    def test_a_wrong_read_names_its_confusion_and_the_scores_stay_apart(self) -> None:
        report = clocks.tally_groups(self.frames(1, 1, 0))
        self.assertEqual(report["confusions"], [("8>3", 1)])
        self.assertEqual(report["groups"]["verdict"]["scores_exact"]["min"], 0.9)
        self.assertEqual(report["groups"]["verdict"]["scores_wrong"]["min"], 0.6)

    def test_percentiles_are_nearest_rank(self) -> None:
        spread = clocks.score_spread([float(value) for value in range(1, 11)])
        self.assertEqual((spread["min"], spread["p10"], spread["p50"], spread["p90"]), (1.0, 1.0, 5.0, 9.0))
        self.assertIsNone(clocks.score_spread([]))


class RealCounts(unittest.TestCase):
    """The rule of the workflow: only numbers leave the runner."""

    SECRET = "2026-10-21 14:03:12 private-customer-name"

    def test_only_integers_and_our_own_names_leave(self) -> None:
        counts = clocks.count_recording([[line(self.SECRET), line("14:03 Secret Project", 0.75)], None])

        def leaves(value: object) -> list:
            if isinstance(value, dict):
                return [leaf for child in value.values() for leaf in leaves(child)]
            return [value]

        self.assertTrue(all(type(leaf) is int for leaf in leaves(counts)))
        for text in (json.dumps(counts), clocks.real_markdown({"RM01": counts})):
            for forbidden in ("private", "Secret", "14:03", "2026-10-21"):
                self.assertNotIn(forbidden, text)

    def test_counts_and_score_marks(self) -> None:
        counts = clocks.count_recording([[line("14:03", 0.75), line("15:04", 0.95), line("words")], None])
        self.assertEqual((counts["frames_read"], counts["frames_failed"], counts["lines_read"]), (1, 1, 3))
        row = counts["classes"]["24h"]
        self.assertEqual((row["lines"], row["score_0.70"], row["score_0.80"], row["score_0.90"]), (2, 2, 1, 1))

    def test_the_summary_says_a_count_is_not_a_clock(self) -> None:
        self.assertIn("A time-shaped line is not a clock", clocks.real_markdown({}))


class Rendering(unittest.TestCase):
    def test_the_set_holds_every_factor_and_the_second_rate_only_at_native_size(self) -> None:
        chosen = clocks.combinations(["DejaVuSans", "LiberationSans-Regular"])
        self.assertEqual(sum(item.encoding == "crf23" for item in chosen), 96)
        self.assertEqual(sum(item.encoding == "1mbit" for item in chosen), 48)
        self.assertTrue(all(item.scale == "1080p" for item in chosen if item.encoding == "1mbit"))

    def test_values_are_distinct_repeatable_and_shared_by_both_rates(self) -> None:
        values = clocks.clock_values(combination())
        self.assertEqual(len(set(values)), clocks.VALUES_PER_COMBINATION)
        self.assertEqual(values, clocks.clock_values(combination()))
        self.assertEqual(values, clocks.clock_values(combination(encoding="1mbit")))

    def test_each_value_holds_its_own_stretch_and_a_1440p_screen_is_scaled_down(self) -> None:
        files = [Path("text") / f"value-{index}.txt" for index in range(2)]
        font = Path("/fonts/DejaVuSans.ttf")
        chosen = combination(scale="1440p-scaled", size=13)
        screen = clocks.screen_filter(chosen, font, files, [Path("text/body.txt")], Path("text/s.txt"), Path("text/d.txt"))
        self.assertIn("enable='between(t,0,2)'", screen)
        self.assertIn("enable='between(t,2,4)'", screen)
        self.assertIn("fontsize=17", screen)
        self.assertTrue(screen.endswith("scale=1920:1080:flags=bicubic"))
        native = clocks.screen_filter(combination(), font, files, [Path("text/body.txt")], Path("text/s.txt"), Path("text/d.txt"))
        self.assertNotIn("scale=", native)

    def test_grain_busies_the_encoder_behind_the_text_and_never_over_the_bar(self) -> None:
        screen = clocks.screen_filter(combination(), Path("font.ttf"), [Path("text/v.txt")], [Path("text/b.txt")],
                                      Path("text/s.txt"), Path("text/d.txt"))
        self.assertTrue(screen.startswith("noise="))
        self.assertEqual(screen.count("noise="), 1)
        self.assertLess(screen.index("noise="), screen.index("drawbox="))

    def test_a_path_a_filter_would_interpret_is_refused(self) -> None:
        with self.assertRaises(clocks.TaskbarCheckError):
            clocks.checked_filter_path(Path("text/a:b.txt"))

    def test_the_body_text_is_not_time_shaped(self) -> None:
        for text in (*clocks.BODY_LINES, clocks.SCROLLING_LINE, clocks.DATE_TEXT):
            self.assertEqual(clocks.line_classes(text), set(), text)


if __name__ == "__main__":
    unittest.main()
