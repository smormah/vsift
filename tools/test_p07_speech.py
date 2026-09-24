"""P07 speech-fixture recipe and verifier tests that need no text-to-speech model.

A fake synthesizer stands in for Kokoro. The FFmpeg-backed tests run when FFmpeg and
FFprobe are found through `VSIFT_P07_FFMPEG`/`VSIFT_P07_FFPROBE` or on `PATH`, and are
skipped otherwise; the CI generation workflow runs them against its pinned build.
"""

from __future__ import annotations

import array
import copy
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import struct
import tempfile
import unittest

import generate_p07_speech as generator
import verify_p07_speech as verifier


MANIFEST = json.loads(generator.MANIFEST.read_text(encoding="utf-8"))
FIXTURES = {fixture["id"]: fixture for fixture in MANIFEST["fixtures"]}
REQUIREMENTS = Path(__file__).with_name("p07-speech-requirements.txt")
# SHA-256 of the first two seconds of office-v1 noise as little-endian doubles. A
# change here changes committed fixture bytes and needs a reviewed recipe version.
OFFICE_NOISE_GOLDEN = "2e72b5b16d7298312c473ac372be17275618f01e512f132323904195ed51efb7"


class FakeSynthesizer:
    """Deterministic stand-in: one 150 ms tone burst per word, 60 ms apart."""

    def __init__(self) -> None:
        self.calls: list[tuple[str, str]] = []

    def synthesize(self, text: str, voice: generator.Voice) -> generator.SynthesizedSegment:
        self.calls.append((text, voice.voice_id))
        rate = generator.SAMPLE_RATE
        samples: list[float] = [0.0] * (rate // 20)
        words = []
        for index, word in enumerate(text.split()):
            start = len(samples)
            frequency = 300 + (index * 37) % 400
            samples.extend(0.3 * math.sin(2 * math.pi * frequency * n / rate) for n in range(rate * 3 // 20))
            words.append(generator.Word(word, start * 1_000_000 // rate, len(samples) * 1_000_000 // rate))
            samples.extend([0.0] * (rate * 3 // 50))
        timed = voice.lang_code == "a"
        return generator.SynthesizedSegment(tuple(samples), (f"fake:{text}",), tuple(words) if timed else None)


def facts() -> dict:
    return {"engine": "fake synthesizer for tests"}


def modified(fixture_id: str, **audio: object) -> dict:
    fixture = copy.deepcopy(FIXTURES[fixture_id])
    fixture["audio"].update(audio)
    return fixture


def find_tool(variable: str, name: str) -> str | None:
    explicit = os.environ.get(variable)
    return explicit if explicit else shutil.which(name)


class ManifestPlanTests(unittest.TestCase):
    def test_speech_bearing_fixtures_follow_the_manifest_modes(self) -> None:
        plans = generator.speech_plans(MANIFEST)
        self.assertEqual([plan.fixture_id for plan in plans],
                         ["F01", "F02", "F03", "F04", "F05", "F06", "F07", "F08", "F09", "F12"])
        for plan in plans:
            self.assertEqual(" ".join(segment.text for segment in plan.segments),
                             FIXTURES[plan.fixture_id]["audio"]["script"])

    def test_f08_is_spoken_in_english_then_spanish_inside_its_speech_event(self) -> None:
        plan = generator.speech_plan(FIXTURES["F08"])
        assert plan is not None
        self.assertTrue(plan.noisy)
        self.assertEqual([segment.language for segment in plan.segments], ["en-US", "es"])
        self.assertEqual(plan.segments[1].text, "El identificador es AB-731.")
        self.assertEqual((plan.window_start_us, plan.window_end_us), (1_000_000, 17_000_000))
        self.assertEqual(plan.window_source, "manifest event F08-E01")

    def test_f09_offset_and_window_come_from_the_manifest(self) -> None:
        plan = generator.speech_plan(FIXTURES["F09"])
        assert plan is not None
        self.assertEqual(plan.audio_offset_us, 750_000)
        self.assertEqual((plan.window_start_us, plan.window_end_us), (4_000_000, 6_000_000))
        self.assertFalse(plan.noisy)

    def test_fixtures_without_a_speech_event_use_the_lead_in_policy(self) -> None:
        plan = generator.speech_plan(FIXTURES["F01"])
        assert plan is not None
        self.assertEqual((plan.window_start_us, plan.window_end_us, plan.window_source),
                         (500_000, 5_750_000, "lead-in policy"))

    def test_silent_fixtures_are_skipped_and_unknown_modes_fail(self) -> None:
        self.assertIsNone(generator.speech_plan(FIXTURES["F10"]))
        self.assertIsNone(generator.speech_plan(FIXTURES["F11"]))
        with self.assertRaises(generator.GenerationError):
            generator.speech_plan(modified("F01", mode="whispered-synthetic"))

    def test_recipe_preconditions_fail_closed(self) -> None:
        cases = [
            modified("F09", offset_us=0),
            modified("F01", script="   "),
            modified("F01", offset_us=6_000_000),
            modified("F08", script="Only one sentence here."),
        ]
        several = copy.deepcopy(FIXTURES["F08"])
        several["events"].append(dict(several["events"][0], id="F08-E02"))
        cases.append(several)
        for fixture in cases:
            with self.subTest(audio=fixture["audio"]), self.assertRaises(generator.GenerationError):
                generator.speech_plan(fixture)

    def test_sentence_split_keeps_decimals_and_is_lossless(self) -> None:
        self.assertEqual(generator.split_sentences(FIXTURES["F03"]["audio"]["script"]),
                         ["This total should be 125.00.", "After recalculation it incorrectly becomes 127.50."])
        self.assertEqual(generator.split_sentences(FIXTURES["F08"]["audio"]["script"])[0],
                         "Request AB-731 fails with code E-409 after 2.5 seconds.")
        with self.assertRaises(generator.GenerationError):
            generator.split_sentences("One.  Two.")


class PlacementTests(unittest.TestCase):
    def test_offset_fixture_places_speech_at_its_event_on_the_stream_local_timeline(self) -> None:
        plan = generator.speech_plan(FIXTURES["F09"])
        assert plan is not None
        placement = generator.place(plan, 36_000)
        self.assertEqual(placement.track_start_frame, 78_000)  # (4.0 - 0.75) s at 24 kHz.
        self.assertEqual(placement.track_frames, 270_000)  # 11.25 s.
        self.assertEqual((placement.speech_start_us, placement.speech_end_us), (4_000_000, 5_500_000))

    def test_end_time_rounds_up_to_the_next_microsecond(self) -> None:
        plan = generator.speech_plan(FIXTURES["F01"])
        assert plan is not None
        self.assertEqual(generator.place(plan, 1).speech_end_us, 500_042)

    def test_an_utterance_that_overruns_its_window_is_rejected(self) -> None:
        plan = generator.speech_plan(FIXTURES["F09"])
        assert plan is not None
        generator.place(plan, 48_000)  # Exactly 2.0 s fits the 4.0-6.0 s window.
        with self.assertRaises(generator.GenerationError):
            generator.place(plan, 48_001)
        with self.assertRaises(generator.GenerationError):
            generator.place(plan, 0)

    def test_times_off_the_sample_grid_are_rejected(self) -> None:
        self.assertEqual(generator.exact_frames(3_250_000), 78_000)
        with self.assertRaises(generator.GenerationError):
            generator.exact_frames(10)
        self.assertEqual(generator.format_seconds(750_000), "0.750000")
        self.assertEqual(generator.format_seconds(2_000_000), "2.000000")
        with self.assertRaises(generator.GenerationError):
            generator.format_seconds(-1)


class AudioRecipeTests(unittest.TestCase):
    def test_quantize_rounds_and_counts_clipping(self) -> None:
        samples, clipped = generator.quantize([0.0, 0.5, -0.5, 1.0, -1.0, 1.5, -1.5])
        self.assertEqual(list(samples), [0, 16_384, -16_384, 32_767, -32_767, 32_767, -32_768])
        self.assertEqual(clipped, 2)

    def test_wav_round_trip_is_exact_and_refuses_to_overwrite(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "clip.wav"
            data = array.array("h", [0, 1, -1, 32_767, -32_768])
            generator.write_wav(path, data)
            self.assertEqual(list(generator.read_wav(path)), list(data))
            with self.assertRaises(FileExistsError):
                generator.write_wav(path, data)

    def test_office_noise_is_reproducible_and_seeded(self) -> None:
        noise = generator.office_noise(2 * generator.SAMPLE_RATE)
        self.assertEqual(noise, generator.office_noise(2 * generator.SAMPLE_RATE))
        self.assertNotEqual(noise, generator.office_noise(2 * generator.SAMPLE_RATE, seed=409))
        digest = hashlib.sha256(struct.pack(f"<{len(noise)}d", *noise)).hexdigest()
        self.assertEqual(digest, OFFICE_NOISE_GOLDEN)

    def test_clean_track_is_silence_plus_the_exact_utterance(self) -> None:
        plan = generator.speech_plan(FIXTURES["F01"])
        assert plan is not None
        utterance = array.array("h", [5, -5, 7])
        placement = generator.place(plan, len(utterance))
        track, noise = generator.build_track(plan, placement, utterance)
        self.assertIsNone(noise)
        self.assertEqual(len(track), 144_000)
        self.assertEqual(list(track[12_000:12_003]), [5, -5, 7])
        self.assertEqual(sum(1 for value in track if value), 3)

    def test_noisy_track_meets_the_documented_signal_to_noise_ratio(self) -> None:
        plan = generator.speech_plan(FIXTURES["F08"])
        assert plan is not None
        utterance = array.array("h", [round(8_000 * math.sin(n / 7)) for n in range(48_000)])
        placement = generator.place(plan, len(utterance))
        track, noise = generator.build_track(plan, placement, utterance)
        assert noise is not None
        self.assertEqual(noise["snr_db"], 10)
        self.assertEqual(noise["mix_gain"], 1.0)
        noise_only = [value / generator.INT16_SCALE for value in track[:placement.track_start_frame]]
        speech_level = generator.rms([value / generator.INT16_SCALE for value in utterance])
        ratio_db = 20 * math.log10(speech_level / generator.rms(noise_only))
        self.assertAlmostEqual(ratio_db, 10.0, delta=0.5)
        self.assertEqual(generator.build_track(plan, placement, utterance)[0], track)


class SynthesisAndArgvTests(unittest.TestCase):
    def test_utterance_joins_language_segments_with_the_fixed_pause(self) -> None:
        plan = generator.speech_plan(FIXTURES["F08"])
        assert plan is not None
        fake = FakeSynthesizer()
        utterance, clipped, segments = generator.synthesize_utterance(plan, fake)
        self.assertEqual(clipped, 0)
        self.assertEqual([voice for _, voice in fake.calls], ["af_heart", "ef_dora"])
        first, second = segments
        self.assertEqual(second["start_frame"], first["frames"] + 9_600)
        self.assertEqual(len(utterance), second["start_frame"] + second["frames"])
        self.assertIsNotNone(first["words"])
        self.assertIsNone(second["words"])

    def test_synthesize_writes_every_utterance_and_refuses_an_existing_record(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            record = generator.synthesize(output, FakeSynthesizer(), facts())
            files = [entry["file"] for entry in record["synthesis"]["utterances"]]
            self.assertEqual(len(files), 10)
            self.assertIn("speech/F09-utterance.wav", files)
            self.assertIsNone(record["assembly"])
            for entry in record["synthesis"]["utterances"]:
                self.assertEqual(generator.sha256_file(output / entry["file"]), entry["sha256"])
            with self.assertRaises(FileExistsError):
                generator.synthesize(output, FakeSynthesizer(), facts())

    def test_mux_argv_copies_video_and_encodes_audio_without_a_shell(self) -> None:
        self.assertEqual(
            generator.mux_argv("ffmpeg", "F01.mp4", "track.wav", "F01-speech.mp4", ".mp4", 0, 0),
            ["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin", "-n", "-i", "F01.mp4",
             "-i", "track.wav", "-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy", "-c:a", "aac",
             "-b:a", "64k", "-ar", "16000", "-ac", "1", "-fflags", "+bitexact", "-movflags", "+faststart",
             "F01-speech.mp4"])
        self.assertEqual(
            generator.mux_argv("ffmpeg", "F09.mkv", "track.wav", "F09-speech.mkv", ".mkv", 750_000, 2_000_000),
            ["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin", "-n", "-i", "F09.mkv",
             "-itsoffset", "0.750000", "-i", "track.wav", "-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy",
             "-c:a", "pcm_s16le", "-ar", "16000", "-ac", "1", "-fflags", "+bitexact",
             "-output_ts_offset", "2.000000", "F09-speech.mkv"])
        with self.assertRaises(generator.GenerationError):
            generator.mux_argv("ffmpeg", "F01.avi", "t.wav", "o.avi", ".avi", 0, 0)

    def test_p04_record_supplies_source_names_and_origins(self) -> None:
        bases = generator.base_fixtures(json.loads(generator.BASE_PROVENANCE.read_text(encoding="utf-8")))
        self.assertEqual(bases["F09"][0], "F09.mkv")
        self.assertEqual(bases["F09"][3], 2_000_000)
        self.assertEqual(bases["F01"][0], "F01.mp4")
        self.assertEqual(bases["F01"][3], 0)
        speech_names = {generator.variant_name(fixture, name) for fixture, (name, _, _, _) in bases.items()}
        self.assertFalse(speech_names & {name for name, _, _, _ in bases.values()})

    def test_requirements_file_matches_the_pinned_distributions(self) -> None:
        pins = {}
        for line in REQUIREMENTS.read_text(encoding="utf-8").splitlines():
            if line and not line.startswith("#"):
                match = re.fullmatch(r"([a-z0-9-]+)(?:\[[a-z]+\])?==([0-9.]+)", line)
                self.assertIsNotNone(match, line)
                assert match is not None
                pins[match.group(1)] = match.group(2)
        expected = {name: version.split("+")[0] for name, version in generator.PINNED_DISTRIBUTIONS.items()
                    if name != "en-core-web-sm"}
        self.assertEqual(pins, expected)

    def test_pip_reports_are_summarized_with_their_digests(self) -> None:
        report = {"install": [
            {"metadata": {"name": "Kokoro", "version": "0.9.4"},
             "download_info": {"url": "https://files.example/kokoro.whl",
                               "archive_info": {"hashes": {"sha256": "aa"}}}},
            {"metadata": {"name": "en_core_web_sm", "version": "3.8.0"},
             "download_info": {"url": "file:///wheel", "archive_info": {"hash": "sha256=bb"}}},
        ]}
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "report.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            self.assertEqual(generator.resolved_distributions([path]), [
                {"name": "en-core-web-sm", "version": "3.8.0", "url": "file:///wheel", "sha256": "bb"},
                {"name": "kokoro", "version": "0.9.4", "url": "https://files.example/kokoro.whl", "sha256": "aa"},
            ])

    def test_pins_are_checked_by_size_and_published_digest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            content = b'{"vocab": {}}\n'
            blob = hashlib.sha1(b"blob %d\0" % len(content) + content).hexdigest()
            pin = generator.PinnedFile("kokoro/config.json", "https://example.invalid/config.json",
                                       len(content), git_blob_sha1=blob)
            (root / "kokoro").mkdir()
            (root / "kokoro" / "config.json").write_bytes(content)
            self.assertEqual(generator.verified_file(root, pin)[1], hashlib.sha256(content).hexdigest())
            (root / "kokoro" / "config.json").write_bytes(content.replace(b"{}", b"[]"))
            with self.assertRaises(generator.GenerationError):
                generator.verified_file(root, pin)

    def test_every_model_download_is_https_at_the_pinned_revision(self) -> None:
        for pin in generator.MODEL_FILES:
            self.assertTrue(pin.url.startswith(
                f"https://huggingface.co/hexgrad/Kokoro-82M/resolve/{generator.KOKORO_REVISION}/"))
            self.assertTrue(pin.sha256 or pin.git_blob_sha1)
        self.assertTrue(generator.SPACY_MODEL_WHEEL.url.startswith("https://github.com/explosion/"))


class EnergyCheckTests(unittest.TestCase):
    def test_clean_energy_must_stay_inside_the_recorded_span(self) -> None:
        powers = [0.0] * 50 + [verifier.ACTIVE_POWER * 10] * 50 + [0.0] * 50
        result = verifier.check_clean_energy("FX", powers, 0, 1_000_000, 2_000_000)
        self.assertEqual(result["first_active_us"], 1_000_000)
        with self.assertRaises(verifier.VerificationError):
            verifier.check_clean_energy("FX", powers, 0, 1_500_000, 2_000_000)
        with self.assertRaises(verifier.VerificationError):
            verifier.check_clean_energy("FX", [0.0] * 150, 0, 1_000_000, 2_000_000)

    def test_noisy_energy_needs_full_noise_and_a_louder_speech_span(self) -> None:
        floor = verifier.NOISE_FLOOR_POWER * 100
        powers = [floor] * 50 + [floor * 11] * 50 + [floor] * 50
        verifier.check_noisy_energy("FX", powers, 0, 1_000_000, 2_000_000)
        with self.assertRaises(verifier.VerificationError):
            verifier.check_noisy_energy("FX", [floor] * 150, 0, 1_000_000, 2_000_000)
        with self.assertRaises(verifier.VerificationError):
            verifier.check_noisy_energy("FX", [0.0] * 100 + [floor] * 50, 0, 1_000_000, 2_000_000)

    def test_recorded_paths_cannot_escape_the_generated_directory(self) -> None:
        root = Path("root")
        self.assertEqual(verifier.contained(root, "speech/F01-utterance.wav"),
                         root / "speech" / "F01-utterance.wav")
        for unsafe in ("../F01.mp4", "/etc/passwd", "speech\\..\\x", "C:/x"):
            with self.subTest(unsafe=unsafe), self.assertRaises(verifier.VerificationError):
                verifier.contained(root, unsafe)


FFMPEG = find_tool("VSIFT_P07_FFMPEG", "ffmpeg")
FFPROBE = find_tool("VSIFT_P07_FFPROBE", "ffprobe")


@unittest.skipUnless(FFMPEG and FFPROBE, "FFmpeg and FFprobe are required for the assembly tests")
class AssemblyWithFfmpegTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.temporary = tempfile.TemporaryDirectory(prefix="vsift-p07-test-")
        cls.first = Path(cls.temporary.name) / "first"
        cls.first.mkdir()
        generator.synthesize(cls.first, FakeSynthesizer(), facts())
        cls.second = Path(cls.temporary.name) / "second"
        shutil.copytree(cls.first, cls.second)
        cls.record = generator.assemble(cls.first, str(FFMPEG))
        generator.assemble(cls.second, str(FFMPEG))

    @classmethod
    def tearDownClass(cls) -> None:
        cls.temporary.cleanup()

    def verify(self, generated: Path, provenance: Path | None = None) -> dict:
        return verifier.verify(str(FFMPEG), str(FFPROBE), generated,
                               provenance or generated / generator.PROVENANCE_NAME, generator.GENERATED)

    def test_assembled_variants_pass_the_independent_verifier(self) -> None:
        report = self.verify(self.first)
        self.assertEqual(len(report["variants"]), 10)
        by_fixture = {check["fixture"]: check for check in report["variants"]}
        self.assertEqual(by_fixture["F09"]["origin_us"], 2_000_000)
        self.assertEqual(by_fixture["F09"]["audio_offset_us"], 750_000)
        self.assertEqual(by_fixture["F09"]["speech_span_us"][0], 4_000_000)
        self.assertIn("speech_to_noise_power_ratio", by_fixture["F08"]["energy"])

    def test_the_recorded_command_template_holds_no_local_paths(self) -> None:
        for variant in self.record["assembly"]["variants"]:
            template = " ".join(variant["command_template"])
            self.assertNotIn(self.temporary.name, template)
            self.assertTrue(variant["command_template"][0] == "<FFMPEG>")

    def test_a_second_assembly_produces_identical_bytes(self) -> None:
        self.verify(self.second, self.first / generator.PROVENANCE_NAME)

    def test_existing_p04_fixtures_are_untouched(self) -> None:
        base = json.loads(generator.BASE_PROVENANCE.read_text(encoding="utf-8"))
        for entry in base["fixtures"]:
            name = re.split(r"[\\/]", entry["command_template"][-1])[-1]
            self.assertEqual(generator.sha256_file(generator.GENERATED / name), entry["sha256"])

    def test_assembly_refuses_to_run_twice_into_one_directory(self) -> None:
        with self.assertRaises(generator.GenerationError):
            generator.assemble(self.first, str(FFMPEG))

    def test_a_tampered_utterance_or_misplaced_span_fails_verification(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            tampered = Path(temporary) / "tampered"
            shutil.copytree(self.first, tampered)
            utterance = tampered / "speech" / "F01-utterance.wav"
            data = bytearray(utterance.read_bytes())
            data[-1] ^= 0x01
            utterance.write_bytes(bytes(data))
            with self.assertRaises(verifier.VerificationError):
                self.verify(tampered)

        with tempfile.TemporaryDirectory() as temporary:
            moved = Path(temporary) / "moved"
            shutil.copytree(self.first, moved)
            record_path = moved / generator.PROVENANCE_NAME
            record = json.loads(record_path.read_text(encoding="utf-8"))
            variant = next(entry for entry in record["assembly"]["variants"] if entry["fixture"] == "F09")
            variant["speech_start_us"] += 250_000
            variant["speech_end_us"] += 250_000
            record_path.write_text(json.dumps(record), encoding="utf-8")
            with self.assertRaises(verifier.VerificationError):
                self.verify(moved)


if __name__ == "__main__":
    unittest.main()
