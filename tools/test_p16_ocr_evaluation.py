"""Guardrails for the P16 OCR engine evaluation's matching, parsing and scoring."""

from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

import p16_ocr_evaluation as evaluation
from p16_ocr_evaluation import EvaluationError, Line, Recognition


def lines_of(*texts: str) -> list[Line]:
    return [Line(text, (0, index * 20, 100, 12), 0.9) for index, text in enumerate(texts)]


class TextTests(unittest.TestCase):
    def test_normalise_keeps_case_and_punctuation_and_folds_whitespace(self) -> None:
        self.assertEqual(evaluation.normalise("  Harris\tCounty \n 4,731,145 "), "Harris County 4,731,145")
        self.assertEqual(evaluation.folded("Main Page"), "main page")

    def test_normalise_composes_accents(self) -> None:
        self.assertEqual(evaluation.normalise("Descrição"), "Descrição")

    def test_tokens_strip_edge_punctuation_only(self) -> None:
        self.assertEqual(evaluation.tokens("Rate: 15.2%, ($67,512)."), ["Rate", "15.2%", "$67,512"])

    def test_numbers_are_tokens_with_a_digit(self) -> None:
        self.assertTrue(evaluation.is_number("v1.2.3"))
        self.assertTrue(evaluation.is_number("#4821"))
        self.assertFalse(evaluation.is_number("County"))

    def test_edit_distance(self) -> None:
        self.assertEqual(evaluation.edit_distance("4,731,145", "4,731,145"), 0)
        self.assertEqual(evaluation.edit_distance("4,731,145", "4,731,14!"), 1)
        self.assertEqual(evaluation.edit_distance("4,731,145", "4,/31,145"), 1)
        self.assertEqual(evaluation.edit_distance("", "abc"), 3)

    def test_substring_distance_finds_a_line_inside_joined_columns(self) -> None:
        joined = "Harris County 4,731,145 Bexar County 2,009,324"
        self.assertEqual(evaluation.substring_distance("Bexar County 2,009,324", joined), 0)
        self.assertEqual(evaluation.substring_distance("Bexar County 2,009,324", joined.replace("009", "O09")), 1)
        self.assertEqual(evaluation.substring_distance("abc", ""), 3)
        self.assertEqual(evaluation.substring_distance("", "anything"), 0)


class TessdataTests(unittest.TestCase):
    def test_git_blob_sha1_matches_git_hash_object(self) -> None:
        # The digest `git hash-object` gives a file holding "hello" and a newline.
        self.assertEqual(evaluation.git_blob_sha1(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a")

    def test_a_model_that_differs_from_its_pin_is_never_written(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            with self.assertRaises(EvaluationError):
                evaluation.fetch_tessdata(Path(folder), ["fast"], lambda url, limit: b"not the model")
            self.assertFalse((Path(folder) / "fast" / "eng.traineddata").exists())

    def test_pins_are_commits_and_blob_digests(self) -> None:
        for repository, commit, files in evaluation.TESSDATA.values():
            self.assertTrue(repository.startswith("tesseract-ocr/tessdata_"))
            self.assertRegex(commit, r"^[0-9a-f]{40}$")
            self.assertEqual(set(files), {"eng", "por"})
            for size, blob in files.values():
                self.assertRegex(blob, r"^[0-9a-f]{40}$")
                self.assertLess(size, evaluation.MAX_TESSDATA_BYTES)


class BuildTests(unittest.TestCase):
    def test_the_build_reads_no_image_format_and_reaches_no_network(self) -> None:
        for option in ("-DENABLE_PNG=OFF", "-DENABLE_JPEG=OFF", "-DENABLE_TIFF=OFF", "-DENABLE_WEBP=OFF",
                       "-DENABLE_GIF=OFF", "-DENABLE_OPENJPEG=OFF", "-DENABLE_ZLIB=OFF"):
            self.assertIn(option, evaluation.LEPTONICA_OPTIONS)
        for option in ("-DDISABLE_CURL=ON", "-DDISABLE_ARCHIVE=ON", "-DBUILD_TRAINING_TOOLS=OFF",
                       "-DDISABLED_LEGACY_ENGINE=ON"):
            self.assertIn(option, evaluation.TESSERACT_OPTIONS)

    def test_a_clone_at_another_commit_is_refused_before_anything_builds(self) -> None:
        ran: list[list[str]] = []
        with self.assertRaises(EvaluationError):
            evaluation.build_tesseract(Path("/s"), Path("/b"), Path("/p"), lambda command, cwd: ran.append(command),
                                       lambda folder: "0" * 40)
        self.assertTrue(all(command[0] == "git" for command in ran))
        self.assertEqual(len(ran), len(evaluation.SOURCES))

    def test_leptonica_builds_before_tesseract_with_explicit_arguments(self) -> None:
        commits = {folder: commit for folder, commit in
                   ((Path("/s") / name, source[2]) for name, source in evaluation.SOURCES.items())}
        ran: list[list[str]] = []
        evaluation.build_tesseract(Path("/s"), Path("/b"), Path("/p"), lambda command, cwd: ran.append(command),
                                   lambda folder: commits[folder])
        configures = [command for command in ran if command[:2] == ["cmake", "-S"]]
        self.assertEqual([Path(command[2]).name for command in configures], ["leptonica", "tesseract"])
        self.assertIn("-DLeptonica_DIR=/p/lib/cmake/leptonica", configures[1])


class EngineTests(unittest.TestCase):
    def test_frames_of_two_videos_never_share_a_scratch_file(self) -> None:
        first = evaluation.scratch_file(Path("/scratch"), Path("/work/RM01/frame-000.png"), ".ppm")
        second = evaluation.scratch_file(Path("/scratch"), Path("/work/RM10/frame-000.png"), ".ppm")
        self.assertNotEqual(first, second)

    def test_tesseract_configurations_are_named_by_model_scale_and_page_mode(self) -> None:
        import argparse

        arguments = argparse.Namespace(engine=["tesseract-fast", "tesseract-best-x2-sparse"], tesseract="/t",
                                       tessdata="/d", ffmpeg="/f")
        fast, best = evaluation.build_engines(arguments, Path("/s"))
        self.assertEqual((fast.describe()["scale"], fast.describe()["page_mode"]), (1, "3"))
        self.assertEqual((best.describe()["scale"], best.describe()["page_mode"]), (2, "11"))
        arguments.engine = ["tesseract-fast-x3"]
        with self.assertRaises(EvaluationError):
            evaluation.build_engines(arguments, Path("/s"))

    def test_boxes_read_from_an_enlarged_copy_are_scaled_back(self) -> None:
        line = Line("Rate", (81, 57, 41, 21), 0.9)
        self.assertEqual(evaluation.unscaled(line, 2).box, (40, 28, 21, 11))
        self.assertIs(evaluation.unscaled(line, 1), line)


class ParserTests(unittest.TestCase):
    def test_tesseract_tsv_joins_words_per_line_with_their_mean_confidence(self) -> None:
        header = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext"
        rows = [
            header,
            "1\t1\t0\t0\t0\t0\t0\t0\t1280\t720\t-1\t",
            "5\t1\t1\t1\t1\t1\t40\t28\t50\t10\t90.0\tHarris",
            "5\t1\t1\t1\t1\t2\t95\t28\t50\t10\t80.0\tCounty",
            "5\t1\t1\t1\t2\t1\t40\t60\t70\t12\t70.0\t4,731,145",
            "5\t1\t1\t1\t2\t2\t120\t60\t5\t12\t95.0\t ",
        ]
        lines = evaluation.parse_tesseract_tsv("\n".join(rows))
        self.assertEqual([line.text for line in lines], ["Harris County", "4,731,145"])
        self.assertEqual(lines[0].box, (40, 28, 105, 10))
        self.assertAlmostEqual(lines[0].confidence or 0.0, 0.85)

    def test_tesseract_output_without_a_header_is_refused(self) -> None:
        with self.assertRaises(EvaluationError):
            evaluation.parse_tesseract_tsv("Harris County\n")

    def test_ocrs_json_has_lines_and_boxes_but_no_confidence(self) -> None:
        document = {"paragraphs": [{"lines": [
            {"text": "Harris County", "vertices": [[40, 28], [145, 28], [145, 38], [40, 38]], "words": []},
            {"text": "  ", "vertices": [[0, 0], [1, 1]], "words": []},
        ]}]}
        lines = evaluation.parse_ocrs_json(document)
        self.assertEqual(lines, [Line("Harris County", (40, 28, 105, 10), None)])

    def test_a_polygon_without_points_is_refused(self) -> None:
        with self.assertRaises(EvaluationError):
            evaluation.box_of([])


class SyntheticScreenTests(unittest.TestCase):
    def test_screens_are_deterministic_and_fit_the_frame(self) -> None:
        self.assertEqual(evaluation.screen_lines(3), evaluation.screen_lines(3))
        for screen in range(evaluation.SCREEN_COUNT):
            lines = evaluation.screen_lines(screen)
            self.assertEqual(len(lines), len(evaluation.FONT_SIZES) * len(evaluation.COLUMN_X))
            for line in lines:
                self.assertLess(line["y"] + line["size"] * 2, evaluation.SCREEN_HEIGHT)
                self.assertTrue(any(evaluation.is_number(token) for token in evaluation.tokens(line["text"])))
                self.assertLessEqual(len(line["text"]), 40)

    def test_neither_label_nor_number_shape_follows_the_row(self) -> None:
        # The first design tied each row's number shape and label to its row, so a
        # font size always carried the same label: a size's score was the label's.
        for row in range(len(evaluation.FONT_SIZES)):
            labels = {evaluation.screen_lines(screen)[row]["text"].rsplit(" ", 1)[0]
                      for screen in range(evaluation.SCREEN_COUNT)}
            self.assertGreater(len(labels), 4)
        texts = [line["text"] for screen in range(evaluation.SCREEN_COUNT) for line in evaluation.screen_lines(screen)]
        self.assertGreater(len(set(texts)), len(texts) * 0.95)

    def test_every_number_shape_appears(self) -> None:
        shapes = {evaluation.number_text(seed)[0] for seed in range(8)}
        self.assertTrue({"$", "#", "v", "0"} <= shapes)

    def test_each_fourth_screen_is_dark(self) -> None:
        self.assertEqual([evaluation.screen_theme(screen) for screen in range(4)], ["light", "light", "light", "dark"])

    def test_a_path_a_filter_would_interpret_is_refused(self) -> None:
        line = evaluation.screen_lines(0)[:1]
        with self.assertRaises(EvaluationError):
            evaluation.drawtext_filter(line, Path("/fonts/a:b.ttf"), [Path("/t/0.txt")], "black")
        chain = evaluation.drawtext_filter(line, Path("/fonts/DejaVuSans.ttf"), [Path("/t/0.txt")], "black")
        self.assertIn("expansion=none", chain)
        self.assertIn("textfile=/t/0.txt", chain)

    def test_synthesize_writes_truth_and_runs_tools_with_explicit_arguments(self) -> None:
        commands: list[list[str]] = []
        with tempfile.TemporaryDirectory() as folder:
            out = Path(folder) / "synthetic"

            def record(command: list[str], cwd: Path) -> None:
                self.assertEqual(cwd, out)
                commands.append(command)

            truth = evaluation.synthesize(out, Path("/usr/bin/ffmpeg"), [Path("/fonts/DejaVuSans.ttf")], record)
            self.assertEqual(len(truth["screens"]), evaluation.SCREEN_COUNT)
            self.assertEqual(len(commands), evaluation.SCREEN_COUNT + len(evaluation.ENCODINGS))
            self.assertTrue(all(isinstance(argument, str) for command in commands for argument in command))
            written = json.loads((out / "truth.json").read_text(encoding="utf-8"))
            self.assertEqual(written["screens"][1]["lines"], evaluation.screen_lines(1))
            first_text = (out / "text" / "screen-00-line-00.txt").read_text(encoding="utf-8")
            self.assertEqual(first_text, evaluation.screen_lines(0)[0]["text"])

    def test_the_control_holds_one_frame_per_screen_at_its_middle(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            out = Path(folder) / "synthetic"
            evaluation.synthesize(out, Path("/usr/bin/ffmpeg"), [Path("/fonts/DejaVuSans.ttf")], lambda command, cwd: None)
            for screen in range(evaluation.SCREEN_COUNT):
                (out / "screens" / f"screen-{screen:02d}.png").write_bytes(b"png")
            record = evaluation.write_control(out, Path(folder) / "synthetic-source")
            self.assertEqual(len(record["frames"]), evaluation.SCREEN_COUNT)
            self.assertEqual(record["frames"][2]["time_us"], 7_500_000)


class ScoringTests(unittest.TestCase):
    TRUTH = [
        {"size": 10, "text": "Harris County 4,731,145"},
        {"size": 12, "text": "Rate 15.2%"},
    ]

    def test_a_perfect_reading_is_exact_with_every_number(self) -> None:
        scores = evaluation.score_synthetic_frame(self.TRUTH, lines_of("Harris County 4,731,145", "Rate 15.2%"))
        self.assertTrue(all(score["exact"] for score in scores))
        self.assertEqual([score["numbers_found"] for score in scores], [1, 1])
        self.assertEqual(sum(score["numbers_misread"] for score in scores), 0)

    def test_a_wrong_digit_is_a_misread_and_a_missing_number_is_not(self) -> None:
        scores = evaluation.score_synthetic_frame(self.TRUTH, lines_of("Harris County 4,731,14!", "Rate"))
        self.assertEqual(scores[0]["numbers_misread"], 1)
        self.assertEqual(scores[1]["numbers_misread"], 0)
        self.assertEqual(scores[1]["numbers_found"], 0)
        self.assertAlmostEqual(scores[0]["cer"], 1 / len("Harris County 4,731,145"))

    def test_a_correct_number_of_another_line_is_not_a_misread(self) -> None:
        truth = [{"size": 10, "text": "A 2026-01-02"}, {"size": 10, "text": "B 2026-01-03"}]
        scores = evaluation.score_synthetic_frame(truth, lines_of("B 2026-01-03"))
        self.assertEqual(scores[0]["numbers_misread"], 0)

    def test_screens_are_scored_once_from_their_first_frame(self) -> None:
        truth = {"hold_us": 3_000_000, "screens": [
            {"index": 0, "theme": "light", "lines": self.TRUTH},
            {"index": 1, "theme": "dark", "lines": self.TRUTH},
        ]}
        frames = {"candidate_times_us": [0, 1_000_000, 12_000_000],
                  "frames": [{"file": "a.png", "time_us": 0}, {"file": "b.png", "time_us": 1_000_000},
                             {"file": "c.png", "time_us": 3_000_000}, {"file": "d.png", "time_us": 9_000_000}]}
        readings = [Recognition("a.png", lines_of("Harris County 4,731,145", "Rate 15.2%"), 0.5),
                    Recognition("b.png", [], 0.5), Recognition("c.png", lines_of("Rate 15.2%"), 0.7),
                    Recognition("d.png", [], 0.1)]
        scored = evaluation.score_synthetic(truth, frames, readings)
        self.assertEqual(scored["screens_scored"], 2)
        self.assertEqual(scored["screens_reached_by_candidates"], [0])
        self.assertEqual(scored["overall"]["lines"], 4)
        self.assertEqual(scored["overall"]["exact_share"], 0.75)
        self.assertEqual(scored["dark"]["exact_share"], 0.5)
        self.assertEqual(scored["by_size"]["10"]["lines"], 2)
        self.assertEqual(scored["by_size"]["8"], {"lines": 0})
        self.assertEqual(scored["seconds_per_frame"]["frames"], 4)
        self.assertIsNone(scored["seconds_per_frame"]["cpu_median"])

    def test_cpu_time_is_reported_when_known(self) -> None:
        readings = [Recognition("a.png", [], 0.5, 1.8), Recognition("b.png", [], 0.7, 2.2), Recognition("c.png", [], 0.6)]
        self.assertEqual(evaluation.timing(readings)["cpu_median"], 2.0)
        self.assertIsNone(evaluation.cpu_used(None, 3.0))
        self.assertAlmostEqual(evaluation.cpu_used(1.0, 3.5) or 0.0, 2.5)

    def test_labels_and_corroboration_on_a_real_item(self) -> None:
        readings = {
            "tesseract-fast": [Recognition("f0.png", lines_of("Main page Watchlist"), 1.0)],
            "tesseract-best": [Recognition("f0.png", lines_of("Main page Watchiist"), 2.0)],
            "ocrs": [Recognition("f0.png", lines_of("Main page Watchlist Talk"), 0.5)],
        }
        families = {"tesseract-fast": "tesseract", "tesseract-best": "tesseract", "ocrs": "ocrs"}
        scored = evaluation.score_real("RM01", readings, families)
        by_label = {label["label"]: label["found_by"] for label in scored["per_label"]}
        self.assertEqual(by_label["Watchlist"], ["ocrs", "tesseract-fast"])
        self.assertEqual(by_label["Talk"], ["ocrs"])
        self.assertEqual(by_label["Main page"], ["ocrs", "tesseract-best", "tesseract-fast"])
        self.assertEqual(by_label["Log out"], [])
        # "Watchiist" is corroborated by no other family; the same family's reading does not count.
        self.assertEqual(scored["engines"]["tesseract-best"]["corroborated_share"], 0.6667)
        self.assertEqual(scored["engines"]["tesseract-fast"]["corroborated_share"], 1.0)
        self.assertEqual(scored["engines"]["ocrs"]["median_line_confidence"], 0.9)
        self.assertEqual(scored["reachable_labels"], 3)

    def test_a_label_counts_only_as_whole_words(self) -> None:
        # The first real run counted "Ler" inside a longer word; only whole words count.
        self.assertFalse(evaluation.contains_label("a smaller table", "Ler"))
        self.assertTrue(evaluation.contains_label("ler editar ver histórico", "Ler"))
        self.assertTrue(evaluation.contains_label("print/export tools", "Print/export"))
        self.assertTrue(evaluation.contains_label("data.census.gov search", "census.gov"))
        self.assertFalse(evaluation.contains_label("mapping", "Map"))

    def test_spread_keeps_the_ends_and_the_limit(self) -> None:
        self.assertEqual(evaluation.spread(list(range(10)), 4), [0, 3, 6, 9])
        self.assertEqual(evaluation.spread([1, 2], 4), [1, 2])

    def test_languages_are_named_per_folder(self) -> None:
        self.assertEqual(evaluation.parse_languages(["RM04=por"]), {"RM04": "por"})
        with self.assertRaises(EvaluationError):
            evaluation.parse_languages(["RM04=deu"])

    def test_the_summary_renders_both_kinds(self) -> None:
        truth = {"hold_us": 3_000_000, "screens": [{"index": 0, "theme": "light", "lines": self.TRUTH}]}
        frames = {"frames": [{"file": "a.png", "time_us": 0}]}
        report = {
            "synthetic": {"synthetic-h264": {"frames": 1, "candidates": 1, "candidates_without_periodic": 1, "engines": {
                "ocrs": evaluation.score_synthetic(truth, frames, [Recognition("a.png", lines_of("Rate 15.2%"), 0.2)])}}},
            "real": {"RM01": {"frames": 1, "candidates": 2, **evaluation.score_real(
                "RM01", {"ocrs": [Recognition("f.png", lines_of("Watchlist"), 0.3)]}, {"ocrs": "ocrs"})}},
        }
        text = evaluation.markdown(report)
        self.assertIn("## synthetic-h264", text)
        self.assertIn("| ocrs | 1/1 |", text)
        self.assertIn("## RM01", text)


if __name__ == "__main__":
    unittest.main()
