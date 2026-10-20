"""Generate the P07 noisy-speech set (issue #150) from its frozen scripts.

Test-fixture tooling only, like `tools/generate_p07_speech.py`, whose reviewed pieces it
reuses unchanged: the pinned Kokoro revision and voices (`fetch`, `KokoroSynthesizer`),
the quantizer, the WAV writer and the `office-v1` noise recipe. Kokoro and PyTorch run
only on the disposable runner of `.github/workflows/p07-speech-fixtures.yml`
(`fixture_set: noisy-speech`); the maintainer's machine needs neither.

The set has its own truth file, `fixtures/corpus/noisy-speech/manifest.json`, so the
corpus manifest and `generated/speech-provenance.json` (the agent trials' frozen
`truth`) never change because of it.

``synthesize``  Speak every passage once, sentence by sentence, and write one clean
                utterance WAV per passage plus the synthesis record.
``assemble``    Place each utterance after a fixed noise-only lead-in and mix it at
                every level of the manifest: digital silence for ``clean``, `office-v1`
                noise at 20, 10 and 5 dB for the others, with the same noise sequence
                for one passage at every level. Add one noise-only clip per noisy
                level. Encode each track as audio-only AAC in MP4 with FFmpeg.
``review``      Print what a reviewer reads before committing: durations, clipping,
                phonemes and file sizes.
``release-assets``  Record where the utterances live in the test-media release and copy
                them, under their asset names, into a folder for a person to upload. The
                WAVs are not committed (maintainer decision, 2026-10-20).
``rebuild``     Rebuild every clip from utterances fetched from the release
                (`tools/fetch_p07_noisy_utterances.py`) and require the committed bytes.

Every clip is shorter than one 30 s R0 recognition window, so a measured error comes
from the noise and the recogniser, never from a chunk seam.
"""

from __future__ import annotations

import argparse
import array
from dataclasses import dataclass
import json
import math
import os
from pathlib import Path
import platform
import shutil
import sys
import tempfile
from typing import Sequence

import generate_p07_speech as recipe


SET_DIRECTORY = recipe.CORPUS / "noisy-speech"
MANIFEST = SET_DIRECTORY / "manifest.json"
GENERATED = SET_DIRECTORY / "generated"

GENERATOR_ID = "tools/generate_p07_noisy_speech.py v1"
PROVENANCE_NAME = "noisy-speech-provenance.json"
UTTERANCE_DIRECTORY = "speech"
SET_ID = "vsift-noisy-speech-v1"

SAMPLE_RATE = recipe.SAMPLE_RATE
MICROSECONDS = recipe.MICROSECONDS
SPEED = 1.0
# Sentences are spoken one at a time, so no request reaches Kokoro's 510-phoneme limit,
# beyond which its non-English pipeline truncates silently; a longer phoneme string
# stops generation instead.
MAX_PHONEMES = 510
INTER_SENTENCE_PAUSE_US = 300_000

# Placement policy (generation, not truth): a noise-only lead-in and tail around the
# utterance, and a whole clip inside one 30 s R0 recognition window.
LEAD_IN_US = 2_000_000
TAIL_US = 1_000_000
MAX_CLIP_US = 29_000_000

# Noise seeds: the passage at manifest position n (from 1) uses PASSAGE_SEED_BASE + n at
# every level; the noise-only clips share NOISE_ONLY_SEED. F08 keeps the recipe's 408.
PASSAGE_SEED_BASE = 150_000
NOISE_ONLY_SEED = 150_999

# The power ratio of each reviewed level. Only +, *, / and sqrt are used, as in
# `office-v1`, so a mix is bit-identical on every platform: 5 dB is sqrt(10).
POWER_RATIOS = {20: 100.0, 10: 10.0, 5: math.sqrt(10.0)}
CLEAN_LEVEL = "clean"
LANGUAGES = frozenset(recipe.VOICES)

# The utterances live in this test-media release, not in the repository (maintainer
# decision, 2026-10-20); the clips and the measurement stay committed.
RELEASE_TAG = "test-media-2026-10-19"
RELEASE_BASE_URL = f"https://github.com/smormah/vsift/releases/download/{RELEASE_TAG}/"
COMMITTED_PROVENANCE = GENERATED / PROVENANCE_NAME


class NoisySetError(recipe.GenerationError):
    """A reviewed precondition of the noisy-speech set does not hold."""


@dataclass(frozen=True)
class Level:
    """One level of the set; `snr_db` is None for the clean level."""

    level_id: str
    snr_db: int | None

    @property
    def power_ratio(self) -> float | None:
        return None if self.snr_db is None else POWER_RATIOS[self.snr_db]


@dataclass(frozen=True)
class Passage:
    """One frozen passage: its manifest position, language and sentences."""

    passage_id: str
    position: int
    language: str
    script: str
    sentences: tuple[str, ...]

    @property
    def noise_seed(self) -> int:
        return PASSAGE_SEED_BASE + self.position


@dataclass(frozen=True)
class SetPlan:
    """Everything the manifest fixes for the set."""

    passages: tuple[Passage, ...]
    levels: tuple[Level, ...]
    noise_only_us: int

    @property
    def noisy_levels(self) -> tuple[Level, ...]:
        return tuple(level for level in self.levels if level.snr_db is not None)


def load_manifest(path: Path = MANIFEST) -> dict:
    return recipe.load_json(path)


def set_plan(manifest: dict) -> SetPlan:
    """Read and check the set's truth; anything unreviewed stops generation."""
    if manifest.get("set_id") != SET_ID or manifest.get("noise_recipe") != recipe.NOISE_RECIPE:
        raise NoisySetError("the manifest is not the reviewed noisy-speech set over office-v1")
    levels = []
    for entry in manifest["levels"]:
        snr = entry["snr_db"]
        if (snr is None) != (entry["id"] == CLEAN_LEVEL) or (snr is not None and snr not in POWER_RATIOS):
            raise NoisySetError(f"level {entry['id']} has an unreviewed signal-to-noise ratio")
        levels.append(Level(entry["id"], snr))
    if len({level.level_id for level in levels}) != len(levels) or levels[0].snr_db is not None:
        raise NoisySetError("levels must be unique and start with the clean level")
    passages = []
    for position, entry in enumerate(manifest["passages"], start=1):
        language = entry["language"]
        if language not in LANGUAGES:
            raise NoisySetError(f"{entry['id']} names unreviewed language {language}")
        script = entry["script"]
        if not script.strip():
            raise NoisySetError(f"{entry['id']} has an empty script")
        passages.append(Passage(entry["id"], position, language, script, tuple(recipe.split_sentences(script))))
    if len({passage.passage_id for passage in passages}) != len(passages):
        raise NoisySetError("passage ids must be unique")
    noise_only = manifest["noise_only"]["duration_us"]
    if not 0 < noise_only <= MAX_CLIP_US:
        raise NoisySetError("the noise-only clip must fit one recognition window")
    recipe.exact_frames(noise_only)
    return SetPlan(tuple(passages), tuple(levels), noise_only)


def utterance_path(passage_id: str) -> str:
    return f"{UTTERANCE_DIRECTORY}/{passage_id}-utterance.wav"


def clip_name(subject: str, level_id: str) -> str:
    return f"{subject}-{level_id}.m4a"


def track_frames_for(utterance_frames: int) -> int:
    return recipe.exact_frames(LEAD_IN_US) + utterance_frames + recipe.exact_frames(TAIL_US)


def check_fits(passage: Passage, utterance_frames: int) -> None:
    clip_us = recipe.frames_to_us_ceiling(track_frames_for(utterance_frames))
    if clip_us > MAX_CLIP_US:
        raise NoisySetError(
            f"{passage.passage_id} clip ({clip_us} us) is longer than {MAX_CLIP_US} us; a shorter "
            "passage is a reviewed truth change made before the set is first committed")


def synthesize_passage(passage: Passage, synthesizer: recipe.Synthesizer) -> tuple[array.array, int, list[dict]]:
    """Speak each sentence with the seed reset, joined by a fixed pause."""
    voice = recipe.VOICES[passage.language]
    pause = recipe.exact_frames(INTER_SENTENCE_PAUSE_US)
    samples: list[float] = []
    records: list[dict] = []
    for index, sentence in enumerate(passage.sentences):
        if index:
            samples.extend([0.0] * pause)
        result = synthesizer.synthesize(sentence, voice, SPEED)
        if not result.samples:
            raise NoisySetError(f"{passage.passage_id} sentence {index} produced no audio")
        too_long = [phonemes for phonemes in result.phonemes if len(phonemes) >= MAX_PHONEMES]
        if too_long:
            raise NoisySetError(f"{passage.passage_id} sentence {index} reached Kokoro's phoneme limit")
        start_frame = len(samples)
        samples.extend(result.samples)
        records.append({
            "text": sentence,
            "engine_text": sentence,
            "start_frame": start_frame,
            "frames": len(result.samples),
            "phonemes": list(result.phonemes),
            "words": None if result.words is None else [
                {"text": word.text, "start_us": word.start_us, "end_us": word.end_us} for word in result.words],
        })
    utterance, clipped = recipe.quantize(samples)
    if not any(utterance):
        raise NoisySetError(f"{passage.passage_id} utterance quantized to silence")
    return utterance, clipped, records


def synthesize(output: Path, synthesizer: recipe.Synthesizer, synthesis_facts: dict,
               manifest_path: Path = MANIFEST) -> dict:
    """Write every passage's utterance and a record holding the synthesis section."""
    plan = set_plan(load_manifest(manifest_path))
    (output / UTTERANCE_DIRECTORY).mkdir(parents=True, exist_ok=True)
    spoken = []
    misfits = []
    for passage in plan.passages:
        utterance, clipped, sentences = synthesize_passage(passage, synthesizer)
        try:
            check_fits(passage, len(utterance))
        except NoisySetError as error:
            misfits.append(str(error))
        spoken.append((passage, utterance, clipped, sentences))
    if misfits:  # Fail before writing anything, naming every passage that does not fit.
        raise NoisySetError("; ".join(misfits))
    utterances = []
    for passage, utterance, clipped, sentences in spoken:
        relative = utterance_path(passage.passage_id)
        recipe.write_wav(output / relative, utterance)
        voice = recipe.VOICES[passage.language]
        utterances.append({
            "passage": passage.passage_id, "language": passage.language, "voice": voice.voice_id,
            "speed": SPEED, "file": relative, "sha256": recipe.sha256_file(output / relative),
            "bytes": (output / relative).stat().st_size, "frames": len(utterance),
            "clipped_samples": clipped, "sentences": sentences,
        })
    record = {
        "generator": GENERATOR_ID,
        "purpose": "Test fixtures only. Kokoro is never a VSift runtime dependency.",
        "set_id": SET_ID,
        "manifest_sha256": recipe.sha256_file(manifest_path),
        "synthesis": dict(synthesis_facts, utterances=utterances),
        "assembly": None,
    }
    with (output / PROVENANCE_NAME).open("xb") as handle:
        handle.write(recipe.json_bytes(record))
    return record


def pooled_rms(utterances: Sequence[array.array]) -> float:
    """RMS of every utterance sample together: the speech level noise-only clips use."""
    total = 0
    squares = []
    for utterance in utterances:
        total += len(utterance)
        squares.append(math.fsum((sample / recipe.INT16_SCALE) ** 2 for sample in utterance))
    return math.sqrt(math.fsum(squares) / total)


def mix(speech: list[float], start: int, frames: int, speech_level: float, power_ratio: float,
        seed: int) -> tuple[list[float], dict]:
    """Scale `office-v1` noise to `power_ratio` below `speech_level`, add the speech, limit."""
    noise = recipe.office_noise(frames, seed)
    noise_level = recipe.rms(noise)
    noise_gain = speech_level / (math.sqrt(power_ratio) * noise_level)
    mixed = [value * noise_gain for value in noise]
    for index, value in enumerate(speech):
        mixed[start + index] += value
    peak = max(abs(value) for value in mixed)
    mix_gain = 1.0 if peak <= 1.0 else 1.0 / peak
    if mix_gain != 1.0:
        mixed = [value * mix_gain for value in mixed]
    return mixed, {
        "recipe": recipe.NOISE_RECIPE,
        "seed": seed,
        "power_ratio": power_ratio,
        "speech_rms": speech_level,
        "unscaled_noise_rms": noise_level,
        "noise_gain": noise_gain,
        "mix_gain": mix_gain,
        "covers": "whole clip",
    }


def passage_track(utterance: array.array, level: Level, seed: int) -> tuple[array.array, dict | None]:
    """The clip's track: lead-in, utterance and tail, clean or mixed at `level`."""
    start = recipe.exact_frames(LEAD_IN_US)
    frames = track_frames_for(len(utterance))
    if level.power_ratio is None:
        track = array.array("h", bytes(2 * frames))
        track[start:start + len(utterance)] = utterance
        return track, None
    speech = [sample / recipe.INT16_SCALE for sample in utterance]
    speech_level = recipe.rms(speech)
    if speech_level == 0.0:
        raise NoisySetError("an utterance is silent")
    mixed, record = mix(speech, start, frames, speech_level, level.power_ratio, seed)
    track, clipped = recipe.quantize(mixed)
    if clipped:
        raise NoisySetError("a noisy mix clipped after peak limiting")
    return track, dict(record, snr_db=level.snr_db)


def noise_only_track(frames: int, level: Level, reference_level: float) -> tuple[array.array, dict]:
    """Noise alone, at the level it has under speech of the set's pooled RMS."""
    if level.power_ratio is None:
        raise NoisySetError("the clean level has no noise-only clip")
    mixed, record = mix([], 0, frames, reference_level, level.power_ratio, NOISE_ONLY_SEED)
    track, clipped = recipe.quantize(mixed)
    if clipped:
        raise NoisySetError("a noise-only clip clipped after peak limiting")
    record["speech_rms_definition"] = "pooled RMS of every utterance of the set"
    return track, dict(record, snr_db=level.snr_db)


def encode_argv(ffmpeg: str, track: str, output: str) -> list[str]:
    """Encode a placed track as audio-only AAC in MP4, as the corpus's AAC tracks are."""
    return [ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-n", "-i", track,
            "-map", "0:a:0", "-c:a", "aac", "-b:a", "64k", "-ar", str(recipe.OUTPUT_AUDIO_RATE), "-ac", "1",
            "-fflags", "+bitexact", "-movflags", "+faststart", output]


def assemble(output: Path, ffmpeg: str, manifest_path: Path = MANIFEST) -> dict:
    """Place, mix and encode every clip of every level from the recorded utterances."""
    plan = set_plan(load_manifest(manifest_path))
    provenance_path = output / PROVENANCE_NAME
    record = recipe.load_json(provenance_path)
    if record["manifest_sha256"] != recipe.sha256_file(manifest_path):
        raise NoisySetError("synthesis record was made from different truth")
    if record.get("assembly") is not None:
        raise NoisySetError("assembly already recorded; assemble into a fresh output directory")
    entries = {entry["passage"]: entry for entry in record["synthesis"]["utterances"]}
    if list(entries) != [passage.passage_id for passage in plan.passages]:
        raise NoisySetError("synthesis record does not cover exactly the manifest's passages")
    utterances = {}
    for passage in plan.passages:
        entry = entries[passage.passage_id]
        if entry["file"] != utterance_path(passage.passage_id):
            raise NoisySetError(f"{passage.passage_id} record names an unexpected utterance path")
        if [sentence["text"] for sentence in entry["sentences"]] != list(passage.sentences):
            raise NoisySetError(f"{passage.passage_id} synthesized text differs from the frozen script")
        path = output / entry["file"]
        if recipe.sha256_file(path) != entry["sha256"]:
            raise NoisySetError(f"{passage.passage_id} utterance differs from its synthesis record")
        utterances[passage.passage_id] = recipe.read_wav(path)
        check_fits(passage, len(utterances[passage.passage_id]))
    reference_level = pooled_rms(list(utterances.values()))
    clips = []
    with tempfile.TemporaryDirectory(prefix="vsift-p07-noisy-") as temporary:
        def encode(name: str, track: array.array) -> dict:
            track_path = Path(temporary) / f"{name}.wav"
            recipe.write_wav(track_path, track)
            recipe.run_ffmpeg(encode_argv(ffmpeg, str(track_path), str(output / name)))
            return {"file": name, "sha256": recipe.sha256_file(output / name),
                    "bytes": (output / name).stat().st_size, "track_frames": len(track),
                    "track_sha256": recipe.sha256_file(track_path),
                    "command_template": encode_argv("<FFMPEG>", "<TRACK>", f"<OUTPUT>/{name}")}

        for passage in plan.passages:
            utterance = utterances[passage.passage_id]
            speech_start_us = LEAD_IN_US
            speech_end_us = LEAD_IN_US + recipe.frames_to_us_ceiling(len(utterance))
            for level in plan.levels:
                track, noise = passage_track(utterance, level, passage.noise_seed)
                clip = encode(clip_name(passage.passage_id, level.level_id), track)
                clips.append(dict({"clip": f"{passage.passage_id}-{level.level_id}", "passage": passage.passage_id,
                                   "language": passage.language, "level": level.level_id, "snr_db": level.snr_db,
                                   "speech_start_us": speech_start_us, "speech_end_us": speech_end_us,
                                   "noise": noise}, **clip))
        frames = recipe.exact_frames(plan.noise_only_us)
        for level in plan.noisy_levels:
            track, noise = noise_only_track(frames, level, reference_level)
            clip = encode(clip_name("noise-only", level.level_id), track)
            clips.append(dict({"clip": f"noise-only-{level.level_id}", "passage": None, "language": None,
                               "level": level.level_id, "snr_db": level.snr_db, "speech_start_us": None,
                               "speech_end_us": None, "noise": noise}, **clip))
    record["assembly"] = {
        "ffmpeg_version": recipe.ffmpeg_version(ffmpeg),
        "ffmpeg_sha256": recipe.sha256_file(Path(ffmpeg)),
        "track_sample_rate": SAMPLE_RATE,
        "encoded_sample_rate": recipe.OUTPUT_AUDIO_RATE,
        "placement_policy": {"lead_in_us": LEAD_IN_US, "tail_us": TAIL_US, "max_clip_us": MAX_CLIP_US,
                             "note": "every clip fits one 30 s R0 recognition window"},
        "noise_seeds": {"passage_seed_base": PASSAGE_SEED_BASE, "noise_only_seed": NOISE_ONLY_SEED,
                        "rule": "the passage at manifest position n (from 1) uses base + n at every level"},
        "reference_speech_rms": reference_level,
        "timing_note": ("speech_start_us/speech_end_us are exact by construction; the sentences' word timings "
                        "are Kokoro's own (English only) and are generation facts, not truth"),
        "clips": clips,
    }
    replacement = provenance_path.with_suffix(".json.tmp")
    with replacement.open("xb") as handle:
        handle.write(recipe.json_bytes(record))
    os.replace(replacement, provenance_path)
    return record


def release_asset_name(passage_id: str) -> str:
    return f"noisy-speech-{passage_id}-utterance.wav"


def release_assets(output: Path, assets_dir: Path) -> dict:
    """Record the utterances' release assets in the provenance and copy them for upload.

    `assets_dir` must not exist. Each copy is checked against the recorded SHA-256, so
    what a person uploads is exactly what the record pins.
    """
    provenance_path = output / PROVENANCE_NAME
    record = recipe.load_json(provenance_path)
    if record.get("assembly") is None or "utterance_release" in record:
        raise NoisySetError("release assets are recorded once, after assembly")
    assets_dir.mkdir(parents=True, exist_ok=False)
    names = {}
    for entry in record["synthesis"]["utterances"]:
        name = release_asset_name(entry["passage"])
        shutil.copyfile(output / entry["file"], assets_dir / name)
        if recipe.sha256_file(assets_dir / name) != entry["sha256"]:
            raise NoisySetError(f"{entry['passage']} utterance differs from its synthesis record")
        names[entry["passage"]] = name
    record["utterance_release"] = {
        "tag": RELEASE_TAG,
        "base_url": RELEASE_BASE_URL,
        "assets": names,
        "note": ("the utterance WAVs are not in the repository (maintainer decision, 2026-10-20); "
                 "tools/fetch_p07_noisy_utterances.py fetches them by the bytes and SHA-256 recorded "
                 "under synthesis.utterances, for the rebuild and the verifier"),
    }
    replacement = provenance_path.with_suffix(".json.tmp")
    with replacement.open("xb") as handle:
        handle.write(recipe.json_bytes(record))
    os.replace(replacement, provenance_path)
    return record["utterance_release"]


def rebuild(utterances_dir: Path, output: Path, ffmpeg: str,
            committed_provenance: Path = COMMITTED_PROVENANCE, manifest_path: Path = MANIFEST) -> list[str]:
    """Rebuild every clip from fetched utterances and require the committed bytes.

    `utterances_dir` holds `speech/<passage>-utterance.wav`; `output` must not exist. The
    committed clips are bit-identical only with the same FFmpeg build (the workflow uses
    the pinned P06 build), so a mismatch names the clips and the build.
    """
    committed = recipe.load_json(committed_provenance)
    if committed.get("assembly") is None:
        raise NoisySetError("the committed record has no assembly to compare with")
    (output / UTTERANCE_DIRECTORY).mkdir(parents=True, exist_ok=False)
    for entry in committed["synthesis"]["utterances"]:
        source = utterances_dir / entry["file"]
        if recipe.sha256_file(source) != entry["sha256"]:
            raise NoisySetError(f"{entry['passage']} fetched utterance differs from its recorded SHA-256")
        shutil.copyfile(source, output / entry["file"])
    with (output / PROVENANCE_NAME).open("xb") as handle:
        handle.write(recipe.json_bytes(dict(committed, assembly=None)))
    rebuilt = assemble(output, ffmpeg, manifest_path)
    expected = {clip["clip"]: clip for clip in committed["assembly"]["clips"]}
    differing = [clip["clip"] for clip in rebuilt["assembly"]["clips"]
                 if any(clip[key] != expected.get(clip["clip"], {}).get(key)
                        for key in ("file", "sha256", "bytes", "track_sha256"))]
    if differing or len(expected) != len(rebuilt["assembly"]["clips"]):
        raise NoisySetError(f"rebuilt clips differ from the committed ones ({', '.join(differing)}); "
                            f"FFmpeg {rebuilt['assembly']['ffmpeg_version']}")
    return sorted(expected)


def synthesis_facts(assets: Path, reports: Sequence[Path]) -> dict:
    """The engine, voices and pinned environment, as the corpus recipe records them."""
    fetch_record = recipe.load_json(assets / recipe.FETCH_RECORD_NAME)
    return {
        "engine": "Kokoro text-to-speech (test fixture generation only)",
        "model": {"repository": recipe.KOKORO_REPOSITORY, "revision": recipe.KOKORO_REVISION,
                  "licence": "Apache-2.0",
                  "files": [entry for entry in fetch_record["files"]
                            if entry["path"].startswith(recipe.KOKORO_DIRECTORY)]},
        "voices": {language: {"voice": voice.voice_id, "kokoro_lang_code": voice.lang_code}
                   for language, voice in recipe.VOICES.items()},
        "speed": SPEED,
        "seed": recipe.SEED,
        "seed_policy": "torch.manual_seed reset before every sentence",
        "inter_sentence_pause_us": INTER_SENTENCE_PAUSE_US,
        "torch_threads": recipe.TORCH_THREADS,
        "deterministic_algorithms": True,
        "sample_rate": SAMPLE_RATE,
        "sample_format": "mono signed 16-bit little-endian PCM WAV, round(x * 32767)",
        "environment": {
            "python": platform.python_version(),
            "platform": platform.platform(),
            "machine": platform.machine(),
            "cpu": recipe.cpu_model(),
            "distributions": recipe.installed_distributions(),
            "espeak_ng": recipe.espeak_version(),
            "spacy_model_wheel": next(entry for entry in fetch_record["files"]
                                      if entry["path"] == recipe.SPACY_MODEL_WHEEL.path),
        },
        "resolved_distributions": recipe.resolved_distributions(reports),
    }


def review(output: Path) -> str:
    """A plain-text summary for the reviewer: printed to the job log and its summary."""
    record = recipe.load_json(output / PROVENANCE_NAME)
    lines = ["# Noisy-speech set review", "", "| Passage | Language | Seconds | Clipped | Phonemes |",
             "| --- | --- | ---: | ---: | --- |"]
    for entry in record["synthesis"]["utterances"]:
        phonemes = " / ".join(" ".join(sentence["phonemes"]) for sentence in entry["sentences"])
        lines.append(f"| {entry['passage']} | {entry['language']} | {entry['frames'] / SAMPLE_RATE:.2f} | "
                     f"{entry['clipped_samples']} | {phonemes} |")
    if record.get("assembly") is not None:
        clips = record["assembly"]["clips"]
        total = sum(clip["bytes"] for clip in clips)
        utterance_bytes = sum(entry["bytes"] for entry in record["synthesis"]["utterances"])
        lines += ["", f"Clips: {len(clips)}, {total} bytes; utterances: {utterance_bytes} bytes.", "",
                  "| Clip | Bytes | Mix gain |", "| --- | ---: | ---: |"]
        for clip in clips:
            gain = "" if clip["noise"] is None else f"{clip['noise']['mix_gain']:.4f}"
            lines.append(f"| {clip['clip']} | {clip['bytes']} | {gain} |")
    return "\n".join(lines) + "\n"


def main(argv: Sequence[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    synthesize_command = commands.add_parser("synthesize", help="speak the frozen passages with Kokoro")
    synthesize_command.add_argument("--assets", required=True, type=Path)
    synthesize_command.add_argument("--output-dir", required=True, type=Path)
    synthesize_command.add_argument("--pip-report", action="append", default=[], type=Path)
    assemble_command = commands.add_parser("assemble", help="place, mix and encode every clip")
    assemble_command.add_argument("--output-dir", required=True, type=Path)
    assemble_command.add_argument("--ffmpeg", required=True, help="explicit trusted FFmpeg executable path")
    review_command = commands.add_parser("review", help="print the reviewer's summary")
    review_command.add_argument("--output-dir", required=True, type=Path)
    release_command = commands.add_parser("release-assets", help="record and copy the utterances' release assets")
    release_command.add_argument("--output-dir", required=True, type=Path)
    release_command.add_argument("--assets-dir", required=True, type=Path, help="a folder that does not exist yet")
    rebuild_command = commands.add_parser("rebuild", help="rebuild the clips from fetched utterances")
    rebuild_command.add_argument("--utterances-dir", required=True, type=Path)
    rebuild_command.add_argument("--output-dir", required=True, type=Path, help="a folder that does not exist yet")
    rebuild_command.add_argument("--ffmpeg", required=True, help="explicit trusted FFmpeg executable path")
    args = parser.parse_args(argv)
    if args.command == "synthesize":
        assets = args.assets.resolve(strict=True)
        facts = synthesis_facts(assets, args.pip_report)  # Checks the pinned environment first.
        record = synthesize(args.output_dir.resolve(), recipe.KokoroSynthesizer(assets), facts)
        print(f"Synthesized {len(record['synthesis']['utterances'])} passages")
    elif args.command == "assemble":
        ffmpeg = str(Path(args.ffmpeg).resolve(strict=True))
        record = assemble(args.output_dir.resolve(strict=True), ffmpeg)
        print(f"Assembled {len(record['assembly']['clips'])} clips")
    elif args.command == "release-assets":
        release = release_assets(args.output_dir.resolve(strict=True), args.assets_dir.resolve())
        print(f"Recorded {len(release['assets'])} release assets of {release['tag']}")
    elif args.command == "rebuild":
        clips = rebuild(args.utterances_dir.resolve(strict=True), args.output_dir.resolve(),
                        str(Path(args.ffmpeg).resolve(strict=True)))
        print(f"Rebuilt {len(clips)} clips, identical to the committed ones")
    else:
        sys.stdout.write(review(args.output_dir.resolve(strict=True)))


if __name__ == "__main__":
    main()
