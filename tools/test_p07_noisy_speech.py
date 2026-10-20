"""Noisy-speech set (issue #150) recipe and verifier tests that need no text-to-speech model.

The fake synthesizer of `test_p07_speech.py` stands in for Kokoro. The FFmpeg-backed
tests run when FFmpeg and FFprobe are found through `VSIFT_P07_FFMPEG`/`VSIFT_P07_FFPROBE`
or on `PATH`, and are skipped otherwise; the generation workflow runs them against its
pinned build.
"""

from __future__ import annotations

import copy
import hashlib
import io
import json
import math
from pathlib import Path
import shutil
import tempfile
import unittest

import fetch_p07_noisy_utterances as fetcher
import generate_p07_noisy_speech as noisy
import generate_p07_speech as recipe
import verify_p07_noisy_speech as verifier
from test_p07_speech import FFMPEG, FFPROBE, FakeSynthesizer


MANIFEST = json.loads(noisy.MANIFEST.read_text(encoding="utf-8"))
TRIAL_TRUTH = (recipe.MANIFEST, recipe.GENERATED / recipe.PROVENANCE_NAME)


def facts() -> dict:
    return {"engine": "fake synthesizer for tests"}


def spoken_words(text: str) -> list[str]:
    """Rough word split for counting; the Rust scorer is the reviewed normaliser."""
    return [word for word in "".join(c if c.isalnum() or c in ".:-" else " " for c in text).split() if word]


class LongSynthesizer(FakeSynthesizer):
    """A fake voice that speaks each word for over half a second."""

    def synthesize(self, text: str, voice: recipe.Voice, speed: float) -> recipe.SynthesizedSegment:
        result = super().synthesize(text, voice, speed)
        return recipe.SynthesizedSegment(result.samples * 4, result.phonemes, result.words)


class TruthTests(unittest.TestCase):
    def test_the_set_is_english_and_spanish_with_several_hundred_words_per_level(self) -> None:
        plan = noisy.set_plan(MANIFEST)
        words: dict[str, int] = {}
        for passage in plan.passages:
            words[passage.language] = words.get(passage.language, 0) + len(spoken_words(passage.script))
        self.assertEqual(set(words), {"en-US", "es"})
        for language, count in words.items():
            self.assertGreaterEqual(count, 200, language)
        self.assertGreaterEqual(sum(words.values()), 400)
        self.assertEqual([level.level_id for level in plan.levels], ["clean", "snr20", "snr10", "snr05"])
        self.assertEqual([level.snr_db for level in plan.noisy_levels], [20, 10, 5])

    def test_every_expected_term_is_spoken_in_its_passage(self) -> None:
        for passage in MANIFEST["passages"]:
            self.assertTrue(passage["expected_terms"], passage["id"])
            for term in passage["expected_terms"]:
                self.assertIn(term.lower(), passage["script"].lower(), passage["id"])

    def test_every_passage_has_an_identifier_or_a_number(self) -> None:
        for passage in MANIFEST["passages"]:
            self.assertTrue(any(any(c.isdigit() for c in term) for term in passage["expected_terms"]),
                            passage["id"])

    def test_the_set_is_kept_out_of_the_trial_freeze_truth(self) -> None:
        # The agent trials freeze manifest.json and speech-provenance.json; this set
        # has files of its own and never names them.
        for path in TRIAL_TRUTH:
            self.assertNotIn("noisy-speech", path.read_text(encoding="utf-8"), path.name)
        self.assertNotEqual(noisy.MANIFEST, recipe.MANIFEST)
        self.assertNotEqual(noisy.GENERATED, recipe.GENERATED)

    def test_unreviewed_levels_languages_and_sets_fail_closed(self) -> None:
        for change in (
            lambda manifest: manifest["levels"].append({"id": "snr00", "snr_db": 0}),
            lambda manifest: manifest["levels"][1].update(snr_db=None),
            lambda manifest: manifest["passages"][0].update(language="fr"),
            lambda manifest: manifest["passages"][1].update(id=manifest["passages"][0]["id"]),
            lambda manifest: manifest.update(noise_recipe="office-v2"),
            lambda manifest: manifest["noise_only"].update(duration_us=31_000_000),
        ):
            manifest = copy.deepcopy(MANIFEST)
            change(manifest)
            with self.assertRaises(recipe.GenerationError):
                noisy.set_plan(manifest)


class MixTests(unittest.TestCase):
    def test_power_ratios_are_exact_without_transcendental_functions(self) -> None:
        self.assertEqual(noisy.POWER_RATIOS[20], 100.0)
        self.assertEqual(noisy.POWER_RATIOS[10], 10.0)
        self.assertEqual(noisy.POWER_RATIOS[5], math.sqrt(10.0))

    def test_each_level_meets_its_ratio_with_the_same_noise_sequence(self) -> None:
        utterance = recipe.quantize([0.4 * math.sin(n / 7.0) for n in range(24_000)])[0]
        plan = noisy.set_plan(MANIFEST)
        start = recipe.exact_frames(noisy.LEAD_IN_US)
        records = []
        for level in plan.noisy_levels:
            track, record = noisy.passage_track(utterance, level, 150_001)
            self.assertEqual(record["seed"], 150_001)
            self.assertEqual(record["snr_db"], level.snr_db)
            noise = recipe.office_noise(len(track), 150_001)
            scaled = [value * record["noise_gain"] for value in noise]
            ratio = (record["speech_rms"] / recipe.rms(scaled)) ** 2
            self.assertAlmostEqual(ratio, level.power_ratio, places=9)
            self.assertEqual(len(track), start + len(utterance) + recipe.exact_frames(noisy.TAIL_US))
            records.append(record)
        # Only the gain differs between levels: one unscaled noise sequence for the passage.
        self.assertEqual(len({record["unscaled_noise_rms"] for record in records}), 1)
        self.assertAlmostEqual(records[0]["noise_gain"] / records[1]["noise_gain"], math.sqrt(10.0) ** -1, places=12)

    def test_the_clean_level_is_silence_around_the_exact_utterance(self) -> None:
        utterance = recipe.quantize([0.2] * 4_800)[0]
        track, record = noisy.passage_track(utterance, noisy.Level("clean", None), 150_001)
        start = recipe.exact_frames(noisy.LEAD_IN_US)
        self.assertIsNone(record)
        self.assertFalse(any(track[:start]))
        self.assertEqual(track[start:start + len(utterance)], utterance)
        self.assertFalse(any(track[start + len(utterance):]))

    def test_noise_only_clips_take_the_noise_level_of_pooled_speech(self) -> None:
        track, record = noisy.noise_only_track(48_000, noisy.Level("snr10", 10), 0.1)
        self.assertEqual(record["seed"], noisy.NOISE_ONLY_SEED)
        expected_rms = 0.1 / math.sqrt(10.0)
        measured = recipe.rms([value / recipe.INT16_SCALE for value in track]) / record["mix_gain"]
        self.assertAlmostEqual(measured, expected_rms, places=4)
        with self.assertRaises(recipe.GenerationError):
            noisy.noise_only_track(48_000, noisy.Level("clean", None), 0.1)

    def test_a_passage_too_long_for_one_recognition_window_is_named_before_writing(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaisesRegex(recipe.GenerationError, "EN01.*ES06|ES06.*EN01"):
                noisy.synthesize(Path(temporary), LongSynthesizer(), facts())
            self.assertFalse(any(Path(temporary).rglob("*.wav")))

    def test_a_sentence_at_the_phoneme_limit_stops_generation(self) -> None:
        class Verbose(FakeSynthesizer):
            def synthesize(self, text: str, voice: recipe.Voice, speed: float) -> recipe.SynthesizedSegment:
                result = super().synthesize(text, voice, speed)
                return recipe.SynthesizedSegment(result.samples, ("x" * noisy.MAX_PHONEMES,), result.words)

        passage = noisy.set_plan(MANIFEST).passages[0]
        with self.assertRaisesRegex(recipe.GenerationError, "phoneme limit"):
            noisy.synthesize_passage(passage, Verbose())

    def test_sentences_are_spoken_one_at_a_time_in_the_passage_voice(self) -> None:
        synthesizer = FakeSynthesizer()
        plan = noisy.set_plan(MANIFEST)
        spanish = next(passage for passage in plan.passages if passage.language == "es")
        _, _, sentences = noisy.synthesize_passage(spanish, synthesizer)
        self.assertEqual([text for text, _ in synthesizer.calls], list(spanish.sentences))
        self.assertEqual({voice for _, voice in synthesizer.calls}, {"ef_dora"})
        self.assertEqual(" ".join(sentence["text"] for sentence in sentences), spanish.script)
        pause = recipe.exact_frames(noisy.INTER_SENTENCE_PAUSE_US)
        self.assertEqual(sentences[1]["start_frame"], sentences[0]["frames"] + pause)

    def test_the_encode_command_is_explicit_and_audio_only(self) -> None:
        argv = noisy.encode_argv("ffmpeg", "track.wav", "out.m4a")
        self.assertEqual(argv[0], "ffmpeg")
        self.assertIn("-n", argv)
        self.assertEqual(argv[argv.index("-map") + 1], "0:a:0")
        self.assertEqual(argv[argv.index("-c:a") + 1], "aac")
        self.assertEqual(argv[-1], "out.m4a")


class FakeResponse(io.BytesIO):
    def __init__(self, data: bytes, url: str) -> None:
        super().__init__(data)
        self.url = url

    def geturl(self) -> str:
        return self.url


def release_record(data: bytes) -> dict:
    return {"synthesis": {"utterances": [{"passage": "EN01", "file": "speech/EN01-utterance.wav",
                                           "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}]},
            "utterance_release": {"tag": noisy.RELEASE_TAG, "base_url": noisy.RELEASE_BASE_URL,
                                  "assets": {"EN01": "noisy-speech-EN01-utterance.wav"}}}


class FetchTests(unittest.TestCase):
    """The release fetch, with a fake opener: nothing here touches the network."""

    DATA = b"RIFF" + bytes(200)

    def test_the_committed_record_names_every_utterance_in_the_release(self) -> None:
        record = recipe.load_json(noisy.COMMITTED_PROVENANCE)
        pinned = fetcher.pins(record)
        self.assertEqual([passage for passage, _, _, _ in pinned], [p["id"] for p in MANIFEST["passages"]])
        for _, url, _, _ in pinned:
            self.assertTrue(url.startswith(noisy.RELEASE_BASE_URL), url)

    def test_the_utterances_are_not_committed(self) -> None:
        self.assertFalse(any((noisy.GENERATED / "speech").glob("*.wav")))

    def test_a_pinned_utterance_is_kept_only_with_its_digest(self) -> None:
        url = noisy.RELEASE_BASE_URL + "noisy-speech-EN01-utterance.wav"
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "EN01-utterance.wav"
            fetcher.fetch_one(url, target, len(self.DATA), hashlib.sha256(self.DATA).hexdigest(),
                              lambda _: FakeResponse(self.DATA, url))
            self.assertEqual(target.read_bytes(), self.DATA)
            for body, size in ((self.DATA[:-1] + b"x", len(self.DATA)), (self.DATA + b"x", len(self.DATA))):
                wrong = Path(temporary) / "wrong.wav"
                with self.assertRaises(fetcher.FetchError):
                    fetcher.fetch_one(url, wrong, size, hashlib.sha256(self.DATA).hexdigest(),
                                      lambda _, body=body: FakeResponse(body, url))
                self.assertEqual(sorted(path.name for path in Path(temporary).iterdir()), ["EN01-utterance.wav"])

    def test_only_this_repositorys_release_hosts_are_used(self) -> None:
        self.assertFalse(fetcher.allowed_url("http://github.com/smormah/vsift/releases/download/x/y"))
        self.assertFalse(fetcher.allowed_url("https://example.invalid/noisy-speech-EN01-utterance.wav"))
        self.assertTrue(fetcher.allowed_url("https://release-assets.githubusercontent.com/a/b"))
        with self.assertRaises(fetcher.FetchError):
            fetcher.ReleaseRedirects().redirect_request(None, None, 302, "Found", {}, "https://example.invalid/x")
        with tempfile.TemporaryDirectory() as temporary, self.assertRaises(fetcher.FetchError):
            fetcher.fetch_one(fetcher.RELEASE_PREFIX + "x/y.wav", Path(temporary) / "y.wav", len(self.DATA),
                              hashlib.sha256(self.DATA).hexdigest(),
                              lambda url: FakeResponse(self.DATA, "https://example.invalid/y.wav"))

    def test_a_record_with_another_release_or_asset_name_is_refused(self) -> None:
        for change in (
            lambda record: record["utterance_release"].update(base_url="https://example.invalid/releases/"),
            lambda record: record["utterance_release"]["assets"].update(EN01="../EN01.wav"),
            lambda record: record["utterance_release"]["assets"].update(ES01="noisy-speech-ES01-utterance.wav"),
            lambda record: record.pop("utterance_release"),
        ):
            record = release_record(self.DATA)
            change(record)
            with self.assertRaises(fetcher.FetchError):
                fetcher.pins(record)


@unittest.skipUnless(FFMPEG and FFPROBE, "FFmpeg and FFprobe are required for the assembly tests")
class AssemblyWithFfmpegTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.temporary = tempfile.TemporaryDirectory(prefix="vsift-p07-noisy-test-")
        cls.first = Path(cls.temporary.name) / "first"
        cls.first.mkdir()
        noisy.synthesize(cls.first, FakeSynthesizer(), facts())
        cls.second = Path(cls.temporary.name) / "second"
        shutil.copytree(cls.first, cls.second)
        cls.record = noisy.assemble(cls.first, str(FFMPEG))
        noisy.assemble(cls.second, str(FFMPEG))
        cls.report = verifier.verify(str(FFMPEG), str(FFPROBE), cls.first, cls.first / noisy.PROVENANCE_NAME)

    @classmethod
    def tearDownClass(cls) -> None:
        cls.temporary.cleanup()

    def test_every_passage_at_every_level_and_the_noise_only_clips_pass_the_verifier(self) -> None:
        passages = len(MANIFEST["passages"])
        self.assertEqual(len(self.report["clips"]), passages * 4 + 3)
        by_clip = {check["clip"]: check for check in self.report["clips"]}
        estimates = [by_clip[f"EN01-{level}"]["energy"]["estimated_snr_db"] for level in ("snr20", "snr10", "snr05")]
        self.assertEqual(estimates, sorted(estimates, reverse=True))
        self.assertIn("first_active_us", by_clip["EN01-clean"]["energy"])
        self.assertIn("noise_power_relative_db", by_clip["noise-only-snr05"]["energy"])
        for check in self.report["clips"]:
            self.assertLess(check["duration_us"], 30_000_000, check["clip"])

    def test_a_second_assembly_produces_identical_bytes(self) -> None:
        verifier.verify(str(FFMPEG), str(FFPROBE), self.second, self.first / noisy.PROVENANCE_NAME)

    def test_the_recorded_command_templates_hold_no_local_paths(self) -> None:
        for clip in self.record["assembly"]["clips"]:
            template = " ".join(clip["command_template"])
            self.assertNotIn(self.temporary.name, template)
            self.assertEqual(clip["command_template"][0], "<FFMPEG>")

    def test_the_corpus_and_its_speech_variants_are_untouched(self) -> None:
        record = recipe.load_json(recipe.GENERATED / recipe.PROVENANCE_NAME)
        for variant in record["assembly"]["variants"]:
            self.assertEqual(recipe.sha256_file(recipe.GENERATED / variant["file"]), variant["sha256"])

    def test_assembly_refuses_to_run_twice_into_one_directory(self) -> None:
        with self.assertRaises(recipe.GenerationError):
            noisy.assemble(self.first, str(FFMPEG))

    def test_release_assets_are_recorded_and_the_clips_rebuild_from_them(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            committed = Path(temporary) / "committed"
            shutil.copytree(self.first, committed)
            release = noisy.release_assets(committed, Path(temporary) / "assets")
            self.assertEqual(release["tag"], noisy.RELEASE_TAG)
            self.assertEqual(sorted(path.name for path in (Path(temporary) / "assets").iterdir()),
                             sorted(release["assets"].values()))
            with self.assertRaises(recipe.GenerationError):
                noisy.release_assets(committed, Path(temporary) / "again")
            # What a fetch would write: speech/<passage>-utterance.wav, outside the clips.
            fetched = Path(temporary) / "fetched"
            shutil.copytree(committed / "speech", fetched / "speech")
            shutil.rmtree(committed / "speech")
            clips = noisy.rebuild(fetched, Path(temporary) / "rebuilt", str(FFMPEG),
                                  committed / noisy.PROVENANCE_NAME)
            self.assertEqual(len(clips), len(MANIFEST["passages"]) * 4 + 3)
            report = verifier.verify(str(FFMPEG), str(FFPROBE), committed, committed / noisy.PROVENANCE_NAME,
                                     fetched)
            self.assertEqual(len(report["utterances"]), len(MANIFEST["passages"]))
            with self.assertRaises(verifier.VerificationError):
                verifier.verify(str(FFMPEG), str(FFPROBE), committed, committed / noisy.PROVENANCE_NAME)

            record_path = committed / noisy.PROVENANCE_NAME
            record = json.loads(record_path.read_text(encoding="utf-8"))
            record["assembly"]["clips"][0]["sha256"] = "0" * 64
            record_path.write_text(json.dumps(record), encoding="utf-8")
            with self.assertRaisesRegex(recipe.GenerationError, "EN01-clean"):
                noisy.rebuild(fetched, Path(temporary) / "rebuilt-again", str(FFMPEG), record_path)

    def test_tampering_and_a_wrong_level_fail_verification(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            tampered = Path(temporary) / "tampered"
            shutil.copytree(self.first, tampered)
            clip = tampered / "ES02-snr10.m4a"
            data = bytearray(clip.read_bytes())
            data[-1] ^= 0x01
            clip.write_bytes(bytes(data))
            with self.assertRaises(verifier.VerificationError):
                verifier.verify(str(FFMPEG), str(FFPROBE), tampered, tampered / noisy.PROVENANCE_NAME)

        with tempfile.TemporaryDirectory() as temporary:
            swapped = Path(temporary) / "swapped"
            shutil.copytree(self.first, swapped)
            # The 5 dB clip's bytes under the 20 dB name, with the record made to agree:
            # only the decoded audio shows that the level is wrong.
            shutil.copyfile(swapped / "EN03-snr05.m4a", swapped / "EN03-snr20.m4a")
            record_path = swapped / noisy.PROVENANCE_NAME
            record = json.loads(record_path.read_text(encoding="utf-8"))
            five = next(clip for clip in record["assembly"]["clips"] if clip["clip"] == "EN03-snr05")
            twenty = next(clip for clip in record["assembly"]["clips"] if clip["clip"] == "EN03-snr20")
            twenty.update(sha256=five["sha256"], bytes=five["bytes"])
            record_path.write_text(json.dumps(record), encoding="utf-8")
            with self.assertRaisesRegex(verifier.VerificationError, "EN03-snr20"):
                verifier.verify(str(FFMPEG), str(FFPROBE), swapped, record_path)


if __name__ == "__main__":
    unittest.main()
