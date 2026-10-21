"""Tests of the speaker-label evaluation that need no engine, model or network.

The fake synthesizer of `test_p07_speech.py` stands in for Kokoro. The assembly test that
resamples with FFmpeg runs when FFmpeg is found through `VSIFT_P07_FFMPEG` or on `PATH`,
and is skipped otherwise; the evaluation workflow runs it against its pinned build.
"""

from __future__ import annotations

import io
import json
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest import mock
import wave

import generate_p07_speech as recipe
import p16_speakers_eval as evaluation
from test_p07_speech import FFMPEG, FakeSynthesizer


def turn(speaker: str, start_s: float, end_s: float) -> dict:
    return {"speaker": speaker, "start_us": round(start_s * 1_000_000), "end_us": round(end_s * 1_000_000)}


ALTERNATING = [turn("A", 0.0, 4.0), turn("B", 5.0, 9.0), turn("A", 10.0, 14.0), turn("B", 15.0, 19.0)]


def synthesized(directory: Path) -> dict[tuple[str, str], object]:
    record = evaluation.synthesize(directory, directory, FakeSynthesizer())
    return {(entry["voice"], entry["text"]): recipe.read_wav(directory / evaluation.UTTERANCES / f"{entry['key']}.wav")
            for entry in record["utterances"]}


class ConversationTests(unittest.TestCase):
    def test_every_turn_has_a_reviewed_voice_and_every_fixture_its_speakers(self) -> None:
        for item in evaluation.CONVERSATIONS:
            self.assertEqual(set(item.voices), {turn.speaker for turn in item.turns}, item.fixture_id)
            for voice in item.voices.values():
                self.assertIn(voice, evaluation.VOICES)
                self.assertIsNotNone(evaluation.VOICES[voice].file.sha256)
        counts = {item.fixture_id: len(item.voices) for item in evaluation.CONVERSATIONS}
        self.assertEqual(counts, {"SPK2-MF": 2, "SPK2-FF": 2, "SPK2-MF-N10": 2, "SPK3-MFM": 3, "SPK1-M": 1})

    def test_a_line_shared_by_two_fixtures_is_spoken_once(self) -> None:
        utterances = evaluation.distinct_utterances()
        self.assertEqual(len(utterances), len(set(utterances)))
        support_call_a = [t.text for t in evaluation.SUPPORT_CALL if t.speaker == "A"]
        self.assertEqual(sum(1 for voice, text in utterances if voice == "af_heart" and text in support_call_a),
                         len(set(support_call_a)))

    def test_the_conversations_hold_overlaps_and_same_speaker_pauses(self) -> None:
        self.assertGreaterEqual(sum(1 for t in evaluation.SUPPORT_CALL if t.gap_ms < 0), 2)
        self.assertGreaterEqual(sum(1 for t in evaluation.PLANNING_MEETING if t.gap_ms < 0), 1)
        repeated = [a.speaker == b.speaker for a, b in zip(evaluation.SUPPORT_CALL, evaluation.SUPPORT_CALL[1:])]
        self.assertTrue(any(repeated))

    def test_the_layout_keeps_gaps_overlaps_and_the_lead_in_exactly(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            utterances = synthesized(Path(folder))
        item = evaluation.conversation("SPK2-MF")
        placed, frames = evaluation.lay_out(item, utterances)
        self.assertEqual(placed[0].speech_start_frame, evaluation.frames_for_us(evaluation.LEAD_IN_US))
        latest = placed[0].speech_end_frame
        for previous_turn, current in zip(item.turns[1:], placed[1:]):
            self.assertEqual(current.speech_start_frame,
                             latest + evaluation.frames_for_us(previous_turn.gap_ms * 1_000))
            latest = max(latest, current.speech_end_frame)
        truth = evaluation.merged_truth(placed)
        overlaps = sum(1 for a, b in zip(truth, truth[1:]) if b["start_us"] < a["end_us"])
        self.assertEqual(overlaps, 2)
        self.assertGreaterEqual(frames, max(t.start_frame + len(t.utterance) for t in placed))

    def test_the_noisy_mix_meets_its_signal_to_noise_ratio_and_never_clips(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            utterances = synthesized(Path(folder))
        item = evaluation.conversation("SPK1-M")
        placed, frames = evaluation.lay_out(item, utterances)
        clean, _ = evaluation.mix(placed, frames, None)
        noisy, record = evaluation.mix(placed, frames, 10)
        self.assertLessEqual(max(abs(value) for value in noisy), evaluation.MIX_PEAK + 1e-9)
        self.assertEqual(record["noise"], recipe.NOISE_RECIPE)
        _, clean_gain = evaluation.mix(placed, frames, None)
        speech = [index for t in placed for index in range(t.speech_start_frame, t.speech_end_frame)]
        silent = [index for index in range(frames) if index < placed[0].start_frame]
        speech_power = sum(clean[i] ** 2 for i in speech) / len(speech) * (record["gain"] / clean_gain["gain"]) ** 2
        noise_power = sum(noisy[i] ** 2 for i in silent) / len(silent)
        self.assertGreater(speech_power / noise_power, 10 ** 0.5)
        self.assertLess(speech_power / noise_power, 10 ** 1.5)

    def test_an_utterance_of_silence_is_refused(self) -> None:
        with self.assertRaises(evaluation.EvaluationError):
            evaluation.active_span([0, 1, -2, 0])

    @unittest.skipUnless(FFMPEG, "FFmpeg is required for the assembly test")
    def test_assembly_writes_sixteen_kilohertz_audio_and_its_truth(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            evaluation.synthesize(root, root, FakeSynthesizer())
            summaries = evaluation.assemble(root, Path(FFMPEG))
            self.assertEqual([entry["fixture"] for entry in summaries],
                             [item.fixture_id for item in evaluation.CONVERSATIONS])
            for item in evaluation.CONVERSATIONS:
                truth = json.loads((root / "fixtures" / f"{item.fixture_id}.truth.json").read_text(encoding="utf-8"))
                with wave.open(str(root / "fixtures" / f"{item.fixture_id}.wav"), "rb") as audio:
                    self.assertEqual((audio.getnchannels(), audio.getframerate(), audio.getsampwidth()), (1, 16_000, 2))
                    seconds = audio.getnframes() / 16_000
                self.assertAlmostEqual(seconds, truth["duration_us"] / 1_000_000, delta=0.01)
                self.assertEqual(len(truth["turns"]), len(item.turns))


class ScoringTests(unittest.TestCase):
    def test_a_perfect_hypothesis_scores_zero_whatever_its_labels_are_called(self) -> None:
        renamed = [{**t, "speaker": {"A": "spk1", "B": "spk0"}[t["speaker"]]} for t in ALTERNATING]
        score = evaluation.diarization_error(ALTERNATING, renamed, 20_000_000)
        self.assertEqual(score["der"], 0.0)
        self.assertEqual(score["mapping"], {"A": "spk1", "B": "spk0"})

    def test_one_label_for_two_voices_is_confusion_of_the_second_voice(self) -> None:
        one = [{**t, "speaker": "spk0"} for t in ALTERNATING]
        score = evaluation.diarization_error(ALTERNATING, one, 20_000_000, collar_us=0)
        self.assertAlmostEqual(score["confusion"], 0.5, places=2)
        self.assertEqual(score["miss"], 0.0)
        self.assertEqual(score["false_alarm"], 0.0)

    def test_silence_is_all_missed_and_invented_speech_is_false_alarm(self) -> None:
        self.assertAlmostEqual(evaluation.diarization_error(ALTERNATING, [], 20_000_000)["miss"], 1.0)
        extra = ALTERNATING + [turn("spk9", 4.5, 4.9)]
        renamed = [{**t, "speaker": t["speaker"]} for t in extra]
        score = evaluation.diarization_error(ALTERNATING, renamed, 20_000_000, collar_us=0)
        self.assertAlmostEqual(score["false_alarm"], 0.4 / 16.0, places=2)

    def test_the_collar_hides_small_boundary_errors(self) -> None:
        shifted = [{**t, "start_us": t["start_us"] + 200_000} for t in ALTERNATING]
        self.assertEqual(evaluation.diarization_error(ALTERNATING, shifted, 20_000_000)["der"], 0.0)
        self.assertGreater(evaluation.diarization_error(ALTERNATING, shifted, 20_000_000, collar_us=0)["der"], 0.0)

    def test_unscored_ranges_are_left_out_like_the_collar(self) -> None:
        wrong_in_one_turn = ALTERNATING[:2] + [{**ALTERNATING[2], "speaker": "B"}] + ALTERNATING[3:]
        scored = evaluation.diarization_error(ALTERNATING, wrong_in_one_turn, 20_000_000, collar_us=0)
        self.assertAlmostEqual(scored["confusion"], 0.25, places=2)
        unscored = [{"start_us": 10_000_000, "end_us": 14_000_000}]
        left_out = evaluation.diarization_error(ALTERNATING, wrong_in_one_turn, 20_000_000, 0, unscored)
        self.assertEqual(left_out["der"], 0.0)

    def test_the_rm06_reference_is_well_formed(self) -> None:
        path = Path(evaluation.__file__).with_name("p16_speakers_rm06_reference.json")
        turns, count, unscored = evaluation.read_reference(path)
        self.assertEqual(count, 2)
        self.assertEqual({turn["speaker"] for turn in turns}, {"A", "B", "C"})
        spans = sorted([(t["start_us"], t["end_us"]) for t in turns] + [(u["start_us"], u["end_us"]) for u in unscored])
        for (_, end), (start, _) in zip(spans, spans[1:]):
            self.assertLessEqual(end, start)
        self.assertLessEqual(spans[-1][1], 623_851_000)

    def test_a_reference_with_an_undeclared_speaker_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "reference.json"
            path.write_text(json.dumps({"speakers": {"A": "x"}, "count_given": 1, "unscored": [],
                                        "turns": [{"speaker": "B", "start_s": 1, "end_s": 2}]}), encoding="utf-8")
            with self.assertRaises(evaluation.EvaluationError):
                evaluation.read_reference(path)

    def test_extra_found_speakers_map_one_to_one(self) -> None:
        overlap = {"A": {"x": 5, "y": 9, "z": 0}, "B": {"x": 7, "y": 8, "z": 1}}
        self.assertEqual(evaluation.best_mapping(overlap, ["A", "B"], ["x", "y", "z"]), {"A": "y", "B": "x"})

    def test_a_segment_gets_a_speaker_only_when_one_clearly_holds_it(self) -> None:
        self.assertEqual(evaluation.label_segment(ALTERNATING, 1_000_000, 3_000_000), "A")
        self.assertIsNone(evaluation.label_segment(ALTERNATING, 3_000_000, 6_000_000))  # 1 s of A, 1 s of B
        self.assertIsNone(evaluation.label_segment(ALTERNATING, 3_500_000, 5_200_000))  # mostly silence
        self.assertEqual(evaluation.label_segment(ALTERNATING, 1_000_000, 5_200_000), "A")  # 3 s A, 0.2 s B
        self.assertIsNone(evaluation.label_segment(ALTERNATING, 2_000_000, 2_000_000))

    def test_segment_attribution_counts_labelled_right_and_wrong(self) -> None:
        segments = [{"start_us": s * 1_000_000, "end_us": e * 1_000_000}
                    for s, e in ((0, 4), (5, 9), (10, 14), (15, 19), (3, 6))]
        swapped_last = ALTERNATING[:3] + [{**ALTERNATING[3], "speaker": "A"}]
        result = evaluation.segment_attribution(ALTERNATING, swapped_last, segments, {"A": "A", "B": "B"})
        self.assertEqual(result["segments"], 4)
        self.assertEqual(result["segments_without_one_true_speaker"], 1)
        self.assertEqual(result["labelled_share"], 1.0)
        self.assertEqual(result["correct_share_of_labelled"], 0.75)
        self.assertEqual(result["wrong_share_of_all"], 0.25)


class ParsingTests(unittest.TestCase):
    def test_sherpa_lines_with_and_without_confidence(self) -> None:
        output = ("OfflineSpeakerDiarizationConfig(segmentation=...)\nStarted\n"
                  "0.318 -- 6.865 speaker_00 confidence=0.812\n"
                  "7.017 -- 10.747 speaker_01 confidence=n/a\n12.000 -- 12.000 speaker_01\n")
        self.assertEqual(evaluation.parse_sherpa(output), [
            {"speaker": "spk0", "start_us": 318_000, "end_us": 6_865_000, "confidence": 0.812},
            {"speaker": "spk1", "start_us": 7_017_000, "end_us": 10_747_000}])

    def test_strace_lines_name_the_socket_family(self) -> None:
        lines = ['412 socket(AF_INET, SOCK_STREAM|SOCK_CLOEXEC, IPPROTO_TCP) = 3',
                 '412 connect(3, {sa_family=AF_UNIX, sun_path="/var/run/nscd/socket"}, 110) = -1 ENOENT',
                 '412 connect(4, {sa_family=AF_INET6, sin6_port=htons(443)}, 28) = 0',
                 '412 openat(AT_FDCWD, "model.onnx", O_RDONLY) = 5']
        found = [match.group(1) for line in lines for match in [evaluation.NETWORK_CALL.search(line)] if match]
        self.assertEqual(found, ["AF_INET", "AF_UNIX", "AF_INET6"])

    def test_rttm_lines(self) -> None:
        text = ("SPEAKER meeting 1 5.250 1.500 <NA> <NA> SPEAKER_01 <NA> <NA>\n"
                "SPEAKER meeting 1 0.500 4.000 <NA> <NA> SPEAKER_00 <NA> <NA>\n# comment\n")
        self.assertEqual(evaluation.parse_rttm(text), [turn("SPEAKER_00", 0.5, 4.5), turn("SPEAKER_01", 5.25, 6.75)])

    def test_whisper_json_and_tinydiarize_turns(self) -> None:
        document = {"transcription": [
            {"offsets": {"from": 0, "to": 2000}, "text": " Hello.", "speaker_turn_next": True},
            {"offsets": {"from": 2000, "to": 3500}, "text": " Hi there.", "speaker_turn_next": False},
            {"offsets": {"from": 3500, "to": 5000}, "text": " Go on."}]}
        segments = evaluation.parse_whisper_json(json.dumps(document))
        self.assertEqual([s["text"] for s in segments], ["Hello.", "Hi there.", "Go on."])
        self.assertEqual([t["speaker"] for t in evaluation.alternating_turns(segments)], ["spk0", "spk1", "spk1"])


def tar_with(members: list[tuple[tarfile.TarInfo, bytes | None]]) -> bytes:
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:bz2") as archive:
        for info, data in members:
            if data is not None:
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))
            else:
                archive.addfile(info)
    return buffer.getvalue()


def file_member(name: str) -> tarfile.TarInfo:
    info = tarfile.TarInfo(name)
    info.mode = 0o755
    return info


def link_member(name: str, target: str, kind: bytes = tarfile.SYMTYPE) -> tarfile.TarInfo:
    info = tarfile.TarInfo(name)
    info.type = kind
    info.linkname = target
    return info


class ExtractionTests(unittest.TestCase):
    def extract(self, data: bytes) -> tuple[Path, list[dict]]:
        context = tempfile.TemporaryDirectory()
        self.addCleanup(context.cleanup)
        folder = Path(context.name)
        archive = folder / "engine.tar.bz2"
        archive.write_bytes(data)
        return folder / "out", evaluation.safe_extract(archive, folder / "out")

    def test_files_and_a_library_alias_beside_its_target_are_extracted(self) -> None:
        out, files = self.extract(tar_with([
            (file_member("engine/bin/tool"), b"binary"), (file_member("engine/lib/libx.so.1.2"), b"library"),
            (link_member("engine/lib/libx.so", "libx.so.1.2"), None)]))
        self.assertEqual((out / "engine" / "lib" / "libx.so").read_bytes(), b"library")
        self.assertFalse((out / "engine" / "lib" / "libx.so").is_symlink())
        self.assertEqual(sorted(entry["path"] for entry in files),
                         ["engine/bin/tool", "engine/lib/libx.so", "engine/lib/libx.so.1.2"])

    def test_paths_and_links_that_leave_the_folder_are_refused(self) -> None:
        for members in ([(file_member("../escape"), b"x")], [(file_member("/abs/path"), b"x")],
                        [(file_member("a\\b"), b"x")],
                        [(file_member("lib/real"), b"x"), (link_member("lib/alias", "../../etc/passwd"), None)],
                        [(link_member("lib/alias", "missing"), None)]):
            with self.subTest(members=[info.name for info, _ in members]):
                with self.assertRaises(evaluation.EvaluationError):
                    self.extract(tar_with(members))

    def test_special_members_are_refused(self) -> None:
        fifo = tarfile.TarInfo("engine/pipe")
        fifo.type = tarfile.FIFOTYPE
        with self.assertRaises(evaluation.EvaluationError):
            self.extract(tar_with([(fifo, None)]))

    def test_an_archive_beyond_the_reviewed_size_is_refused(self) -> None:
        with mock.patch.object(evaluation, "MAX_EXTRACTED_BYTES", 4):
            with self.assertRaises(evaluation.EvaluationError):
                self.extract(tar_with([(file_member("engine/big"), b"12345")]))


class RunnerTests(unittest.TestCase):
    def test_a_measured_run_returns_status_output_and_time(self) -> None:
        measured = evaluation.run_measured([sys.executable, "-c", "import sys; print('out'); sys.exit(3)"])
        self.assertEqual(measured.returncode, 3)
        self.assertEqual(measured.stdout.strip(), "out")
        self.assertGreater(measured.wall_s, 0.0)
        if hasattr(os, "wait4"):
            self.assertIsNotNone(measured.peak_rss_mib)

    def test_engines_are_staged_only_on_a_hosted_runner(self) -> None:
        with mock.patch.dict(os.environ, {"GITHUB_ACTIONS": ""}):
            with tempfile.TemporaryDirectory() as folder, self.assertRaises(evaluation.EvaluationError):
                evaluation.stage(Path(folder) / "tools", everything=False)

    def test_every_engine_and_model_pin_has_a_size_and_a_published_digest(self) -> None:
        pins = (list(evaluation.SHERPA_ARCHIVES.values()) + list(evaluation.SEGMENTATION.values())
                + list(evaluation.EMBEDDINGS.values()) + list(evaluation.POLYVOICE.values())
                + [evaluation.TINYDIARIZE])
        for pin in pins:
            with self.subTest(pin=pin.name):
                self.assertTrue(pin.url.startswith("https://"))
                self.assertGreater(pin.size, 0)
                self.assertRegex(pin.sha256 or "", r"^[0-9a-f]{64}$")

    def test_the_candidate_matrix_names_each_configuration_once(self) -> None:
        names = [candidate.candidate_id for candidate in evaluation.candidates(True)]
        self.assertEqual(len(names), len(set(names)))
        self.assertTrue(set(c.candidate_id for c in evaluation.candidates(False)) <= set(names))


if __name__ == "__main__":
    unittest.main()
