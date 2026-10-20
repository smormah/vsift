"""Evaluate speaker-diarization engines for VSift's speaker labels (0.3.0) on a hosted runner.

Evaluation tooling only. Nothing here is a VSift runtime dependency, and no engine or
model it stages is shipped or named in a Cargo manifest. It runs on a disposable GitHub
Actions runner (`.github/workflows/p16-speakers-eval.yml`); the maintainer's machine
downloads none of it. The design, the candidates and the results are
`docs/planning/speaker-labels-evaluation.md`.

The stages:

``fetch-voices``  Download the extra pinned Kokoro voices the conversations use, next to
                  the assets of `generate_p07_speech.py fetch` (same revision, same rules).
``synthesize``    Speak every turn of the frozen conversations below with Kokoro, one
                  utterance WAV per distinct (voice, text), plus the synthesis record.
                  Needs the pinned P07 generation environment.
``assemble``      Lay the turns on each conversation's timeline (the gaps and overlaps
                  below), add `office-v1` noise to the noisy variant, resample to 16 kHz
                  mono PCM with FFmpeg and write the exact truth: who speaks when, from
                  sample positions, never from listening. Standard library plus FFmpeg.
``media``         Extract 16 kHz mono PCM from the real-media items the workflow fetched.
``stage``         Download the pinned engines and models for this platform, each bounded
                  by its reviewed size and checked against its published SHA-256 (or
                  recorded on first use where the publisher gives none), and extract
                  archives member by member without links or paths that leave the folder.
``evaluate``      Run every candidate configuration on every input, measuring wall time,
                  CPU time and peak memory, transcribe the inputs with the pinned
                  whisper.cpp, and score: diarization error rate against exact truth, the
                  speaker count found, and the segment labelling VSift would show
                  (a label only where one speaker clearly holds the segment, otherwise
                  "no confident speaker"), with how much it covers and how often it is wrong.

No shell is invoked: every external program is started with an explicit argument list.
The real recordings are public-domain NASA items of the project's own test-media release;
only numbers and, for RM06 (public domain), its labelled transcript are kept.
"""

from __future__ import annotations

import argparse
import array
from dataclasses import dataclass
import hashlib
import itertools
import json
import math
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import threading
import time
from typing import Iterable, Mapping, Sequence
import wave

import generate_p07_speech as recipe


MICROSECONDS = 1_000_000
SYNTHESIS_RATE = recipe.SAMPLE_RATE  # Kokoro's 24 kHz.
ENGINE_RATE = 16_000  # Every candidate reads 16 kHz mono PCM.
GENERATOR_ID = "tools/p16_speakers_eval.py v1"
SYNTHESIS_RECORD = "speaker-synthesis.json"
UTTERANCES = "utterances"
FIXTURES = "fixtures"

# Placement policy (generation, not truth): silence around each conversation.
LEAD_IN_US = 1_000_000
TAIL_US = 1_000_000
# A turn's truth runs from its first to its last sample at or above this amplitude
# (about -40 dBFS); pauses inside a turn count as that speaker's speech.
TRIM_AMPLITUDE = 0.01
# The mix is scaled so its peak is at most this, then quantised to 16-bit PCM.
MIX_PEAK = 0.8
NOISE_SEED = 160_016
NOISY_SNR_DB = 10

# Scoring policy (NIST-style): a 250 ms collar around every true turn boundary is not
# scored, overlapping speech is scored, frames are 10 ms.
COLLAR_US = 250_000
FRAME_US = 10_000
# RM06's reference is labelled by reading the transcript, at segment precision, so its
# collar is wider.
REFERENCE_COLLAR_US = 500_000
# VSift's proposed labelling rule for one transcript segment: some speaker's speech
# covers at least half the segment, and one speaker holds at least 70% of that speech.
LABEL_MIN_SPEECH_SHARE = 0.5
LABEL_MIN_DOMINANT_SHARE = 0.7

ENGINE_TIMEOUT_S = 1_800
THREADS = 2


class EvaluationError(Exception):
    """A reviewed precondition of the evaluation does not hold."""


# ---------------------------------------------------------------------------
# Voices and conversations (the truth source of the synthetic fixtures)
# ---------------------------------------------------------------------------

def _voice(voice_id: str, lang_code: str, size: int, sha256: str) -> recipe.Voice:
    """A Kokoro voice of the reviewed revision, keyed by its own id."""
    language = "en-US" if lang_code == "a" else "en-GB"
    pin = recipe.PinnedFile(f"{recipe.KOKORO_DIRECTORY}/voices/{voice_id}.pt",
                            f"{recipe.KOKORO_BASE_URL}voices/{voice_id}.pt", size, sha256)
    return recipe.Voice(f"{language}/{voice_id}", lang_code, voice_id, pin)


# Sizes and SHA-256 published by Hugging Face (LFS) for revision
# f3ff3571791e39611d31c381e3a41a3af07b4987 of hexgrad/Kokoro-82M (Apache-2.0).
VOICES = {
    "af_heart": recipe.Voice("en-US/af_heart", "a", "af_heart", recipe.VOICES["en-US"].file),
    "af_bella": _voice("af_bella", "a", 523_425,
                       "8cb64e02fcc8de0327a8e13817e49c76c945ecf0052ceac97d3081480e8e48d6"),
    "am_michael": _voice("am_michael", "a", 523_435,
                         "9a443b79a4b22489a5b0ab7c651a0bcd1a30bef675c28333f06971abbd47bd37"),
    "bf_emma": _voice("bf_emma", "b", 523_420,
                      "d0a423deabf4a52b4f49318c51742c54e21bb89bbbe9a12141e7758ddb5da701"),
    "bm_george": _voice("bm_george", "b", 523_430,
                        "f1bc812213dc59774769e5c80004b13eeb79bd78130b11b2d7f934542dab811b"),
}
EXTRA_VOICE_FILES = tuple(voice.file for key, voice in VOICES.items() if key != "af_heart")


@dataclass(frozen=True)
class Turn:
    """One turn: who speaks, what, and the gap after the previous turn ends.

    A negative gap starts the turn before the previous one ends (overlapping speech).
    """

    speaker: str
    text: str
    gap_ms: int = 600


SUPPORT_CALL = (
    Turn("A", "Hi, thanks for picking up. I have a problem with the export screen, and I recorded it this morning.", 0),
    Turn("B", "No problem. Tell me what happens when you press the export button."),
    Turn("A", "The progress bar fills up to about ninety percent, and then it just stops. Nothing else changes on the screen."),
    Turn("B", "Does it show an error message, or does the window stay open?", 400),
    Turn("A", "The window stays open. There is no message at all."),
    Turn("B", "Okay.", 300),
    Turn("B", "And which version are you running? It is shown in the bottom corner of the settings page.", 900),
    Turn("A", "It says version four point two, build seventeen."),
    Turn("B", "Right. That build had a known issue with large files. How big is the project you are exporting?", 500),
    Turn("A", "About two gigabytes, with a lot of video clips in it."),
    Turn("B", "That explains it.", -400),
    Turn("A", "Is there a way around it?", 300),
    Turn("B", "Yes. You can split the export into two parts, or update to build eighteen, which fixes the memory limit.", 1200),
    Turn("A", "I will try the update first. Do I need to restart the computer afterwards?"),
    Turn("B", "No, just close the application and open it again."),
    Turn("A", "Perfect, thank you."),
    Turn("B", "You are welcome.", -250),
    Turn("B", "If it still stops at ninety percent, send me the recording and the log file, and I will look at it today.", 700),
    Turn("A", "Will do. Thanks again, goodbye.", 500),
)

PLANNING_MEETING = (
    Turn("A", "Let's start. The goal today is to agree on the release date and on who tests what.", 0),
    Turn("B", "I can take the installer tests on all three systems, but I need the build by Wednesday."),
    Turn("C", "Wednesday is tight. The last merge is planned for Tuesday evening."),
    Turn("A", "Then let's say Wednesday at noon for the build.", 400),
    Turn("B", "That works for me."),
    Turn("C", "I will also run the long recordings overnight, so we have the timing numbers on Thursday morning.", 800),
    Turn("B", "Can you send me the list of recordings you plan to use?"),
    Turn("C", "Yes, I will put it in the shared folder after this meeting."),
    Turn("A", "Good. What about the documentation? Who reviews the new pages?", 1000),
    Turn("B", "I reviewed half of them yesterday. The rest still need screenshots."),
    Turn("A", "I can make the screenshots this afternoon.", 400),
    Turn("C", "One more thing. The search results changed their order, and two tests depend on it.", 900),
    Turn("A", "Which tests?", -300),
    Turn("C", "The ones for the timeline export. They compare the first three results.", 400),
    Turn("B", "Then we should fix the tests, not the order. The new order is better."),
    Turn("A", "Agreed. Let's do that. So the release is Friday, if the overnight runs are clean.", 700),
    Turn("C", "Friday it is.", 300),
    Turn("B", "Sounds good. I will write the summary and send it to everyone."),
)

WALKTHROUGH = (
    Turn("A", "In this recording I will walk through the new settings page.", 0),
    Turn("A", "On the left there is a list of sections, starting with general settings and ending with privacy.", 900),
    Turn("A", "When I open the privacy section, three switches appear, and all of them are off by default.", 1200),
    Turn("A", "The first switch keeps a history of searches. The second one shares crash reports.", 800),
    Turn("A", "The third switch is new in this version. It keeps the transcript after the window is closed.", 1600),
    Turn("A", "I will turn it on now, and you can see that a confirmation message appears at the top.", 1000),
    Turn("A", "That is everything for this page. Next time I will show the export options.", 1400),
)


@dataclass(frozen=True)
class Conversation:
    """One synthetic fixture: its script, the voice of each speaker, and its noise."""

    fixture_id: str
    turns: tuple[Turn, ...]
    voices: Mapping[str, str]
    snr_db: int | None = None
    description: str = ""


CONVERSATIONS = (
    Conversation("SPK2-MF", SUPPORT_CALL, {"A": "af_heart", "B": "am_michael"},
                 description="two voices, female and male, American; short replies, two overlaps"),
    Conversation("SPK2-FF", SUPPORT_CALL, {"A": "af_heart", "B": "af_bella"},
                 description="the same call with two similar voices (both female, American)"),
    Conversation("SPK2-MF-N10", SUPPORT_CALL, {"A": "af_heart", "B": "am_michael"}, NOISY_SNR_DB,
                 description="SPK2-MF with office-v1 noise at 10 dB"),
    Conversation("SPK3-MFM", PLANNING_MEETING, {"A": "am_michael", "B": "bf_emma", "C": "bm_george"},
                 description="three voices (American male, British female, British male); one overlap"),
    Conversation("SPK1-M", WALKTHROUGH, {"A": "bm_george"},
                 description="one voice with long pauses: a second speaker found here is invented"),
)


def conversation(fixture_id: str) -> Conversation:
    for candidate in CONVERSATIONS:
        if candidate.fixture_id == fixture_id:
            return candidate
    raise EvaluationError(f"no conversation {fixture_id}")


def utterance_key(voice_id: str, text: str) -> str:
    """A stable file name for one (voice, text): the same line is spoken once."""
    digest = hashlib.sha256(f"{voice_id}\n{text}".encode("utf-8")).hexdigest()[:16]
    return f"{voice_id}-{digest}"


def distinct_utterances() -> list[tuple[str, str]]:
    """Every (voice id, text) the conversations need, in first-use order."""
    seen: dict[tuple[str, str], None] = {}
    for item in CONVERSATIONS:
        for turn in item.turns:
            if turn.speaker not in item.voices:
                raise EvaluationError(f"{item.fixture_id} turn names unknown speaker {turn.speaker}")
            seen.setdefault((item.voices[turn.speaker], turn.text), None)
    return list(seen)


# ---------------------------------------------------------------------------
# Synthesis (Kokoro; only on the runner, in the pinned P07 environment)
# ---------------------------------------------------------------------------

def fetch_voices(assets: Path) -> list[dict]:
    """Download the extra voices beside the P07 assets; each is checked against its pin."""
    return [{"path": pin.path, "bytes": pin.size, "sha256": recipe.download(assets, pin)}
            for pin in EXTRA_VOICE_FILES]


class ConversationSynthesizer(recipe.KokoroSynthesizer):
    """The P07 Kokoro adapter, with the conversation voices added to its voice files."""

    def __init__(self, assets: Path) -> None:
        super().__init__(assets)
        for voice in VOICES.values():
            path, _ = recipe.verified_file(assets, voice.file)
            self._voice_paths[voice.language] = path


def synthesize(assets: Path, output_dir: Path, synthesizer: recipe.Synthesizer | None = None) -> dict:
    """Speak each distinct utterance once and record what was spoken."""
    engine = synthesizer if synthesizer is not None else ConversationSynthesizer(assets)
    folder = output_dir / UTTERANCES
    folder.mkdir(parents=True, exist_ok=False)
    records = []
    for voice_id, text in distinct_utterances():
        result = engine.synthesize(text, VOICES[voice_id], recipe.SPEED)
        if not result.samples:
            raise EvaluationError(f"the voice {voice_id} returned no audio")
        pcm, clipped = recipe.quantize(result.samples)
        key = utterance_key(voice_id, text)
        recipe.write_wav(folder / f"{key}.wav", pcm, SYNTHESIS_RATE)
        records.append({"key": key, "voice": voice_id, "text": text, "frames": len(pcm),
                        "clipped_samples": clipped, "sha256": recipe.sha256_file(folder / f"{key}.wav")})
    record = {"generator": GENERATOR_ID, "kokoro_revision": recipe.KOKORO_REVISION,
              "voices": {key: voice.file.sha256 for key, voice in VOICES.items()},
              "utterances": records}
    with (output_dir / SYNTHESIS_RECORD).open("xb") as handle:
        handle.write(recipe.json_bytes(record))
    return record


# ---------------------------------------------------------------------------
# Assembly (standard library plus FFmpeg)
# ---------------------------------------------------------------------------

def frames_for_us(microseconds: int, rate: int = SYNTHESIS_RATE) -> int:
    return microseconds * rate // MICROSECONDS


def us_for_frames(frames: int, rate: int = SYNTHESIS_RATE) -> int:
    return frames * MICROSECONDS // rate


def active_span(samples: Sequence[int], threshold: float = TRIM_AMPLITUDE) -> tuple[int, int]:
    """The first and one-past-last sample at or above the threshold (16-bit PCM)."""
    limit = threshold * recipe.INT16_SCALE
    first = next((index for index, value in enumerate(samples) if abs(value) >= limit), None)
    if first is None:
        raise EvaluationError("an utterance holds no sample above the speech threshold")
    last = next(index for index in range(len(samples) - 1, -1, -1) if abs(samples[index]) >= limit)
    return first, last + 1


@dataclass(frozen=True)
class PlacedTurn:
    """Where one turn's utterance lands, in frames at the synthesis rate."""

    speaker: str
    text: str
    start_frame: int
    utterance: array.array
    speech_start_frame: int
    speech_end_frame: int


def lay_out(item: Conversation, utterances: Mapping[tuple[str, str], array.array]) -> tuple[list[PlacedTurn], int]:
    """Place every turn after the previous one ends (by its gap) and return the length."""
    placed: list[PlacedTurn] = []
    cursor = frames_for_us(LEAD_IN_US)
    end = cursor
    for index, turn in enumerate(item.turns):
        samples = utterances[(item.voices[turn.speaker], turn.text)]
        first, last = active_span(samples)
        if index == 0:
            start = cursor - first
        else:
            # The gap counts from the latest end so far, so a turn after an
            # interjection never starts inside the turn it interrupted.
            latest_end = max(previous.speech_end_frame for previous in placed)
            start = latest_end + frames_for_us(turn.gap_ms * 1_000) - first
        if start < 0:
            raise EvaluationError(f"{item.fixture_id} turn {index + 1} would start before the recording")
        placed.append(PlacedTurn(turn.speaker, turn.text, start, samples, start + first, start + last))
        end = max(end, start + len(samples))
    return placed, end + frames_for_us(TAIL_US)


def mix(placed: Sequence[PlacedTurn], frames: int, snr_db: int | None,
        noise_seed: int = NOISE_SEED) -> tuple[list[float], dict]:
    """Sum the turns, add noise at the SNR over the true speech, and scale to MIX_PEAK."""
    track = [0.0] * frames
    for turn in placed:
        for offset, value in enumerate(turn.utterance):
            track[turn.start_frame + offset] += value / recipe.INT16_SCALE
    record: dict = {"snr_db": snr_db}
    if snr_db is not None:
        speech = [track[index] for turn in placed for index in range(turn.speech_start_frame, turn.speech_end_frame)]
        speech_power = math.fsum(value * value for value in speech) / len(speech)
        noise = recipe.office_noise(frames, noise_seed, SYNTHESIS_RATE)
        noise_power = math.fsum(value * value for value in noise) / frames
        scale = math.sqrt(speech_power / (noise_power * (10.0 ** (snr_db / 10.0))))
        track = [value + sample * scale for value, sample in zip(track, noise)]
        record.update({"noise": recipe.NOISE_RECIPE, "noise_seed": noise_seed})
    peak = max(abs(value) for value in track)
    gain = MIX_PEAK / peak if peak > MIX_PEAK else 1.0
    record["gain"] = gain
    return [value * gain for value in track], record


def merged_truth(placed: Sequence[PlacedTurn]) -> list[dict]:
    """The true turns in start order, in microseconds on the recording's timeline."""
    turns = [{"speaker": turn.speaker, "start_us": us_for_frames(turn.speech_start_frame),
              "end_us": us_for_frames(turn.speech_end_frame), "text": turn.text} for turn in placed]
    return sorted(turns, key=lambda turn: (turn["start_us"], turn["speaker"]))


def resample(ffmpeg: Path, source: Path, destination: Path) -> None:
    """Convert to 16 kHz mono 16-bit PCM WAV with an explicit argument list."""
    command = [str(ffmpeg), "-nostdin", "-hide_banner", "-loglevel", "error", "-i", str(source),
               "-vn", "-ac", "1", "-ar", str(ENGINE_RATE), "-c:a", "pcm_s16le", "-map_metadata", "-1",
               "-fflags", "+bitexact", "-flags:a", "+bitexact", "-f", "wav", str(destination)]
    completed = subprocess.run(command, capture_output=True, timeout=600, check=False)
    if completed.returncode != 0 or not destination.is_file():
        raise EvaluationError(f"FFmpeg could not write {destination.name}")


def assemble(output_dir: Path, ffmpeg: Path) -> list[dict]:
    """Build every conversation and its truth from the synthesized utterances."""
    record = recipe.load_json(output_dir / SYNTHESIS_RECORD)
    utterances: dict[tuple[str, str], array.array] = {}
    for entry in record["utterances"]:
        path = output_dir / UTTERANCES / f"{entry['key']}.wav"
        if recipe.sha256_file(path) != entry["sha256"]:
            raise EvaluationError(f"{path.name} differs from its synthesis record")
        utterances[(entry["voice"], entry["text"])] = recipe.read_wav(path, SYNTHESIS_RATE)
    fixtures = output_dir / FIXTURES
    fixtures.mkdir(exist_ok=False)
    summaries = []
    for item in CONVERSATIONS:
        placed, frames = lay_out(item, utterances)
        samples, mixing = mix(placed, frames, item.snr_db)
        pcm, clipped = recipe.quantize(samples)
        native = fixtures / f"{item.fixture_id}-24k.wav"
        recipe.write_wav(native, pcm, SYNTHESIS_RATE)
        engine_wav = fixtures / f"{item.fixture_id}.wav"
        resample(ffmpeg, native, engine_wav)
        native.unlink()
        truth = {"fixture": item.fixture_id, "description": item.description, "generator": GENERATOR_ID,
                 "duration_us": us_for_frames(frames), "speakers": dict(item.voices),
                 "mixing": mixing, "clipped_samples": clipped, "turns": merged_truth(placed),
                 "wav_sha256": recipe.sha256_file(engine_wav)}
        with (fixtures / f"{item.fixture_id}.truth.json").open("xb") as handle:
            handle.write(recipe.json_bytes(truth))
        summaries.append({"fixture": item.fixture_id, "seconds": round(truth["duration_us"] / MICROSECONDS, 2),
                          "turns": len(truth["turns"]), "speakers": len(item.voices), "clipped": clipped})
    return summaries


def extract_media(media_dir: Path, ffmpeg: Path, output_dir: Path, items: Sequence[str]) -> list[str]:
    """16 kHz mono PCM of each fetched real-media item (``<media>/<id>/<file>``)."""
    output_dir.mkdir(parents=True, exist_ok=True)
    written = []
    for item in items:
        folder = media_dir / item
        candidates = [path for path in folder.iterdir() if path.is_file() and path.name != "ATTRIBUTION.txt"] \
            if folder.is_dir() else []
        if len(candidates) != 1:
            raise EvaluationError(f"{item} is not fetched (expected one media file in its folder)")
        resample(ffmpeg, candidates[0], output_dir / f"{item}.wav")
        written.append(item)
    return written


# ---------------------------------------------------------------------------
# Engines and models (pinned; staged only on a hosted runner)
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class Pin:
    """One reviewed download: URL, exact size and the digest its publisher gives.

    ``sha256`` is None only where the publisher gives no digest; the first hosted run
    records it, and the evaluation document says so.
    """

    name: str
    url: str
    size: int
    sha256: str | None
    digest_source: str


SHERPA_VERSION = "v1.13.8"
_SHERPA = f"https://github.com/k2-fsa/sherpa-onnx/releases/download/{SHERPA_VERSION}/"
# Digests: GitHub's release-asset SHA-256 (sherpa-onnx publishes no checksum file for
# its engine archives).
SHERPA_ARCHIVES = {
    "linux-x64": Pin("sherpa-onnx-linux-x64.tar.bz2", _SHERPA + "sherpa-onnx-v1.13.8-linux-x64-shared.tar.bz2",
                     28_156_791, "c0bdb7907d3a74bba1d55d22bf4d9fa75586cf1530614ebe88a27b9118e015c4",
                     "GitHub release-asset digest"),
    "windows-x64": Pin("sherpa-onnx-windows-x64.tar.bz2",
                       _SHERPA + "sherpa-onnx-v1.13.8-win-x64-shared-MT-Release.tar.bz2",
                       24_805_859, "6dffdc715a4465b989446a6105265d2cb345e7101591a17d35534b6758f6e8df",
                       "GitHub release-asset digest"),
    "macos-arm64": Pin("sherpa-onnx-macos-arm64.tar.bz2", _SHERPA + "sherpa-onnx-v1.13.8-osx-arm64-shared.tar.bz2",
                       20_314_448, "b10e5c7e2c30ea03de9c442655d14860d9edc475c6251d58a8f5f06e913a1d56",
                       "GitHub release-asset digest"),
}

_SEGMENTATION = ("https://huggingface.co/csukuangfj/sherpa-onnx-pyannote-segmentation-3-0/resolve/"
                 "9403a6902bb58e3d5ae8c7e77c3422de279db2e0/")
SEGMENTATION = {
    "pyannote-3.0": Pin("pyannote-segmentation-3.0.onnx", _SEGMENTATION + "model.onnx", 5_992_913,
                        "220ad67ca923bef2fa91f2390c786097bf305bceb5e261d4af67b38e938e1079",
                        "Hugging Face LFS SHA-256"),
    "pyannote-3.0-int8": Pin("pyannote-segmentation-3.0.int8.onnx", _SEGMENTATION + "model.int8.onnx", 1_540_506,
                             "d582f4b4c6b48205de7e0643c57df0df5615a3c176189be3fc461e9d18827b5d",
                             "Hugging Face LFS SHA-256"),
}

_EMBEDDINGS = "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/"
# Digests: the release's own checksum.txt (sherpa-onnx, uploaded 2026-05-20).
EMBEDDINGS = {
    "campplus-en": Pin("3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx",
                       _EMBEDDINGS + "3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx", 29_596_978,
                       "357a834f702b80161e5b981182c038e18553c1f2ca752ed6cec2052365d4129b", "checksum.txt"),
    "eres2net-en": Pin("3dspeaker_speech_eres2net_sv_en_voxceleb_16k.onnx",
                       _EMBEDDINGS + "3dspeaker_speech_eres2net_sv_en_voxceleb_16k.onnx", 26_485_263,
                       "c59158379255ad66e161679cca6af8d52d51e389e3224ab7d7a7baae295c2db5", "checksum.txt"),
    "titanet-small": Pin("nemo_en_titanet_small.onnx", _EMBEDDINGS + "nemo_en_titanet_small.onnx", 40_257_283,
                         "ad4a1802485d8b34c722d2a9d04249662f2ece5d28a7a039063ca22f515a789e", "checksum.txt"),
    "wespeaker-r34": Pin("wespeaker_en_voxceleb_resnet34.onnx", _EMBEDDINGS + "wespeaker_en_voxceleb_resnet34.onnx",
                         26_534_365, "5ef208a9da1453335308a6b6f4e6dfbd7e183a38b604de0a57664f45d257fe94",
                         "checksum.txt"),
}

_POLYVOICE = "https://github.com/ekhodzitsky/polyvoice/releases/download/v1.0.0/"
POLYVOICE = {
    "linux-x64": Pin("polyvoice", _POLYVOICE + "polyvoice-linux-x86_64", 5_551_256,
                     "ca6595a85b2d6ad93192b5d4b5c497299e7c646c5e1409374b93e03e52ee0c77", "GitHub release-asset digest"),
    "windows-x64": Pin("polyvoice.exe", _POLYVOICE + "polyvoice-windows-x86_64.exe", 5_895_168,
                       "7d9ad64259c334ad9693d6fb722bf40baf7d7429dd44ad37b333d99625358ed3",
                       "GitHub release-asset digest"),
    "macos-arm64": Pin("polyvoice", _POLYVOICE + "polyvoice-macos-arm64", 3_816_912,
                       "3f25ba520d71a7a4694ed4f0e8fccd46af2add2f03a745a259a4bcfba596b51d",
                       "GitHub release-asset digest"),
}

TINYDIARIZE = Pin("ggml-small.en-tdrz.bin",
                  "https://huggingface.co/akashmjn/tinydiarize-whisper.cpp/resolve/"
                  "d44ba793fc67e509623a88a409723311fa677744/ggml-small.en-tdrz.bin",
                  487_614_184, "ceac3ec06d1d98ef71aec665283564631055fd6129b79d8e1be4f9cc33cc54b4",
                  "Hugging Face LFS SHA-256")

MAX_ARCHIVE_MEMBERS = 2_000
MAX_EXTRACTED_BYTES = 400 * 1024 * 1024
USER_AGENT = "vsift-p16-speakers-eval/1"


def download(pin: Pin, destination: Path) -> str:
    """Stream at most the reviewed byte count over HTTPS and check the digest."""
    if destination.exists():
        raise EvaluationError(f"{destination.name} already exists")
    destination.parent.mkdir(parents=True, exist_ok=True)
    reviewed = recipe.PinnedFile(destination.name, pin.url, pin.size, pin.sha256)
    return recipe.download(destination.parent, reviewed)


def validate_member_name(name: str) -> PurePosixPath:
    """Refuse absolute, parent-relative, Windows-special and empty archive paths."""
    if "\\" in name or ":" in name or "\x00" in name:
        raise EvaluationError("an archive member has an ambiguous path")
    path = PurePosixPath(name)
    if path.is_absolute() or any(part == ".." for part in path.parts):
        raise EvaluationError("an archive member leaves the extraction folder")
    parts = [part for part in path.parts if part != "."]
    if not parts:
        raise EvaluationError("an archive member has an empty path")
    return PurePosixPath(*parts)


def safe_extract(archive: Path, destination: Path) -> list[dict]:
    """Extract regular files and folders only; every link is recorded and refused.

    The shared builds' library aliases are links; they are re-created afterwards by
    `link_aliases` only when their target is a file of the same folder.
    """
    destination.mkdir(parents=True, exist_ok=False)
    files: list[dict] = []
    links: list[tuple[PurePosixPath, str]] = []
    total = 0
    with tarfile.open(archive, mode="r:*") as source:
        members = source.getmembers()
        if len(members) > MAX_ARCHIVE_MEMBERS:
            raise EvaluationError("the archive holds more members than reviewed")
        for member in members:
            relative = validate_member_name(member.name)
            target = destination.joinpath(*relative.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            elif member.isfile():
                total += member.size
                if total > MAX_EXTRACTED_BYTES:
                    raise EvaluationError("the archive expands beyond the reviewed size")
                target.parent.mkdir(parents=True, exist_ok=True)
                handle = source.extractfile(member)
                if handle is None:
                    raise EvaluationError("an archive file could not be read")
                digest = hashlib.sha256()
                with handle, target.open("xb") as sink:
                    while block := handle.read(1024 * 1024):
                        digest.update(block)
                        sink.write(block)
                if member.mode & 0o111:
                    target.chmod(0o755)
                files.append({"path": relative.as_posix(), "bytes": member.size, "sha256": digest.hexdigest()})
            elif member.issym() or member.islnk():
                links.append((relative, member.linkname))
            else:
                raise EvaluationError("the archive holds a device, FIFO or other special member")
    for relative, link_target in links:
        link_alias(destination, relative, link_target)
        files.append({"path": relative.as_posix(), "link_to": link_target})
    return files


def link_alias(destination: Path, relative: PurePosixPath, link_target: str) -> None:
    """Copy a library alias from its target when the target is a file of the same folder."""
    target_name = PurePosixPath(link_target)
    if len(target_name.parts) != 1 or target_name.name in {"", ".", ".."}:
        raise EvaluationError("an archive link points outside its own folder")
    folder = destination.joinpath(*relative.parent.parts)
    source = folder / target_name.name
    alias = folder / relative.name
    if not source.is_file() or source.is_symlink():
        raise EvaluationError("an archive link does not point at a file of the archive")
    shutil.copyfile(source, alias)
    alias.chmod(source.stat().st_mode & 0o777)


def find_one(root: Path, name: str) -> Path:
    found = [path for path in root.rglob(name) if path.is_file()]
    if len(found) != 1:
        raise EvaluationError(f"expected exactly one {name} in the staged engine, found {len(found)}")
    return found[0]


def platform_key() -> str:
    machine = platform.machine().lower()
    if sys.platform.startswith("linux") and machine in {"x86_64", "amd64"}:
        return "linux-x64"
    if os.name == "nt" and machine in {"x86_64", "amd64"}:
        return "windows-x64"
    if sys.platform == "darwin" and machine in {"arm64", "aarch64"}:
        return "macos-arm64"
    raise EvaluationError("no engine build is pinned for this platform")


def stage(dest: Path, everything: bool) -> dict:
    """Stage sherpa-onnx, the models and polyvoice; everything adds the alternatives."""
    if not os.environ.get("GITHUB_ACTIONS"):
        raise EvaluationError("engines and models are staged only on a hosted runner")
    key = platform_key()
    dest.mkdir(parents=True, exist_ok=False)
    inventory: dict = {"platform": key, "downloads": [], "engines": {}}

    def fetch(pin: Pin, folder: str) -> Path:
        path = dest / folder / pin.name
        digest = download(pin, path)
        inventory["downloads"].append({"name": pin.name, "url": pin.url, "bytes": pin.size, "sha256": digest,
                                       "pinned": pin.sha256 is not None, "digest_source": pin.digest_source})
        return path

    archive = fetch(SHERPA_ARCHIVES[key], "archives")
    files = safe_extract(archive, dest / "sherpa-onnx")
    executable = "sherpa-onnx-offline-speaker-diarization" + (".exe" if key == "windows-x64" else "")
    binary = find_one(dest / "sherpa-onnx", executable)
    inventory["engines"]["sherpa-onnx"] = {
        "version": SHERPA_VERSION, "binary": str(binary.relative_to(dest)),
        "files": len(files), "extracted_bytes": sum(entry.get("bytes", 0) for entry in files),
        "needed_by_binary": runtime_files(binary, files)}
    archive.unlink()
    segmentation = SEGMENTATION if everything else {"pyannote-3.0": SEGMENTATION["pyannote-3.0"]}
    embeddings = EMBEDDINGS if everything else {"campplus-en": EMBEDDINGS["campplus-en"]}
    for pin in segmentation.values():
        fetch(pin, "models")
    for pin in embeddings.values():
        fetch(pin, "models")
    polyvoice = fetch(POLYVOICE[key], "polyvoice")
    polyvoice.chmod(0o755)
    inventory["engines"]["polyvoice"] = {"version": "v1.0.0", "binary": str(polyvoice.relative_to(dest))}
    if everything:
        fetch(TINYDIARIZE, "models")
    with (dest / "inventory.json").open("x", encoding="utf-8") as handle:
        json.dump(inventory, handle, indent=2)
        handle.write("\n")
    return inventory


def runtime_files(binary: Path, files: Sequence[dict]) -> dict:
    """What the diarization binary needs beside itself: its folder's libraries."""
    libraries = [entry for entry in files if "bytes" in entry and re.search(r"\.(so(\.\d+)*|dylib|dll)$", entry["path"])]
    return {"binary_bytes": binary.stat().st_size,
            "libraries": sorted({PurePosixPath(entry["path"]).name for entry in libraries}),
            "library_bytes": sum(entry["bytes"] for entry in libraries)}


# ---------------------------------------------------------------------------
# Running engines with measurement
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class Measured:
    """One finished run: exit status, times and peak memory (None where unmeasured)."""

    returncode: int
    wall_s: float
    cpu_s: float | None
    peak_rss_mib: float | None
    stdout: str
    stderr: str


def run_measured(argv: Sequence[str], env: Mapping[str, str] | None = None, cwd: Path | None = None,
                 timeout_s: int = ENGINE_TIMEOUT_S) -> Measured:
    """Run one program with an explicit argument list; on POSIX read its own rusage."""
    started = time.monotonic()
    process = subprocess.Popen(list(argv), stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               stdin=subprocess.DEVNULL, env=dict(env) if env is not None else None,
                               cwd=str(cwd) if cwd is not None else None)
    output: dict[str, bytes] = {}

    def drain(name: str, stream) -> None:
        output[name] = stream.read()

    readers = [threading.Thread(target=drain, args=("out", process.stdout)),
               threading.Thread(target=drain, args=("err", process.stderr))]
    for reader in readers:
        reader.start()
    timer = threading.Timer(timeout_s, process.kill)
    timer.start()
    cpu_s = peak = None
    try:
        if hasattr(os, "wait4"):
            _, status, usage = os.wait4(process.pid, 0)
            process.returncode = os.waitstatus_to_exitcode(status)
            cpu_s = usage.ru_utime + usage.ru_stime
            # ru_maxrss is KiB on Linux and bytes on macOS.
            peak = usage.ru_maxrss / 1024.0 if sys.platform.startswith("linux") else usage.ru_maxrss / (1024.0 * 1024.0)
        else:
            process.wait()
    finally:
        timer.cancel()
        for reader in readers:
            reader.join()
        for stream in (process.stdout, process.stderr):
            if stream is not None:
                stream.close()
    return Measured(process.returncode, time.monotonic() - started, cpu_s, peak,
                    output.get("out", b"").decode("utf-8", "replace"),
                    output.get("err", b"").decode("utf-8", "replace"))


SHERPA_LINE = re.compile(r"^\s*(\d+\.\d+)\s+--\s+(\d+\.\d+)\s+speaker_(\d+)(?:\s+confidence=(\S+))?\s*$")


def parse_sherpa(stdout: str) -> list[dict]:
    """Turns from sherpa-onnx's ``start -- end speaker_NN [confidence=x]`` lines."""
    turns = []
    for line in stdout.splitlines():
        match = SHERPA_LINE.match(line)
        if match is None:
            continue
        start, end, speaker, confidence = match.groups()
        turn = {"speaker": f"spk{int(speaker)}", "start_us": round(float(start) * MICROSECONDS),
                "end_us": round(float(end) * MICROSECONDS)}
        if confidence is not None and confidence != "n/a":
            turn["confidence"] = float(confidence)
        if turn["end_us"] > turn["start_us"]:
            turns.append(turn)
    return turns


def parse_rttm(text: str) -> list[dict]:
    """Turns from RTTM ``SPEAKER`` lines (start and duration in seconds)."""
    turns = []
    for line in text.splitlines():
        fields = line.split()
        if len(fields) < 8 or fields[0] != "SPEAKER":
            continue
        start = round(float(fields[3]) * MICROSECONDS)
        duration = round(float(fields[4]) * MICROSECONDS)
        if duration > 0:
            turns.append({"speaker": fields[7], "start_us": start, "end_us": start + duration})
    return sorted(turns, key=lambda turn: turn["start_us"])


def parse_whisper_json(text: str) -> list[dict]:
    """Segments of whisper-cli's ``-oj`` output: times in microseconds, text, turn mark."""
    document = json.loads(text)
    segments = []
    for entry in document.get("transcription", []):
        offsets = entry.get("offsets", {})
        start, end = int(offsets.get("from", 0)) * 1_000, int(offsets.get("to", 0)) * 1_000
        if end > start:
            segments.append({"start_us": start, "end_us": end, "text": str(entry.get("text", "")).strip(),
                             "speaker_turn_next": bool(entry.get("speaker_turn_next", False))})
    return segments


def alternating_turns(segments: Sequence[dict]) -> list[dict]:
    """tinydiarize marks only where the speaker changes: alternate two labels at each mark."""
    turns = []
    speaker = 0
    for segment in segments:
        turns.append({"speaker": f"spk{speaker}", "start_us": segment["start_us"], "end_us": segment["end_us"]})
        if segment["speaker_turn_next"]:
            speaker = 1 - speaker
    return turns


# ---------------------------------------------------------------------------
# Scoring
# ---------------------------------------------------------------------------

def frame_count(duration_us: int, frame_us: int = FRAME_US) -> int:
    return -(-duration_us // frame_us)


def activity(turns: Iterable[dict], frames: int, frame_us: int = FRAME_US) -> dict[str, bytearray]:
    """One 0/1 array per speaker: the frames whose centre lies inside one of its turns."""
    arrays: dict[str, bytearray] = {}
    for turn in turns:
        row = arrays.setdefault(turn["speaker"], bytearray(frames))
        first = max(0, (turn["start_us"] + frame_us // 2) // frame_us)
        last = min(frames, (turn["end_us"] + frame_us // 2) // frame_us)
        for index in range(first, last):
            row[index] = 1
    return arrays


def collar_mask(reference: Iterable[dict], frames: int, collar_us: int, frame_us: int = FRAME_US) -> bytearray:
    """Frames within the collar of any true boundary are not scored (value 0)."""
    mask = bytearray([1]) * frames
    if collar_us <= 0:
        return mask
    for turn in reference:
        for boundary in (turn["start_us"], turn["end_us"]):
            first = max(0, (boundary - collar_us) // frame_us)
            last = min(frames, -(-(boundary + collar_us) // frame_us))
            for index in range(first, last):
                mask[index] = 0
    return mask


def best_mapping(overlap: Mapping[str, Mapping[str, int]], reference: Sequence[str],
                 hypothesis: Sequence[str]) -> dict[str, str]:
    """The one-to-one map of reference to hypothesis speakers with the most shared frames."""
    best: tuple[int, dict[str, str]] = (-1, {})
    choices = list(hypothesis) + [None]
    for assignment in itertools.product(choices, repeat=len(reference)):
        chosen = [label for label in assignment if label is not None]
        if len(chosen) != len(set(chosen)):
            continue
        score = sum(overlap[ref][hyp] for ref, hyp in zip(reference, assignment) if hyp is not None)
        if score > best[0]:
            best = (score, {ref: hyp for ref, hyp in zip(reference, assignment) if hyp is not None})
    return best[1]


def diarization_error(reference: Sequence[dict], hypothesis: Sequence[dict], duration_us: int,
                      collar_us: int = COLLAR_US) -> dict:
    """NIST-style DER with overlap scored: miss, false alarm and confusion over true speech."""
    frames = frame_count(duration_us)
    ref = activity(reference, frames)
    hyp = activity(hypothesis, frames)
    mask = collar_mask(reference, frames, collar_us)
    ref_labels = sorted(ref)
    hyp_labels = sorted(hyp)
    overlap = {r: {h: 0 for h in hyp_labels} for r in ref_labels}
    for r in ref_labels:
        for h in hyp_labels:
            overlap[r][h] = sum(1 for index in range(frames) if mask[index] and ref[r][index] and hyp[h][index])
    mapping = best_mapping(overlap, ref_labels, hyp_labels)
    miss = false_alarm = confusion = total = 0
    for index in range(frames):
        if not mask[index]:
            continue
        n_ref = sum(ref[r][index] for r in ref_labels)
        n_hyp = sum(hyp[h][index] for h in hyp_labels)
        correct = sum(1 for r in ref_labels if ref[r][index] and r in mapping and hyp[mapping[r]][index])
        total += n_ref
        miss += max(0, n_ref - n_hyp)
        false_alarm += max(0, n_hyp - n_ref)
        confusion += min(n_ref, n_hyp) - correct
    if total == 0:
        raise EvaluationError("the reference holds no scored speech")
    return {"der": (miss + false_alarm + confusion) / total, "miss": miss / total,
            "false_alarm": false_alarm / total, "confusion": confusion / total,
            "scored_speech_s": total * FRAME_US / MICROSECONDS, "collar_s": collar_us / MICROSECONDS,
            "reference_speakers": len(ref_labels), "found_speakers": len(hyp_labels),
            "mapping": mapping}


def overlap_by_speaker(turns: Sequence[dict], start_us: int, end_us: int) -> dict[str, int]:
    shares: dict[str, int] = {}
    for turn in turns:
        shared = min(end_us, turn["end_us"]) - max(start_us, turn["start_us"])
        if shared > 0:
            shares[turn["speaker"]] = shares.get(turn["speaker"], 0) + shared
    return shares


def label_segment(turns: Sequence[dict], start_us: int, end_us: int) -> str | None:
    """VSift's proposed rule: the segment's speaker, or None for no confident speaker."""
    length = end_us - start_us
    shares = overlap_by_speaker(turns, start_us, end_us)
    if length <= 0 or not shares:
        return None
    speech = sum(shares.values())
    speaker, dominant = max(shares.items(), key=lambda item: (item[1], item[0]))
    if min(speech, length) < LABEL_MIN_SPEECH_SHARE * length or dominant < LABEL_MIN_DOMINANT_SHARE * speech:
        return None
    return speaker


def segment_attribution(reference: Sequence[dict], hypothesis: Sequence[dict], segments: Sequence[dict],
                        mapping: Mapping[str, str]) -> dict:
    """How the proposed rule labels the transcript's segments against the truth."""
    inverse = {hyp: ref for ref, hyp in mapping.items()}
    counted = labelled = correct = wrong = ambiguous_truth = 0
    for segment in segments:
        truth = label_segment(reference, segment["start_us"], segment["end_us"])
        if truth is None:
            ambiguous_truth += 1
            continue
        counted += 1
        found = label_segment(hypothesis, segment["start_us"], segment["end_us"])
        if found is None:
            continue
        labelled += 1
        if inverse.get(found) == truth:
            correct += 1
        else:
            wrong += 1
    return {"segments": counted, "segments_without_one_true_speaker": ambiguous_truth,
            "labelled_share": labelled / counted if counted else None,
            "correct_share_of_labelled": correct / labelled if labelled else None,
            "wrong_share_of_all": wrong / counted if counted else None}


# ---------------------------------------------------------------------------
# Evaluation
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class Candidate:
    """One engine configuration to run on every input."""

    candidate_id: str
    engine: str
    segmentation: str | None = None
    embedding: str | None = None
    threshold: float | None = None
    known_count: bool = True


def candidates(everything: bool) -> list[Candidate]:
    listed = [Candidate("sherpa/campplus-en/known", "sherpa", "pyannote-3.0", "campplus-en"),
              Candidate("sherpa/campplus-en/auto-0.5", "sherpa", "pyannote-3.0", "campplus-en", 0.5, False),
              Candidate("polyvoice/known", "polyvoice"),
              Candidate("polyvoice/auto", "polyvoice", known_count=False)]
    if not everything:
        return listed
    for embedding in ("eres2net-en", "titanet-small", "wespeaker-r34"):
        listed.append(Candidate(f"sherpa/{embedding}/known", "sherpa", "pyannote-3.0", embedding))
        listed.append(Candidate(f"sherpa/{embedding}/auto-0.5", "sherpa", "pyannote-3.0", embedding, 0.5, False))
    for threshold in (0.7, 0.9):
        listed.append(Candidate(f"sherpa/campplus-en/auto-{threshold}", "sherpa", "pyannote-3.0", "campplus-en",
                                threshold, False))
    listed.append(Candidate("sherpa-int8seg/campplus-en/known", "sherpa", "pyannote-3.0-int8", "campplus-en"))
    listed.append(Candidate("whisper-tdrz/turns", "tdrz"))
    return listed


def engine_environment(tools: Path, binary: Path) -> dict[str, str]:
    """The shared builds keep their libraries in ``lib`` beside ``bin``."""
    environment = dict(os.environ)
    library_dir = binary.parent.parent / "lib"
    if library_dir.is_dir():
        if sys.platform.startswith("linux"):
            environment["LD_LIBRARY_PATH"] = str(library_dir)
        elif sys.platform == "darwin":
            environment["DYLD_LIBRARY_PATH"] = str(library_dir)
        elif os.name == "nt":
            environment["PATH"] = f"{library_dir}{os.pathsep}{environment.get('PATH', '')}"
    return environment


def polyvoice_environment(tools: Path) -> dict[str, str]:
    """polyvoice keeps its models in the user's cache folder: point it inside the staging.

    On Windows the folder is the known Local AppData folder, which no variable moves;
    the disposable runner's own is used there.
    """
    environment = dict(os.environ)
    cache = tools / "polyvoice-cache"
    cache.mkdir(exist_ok=True)
    if os.name != "nt":
        environment["XDG_CACHE_HOME"] = str(cache)
        environment["HOME"] = str(cache)
    return environment


def polyvoice_models(tools: Path) -> Path:
    if os.name == "nt":
        return Path(os.environ["LOCALAPPDATA"]) / "polyvoice" / "models"
    if sys.platform == "darwin":
        return tools / "polyvoice-cache" / "Library" / "Caches" / "polyvoice" / "models"
    return tools / "polyvoice-cache" / "polyvoice" / "models"


def prepare_polyvoice(tools: Path, inventory: dict, warm_up: Path) -> dict:
    """Let polyvoice fetch its own pinned, signed models once, then list what it fetched."""
    binary = tools / inventory["engines"]["polyvoice"]["binary"]
    environment = polyvoice_environment(tools)
    download = run_measured([str(binary), "download-models", "--profile", "balanced"], env=environment)
    warm = run_measured([str(binary), "diarize", str(warm_up), "--format", "rttm", "--quiet",
                         "--models-cache", str(polyvoice_models(tools))], env=environment)
    fetched = []
    models = polyvoice_models(tools)
    for path in sorted(models.rglob("*")) if models.is_dir() else []:
        if path.is_file():
            fetched.append({"path": path.relative_to(models).as_posix(), "bytes": path.stat().st_size,
                            "sha256": recipe.sha256_file(path)})
    return {"download_models_exit": download.returncode, "warm_up_exit": warm.returncode,
            "warm_up_stderr_tail": warm.stderr[-2000:], "files": fetched}


def run_candidate(candidate: Candidate, tools: Path, inventory: dict, wav: Path, speakers: int | None,
                  whisper_cli: Path | None, work: Path) -> tuple[list[dict] | None, dict]:
    """Run one configuration on one input; returns its turns (None on failure) and facts."""
    if candidate.engine == "sherpa":
        binary = tools / inventory["engines"]["sherpa-onnx"]["binary"]
        argv = [str(binary),
                f"--segmentation.pyannote-model={tools / 'models' / SEGMENTATION[candidate.segmentation].name}",
                f"--embedding.model={tools / 'models' / EMBEDDINGS[candidate.embedding].name}",
                f"--segmentation.num-threads={THREADS}", f"--embedding.num-threads={THREADS}",
                "--clustering.compute-confidence=true"]
        if candidate.known_count:
            if speakers is None:
                return None, {"skipped": "speaker count unknown"}
            argv.append(f"--clustering.num-clusters={speakers}")
        else:
            argv.append(f"--clustering.cluster-threshold={candidate.threshold}")
        argv.append(str(wav))
        measured = run_measured(argv, env=engine_environment(tools, binary))
        turns = parse_sherpa(measured.stdout) if measured.returncode == 0 else None
    elif candidate.engine == "polyvoice":
        binary = tools / inventory["engines"]["polyvoice"]["binary"]
        output = work / f"{wav.stem}.{candidate.candidate_id.replace('/', '_')}.rttm"
        argv = [str(binary), "diarize", str(wav), "--format", "rttm", "--quiet", "--output", str(output),
                "--models-cache", str(polyvoice_models(tools))]
        if candidate.known_count:
            if speakers is None:
                return None, {"skipped": "speaker count unknown"}
            argv += ["--speakers", str(speakers)]
        measured = run_measured(argv, env=polyvoice_environment(tools))
        turns = parse_rttm(output.read_text(encoding="utf-8")) if measured.returncode == 0 and output.is_file() else None
    elif candidate.engine == "tdrz":
        if whisper_cli is None or (speakers is not None and speakers != 2):
            return None, {"skipped": "tinydiarize marks turns only; scored for two speakers"}
        prefix = work / f"{wav.stem}.tdrz"
        argv = [str(whisper_cli), "-m", str(tools / "models" / TINYDIARIZE.name), "-f", str(wav), "-l", "en",
                "-t", str(THREADS * 2), "-tdrz", "-oj", "-of", str(prefix), "-np"]
        measured = run_measured(argv)
        path = prefix.with_suffix(".json")
        turns = alternating_turns(parse_whisper_json(path.read_text(encoding="utf-8", errors="replace"))) \
            if measured.returncode == 0 and path.is_file() else None
    else:
        raise EvaluationError(f"unknown engine {candidate.engine}")
    facts = {"exit": measured.returncode, "wall_s": round(measured.wall_s, 3),
             "cpu_s": None if measured.cpu_s is None else round(measured.cpu_s, 3),
             "peak_rss_mib": None if measured.peak_rss_mib is None else round(measured.peak_rss_mib, 1)}
    if turns is None:
        facts["stderr_tail"] = measured.stderr[-2000:]
    return turns, facts


def transcribe(whisper_cli: Path, model: Path, wav: Path, work: Path) -> list[dict]:
    prefix = work / f"{wav.stem}.whisper"
    measured = run_measured([str(whisper_cli), "-m", str(model), "-f", str(wav), "-l", "en", "-t", str(THREADS * 2),
                             "-oj", "-of", str(prefix), "-np"])
    path = prefix.with_suffix(".json")
    if measured.returncode != 0 or not path.is_file():
        raise EvaluationError(f"whisper.cpp could not transcribe {wav.name}")
    return parse_whisper_json(path.read_text(encoding="utf-8", errors="replace"))


NETWORK_CALL = re.compile(r"\b(?:socket|connect)\([^)]*?\b(AF_[A-Z0-9]+)")


def network_check(tools: Path, wav: Path, output: Path) -> dict:
    """Trace the default sherpa-onnx and polyvoice runs for socket and connect calls (Linux).

    The diarization itself must never touch the network: a call of an internet family
    is a finding. strace is the runner's; no shell is used.
    """
    inventory = recipe.load_json(tools / "inventory.json")
    sherpa = tools / inventory["engines"]["sherpa-onnx"]["binary"]
    polyvoice = tools / inventory["engines"]["polyvoice"]["binary"]
    commands = {
        "sherpa-onnx": ([str(sherpa),
                         f"--segmentation.pyannote-model={tools / 'models' / SEGMENTATION['pyannote-3.0'].name}",
                         f"--embedding.model={tools / 'models' / EMBEDDINGS['campplus-en'].name}",
                         "--clustering.num-clusters=2", str(wav)], engine_environment(tools, sherpa)),
        "polyvoice": ([str(polyvoice), "diarize", str(wav), "--format", "rttm", "--quiet", "--speakers", "2",
                       "--models-cache", str(polyvoice_models(tools))], polyvoice_environment(tools)),
    }
    results = {}
    for name, (argv, environment) in commands.items():
        log = output.parent / f"strace-{name}.log"
        measured = run_measured(["strace", "-f", "-qq", "-e", "trace=socket,connect", "-o", str(log), *argv],
                                env=environment)
        calls = [match.group(1) for line in log.read_text(encoding="utf-8", errors="replace").splitlines()
                 for match in [NETWORK_CALL.search(line)] if match is not None] if log.is_file() else []
        internet = [family for family in calls if family in {"AF_INET", "AF_INET6"}]
        results[name] = {"exit": measured.returncode, "socket_or_connect_calls": len(calls),
                         "internet_family_calls": len(internet)}
    output.write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")
    return results


def wav_duration_us(path: Path) -> int:
    with wave.open(str(path), "rb") as source:
        if (source.getnchannels(), source.getframerate()) != (1, ENGINE_RATE):
            raise EvaluationError(f"{path.name} is not 16 kHz mono")
        return source.getnframes() * MICROSECONDS // ENGINE_RATE


@dataclass(frozen=True)
class EvaluationInput:
    """One WAV to diarize: its truth (exact or read), the speaker count, its role."""

    input_id: str
    wav: Path
    reference: list[dict] | None
    speakers: int | None
    collar_us: int
    exact: bool
    transcribe: bool


def load_inputs(inputs_dir: Path, rm06_reference: Path | None) -> list[EvaluationInput]:
    loaded = []
    for item in CONVERSATIONS:
        truth = recipe.load_json(inputs_dir / FIXTURES / f"{item.fixture_id}.truth.json")
        loaded.append(EvaluationInput(item.fixture_id, inputs_dir / FIXTURES / f"{item.fixture_id}.wav",
                                      truth["turns"], len(item.voices), COLLAR_US, True, True))
    rm06 = inputs_dir / "media" / "RM06.wav"
    if rm06.is_file():
        reference = None
        speakers = 2
        if rm06_reference is not None and rm06_reference.is_file():
            document = recipe.load_json(rm06_reference)
            reference = [{"speaker": turn["speaker"], "start_us": turn["start_us"], "end_us": turn["end_us"]}
                         for turn in document["turns"]]
            speakers = len({turn["speaker"] for turn in reference})
        loaded.append(EvaluationInput("RM06", rm06, reference, speakers, REFERENCE_COLLAR_US, False, True))
    rm09 = inputs_dir / "media" / "RM09.wav"
    if rm09.is_file():
        loaded.append(EvaluationInput("RM09", rm09, None, None, COLLAR_US, False, False))
    return loaded


def evaluate(tools: Path, inputs_dir: Path, whisper_cli: Path | None, whisper_model: Path | None,
             output_dir: Path, everything: bool, rm06_reference: Path | None) -> dict:
    """Run every candidate on every input and score it; without whisper.cpp (the
    Windows and macOS smoke) the segment labelling is not measured."""
    inventory = recipe.load_json(tools / "inventory.json")
    output_dir.mkdir(parents=True, exist_ok=False)
    work = output_dir / "hypotheses"
    work.mkdir()
    inputs = load_inputs(inputs_dir, rm06_reference)
    report: dict = {"generator": GENERATOR_ID, "platform": inventory["platform"], "threads": THREADS,
                    "inventory": inventory, "rule": {"min_speech_share": LABEL_MIN_SPEECH_SHARE,
                                                     "min_dominant_share": LABEL_MIN_DOMINANT_SHARE},
                    "inputs": {}, "runs": []}
    report["polyvoice_models"] = prepare_polyvoice(tools, inventory, inputs[0].wav)
    segments_by_input: dict[str, list[dict]] = {}
    for item in inputs:
        duration = wav_duration_us(item.wav)
        report["inputs"][item.input_id] = {"seconds": round(duration / MICROSECONDS, 2), "speakers": item.speakers,
                                           "reference": "exact" if item.exact else
                                           ("read from the transcript" if item.reference else "none")}
        if item.transcribe and whisper_cli is not None and whisper_model is not None:
            segments_by_input[item.input_id] = transcribe(whisper_cli, whisper_model, item.wav, work)
    for candidate in candidates(everything):
        for item in inputs:
            turns, facts = run_candidate(candidate, tools, inventory, item.wav, item.speakers, whisper_cli, work)
            duration = wav_duration_us(item.wav)
            row = {"candidate": candidate.candidate_id, "input": item.input_id, **facts,
                   "rtf": round(facts["wall_s"] / (duration / MICROSECONDS), 4) if "wall_s" in facts else None}
            if turns is not None:
                row["found_speakers"] = len({turn["speaker"] for turn in turns})
                (work / f"{item.input_id}.{candidate.candidate_id.replace('/', '_')}.json").write_text(
                    json.dumps(turns), encoding="utf-8")
                if item.reference is not None:
                    score = diarization_error(item.reference, turns, duration, item.collar_us)
                    row["score"] = score
                    if item.input_id in segments_by_input:
                        row["segments"] = segment_attribution(item.reference, turns, segments_by_input[item.input_id],
                                                              score["mapping"])
            report["runs"].append(row)
            print(f"{candidate.candidate_id} on {item.input_id}: exit {facts.get('exit')}, "
                  f"{row.get('found_speakers')} speakers, DER {row.get('score', {}).get('der')}", flush=True)
    if "RM06" in segments_by_input:
        write_labelled_transcript(output_dir / "rm06-labelled-transcript.md", segments_by_input["RM06"], report, work)
    with (output_dir / "report.json").open("x", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2)
        handle.write("\n")
    (output_dir / "summary.md").write_text(summary(report), encoding="utf-8")
    return report


def write_labelled_transcript(path: Path, segments: Sequence[dict], report: dict, work: Path) -> None:
    """RM06 (public domain): each segment with each candidate's label, for a person to judge."""
    columns = [row["candidate"] for row in report["runs"] if row["input"] == "RM06" and row.get("found_speakers")]
    hypotheses = {name: json.loads((work / f"RM06.{name.replace('/', '_')}.json").read_text(encoding="utf-8"))
                  for name in columns}
    lines = ["# RM06 labelled transcript (public domain, NASA Johnson Space Center)", "",
             "Each whisper.cpp `base` segment with the label each candidate's turns give it under the proposed "
             "rule (`-` is no confident speaker). Labels are each run's own; they are not mapped to people.", "",
             "| Start | End | " + " | ".join(columns) + " | Text |",
             "| --- | --- | " + " | ".join("---" for _ in columns) + " | --- |"]
    for segment in segments:
        labels = [label_segment(hypotheses[name], segment["start_us"], segment["end_us"]) or "-" for name in columns]
        text = segment["text"].replace("|", "/")
        lines.append(f"| {segment['start_us'] / MICROSECONDS:.2f} | {segment['end_us'] / MICROSECONDS:.2f} | "
                     + " | ".join(labels) + f" | {text} |")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def percent(value: float | None) -> str:
    return "n/a" if value is None else f"{value * 100:.1f}%"


def summary(report: dict) -> str:
    """The job summary: one table of accuracy, one of cost."""
    lines = [f"## Speaker-label engines on {report['platform']} ({report['threads']} threads per engine)", "",
             "DER: diarization error rate (miss + false alarm + confusion over true speech; 0.25 s collar, "
             "0.5 s on RM06's read reference). Labelled: the share of transcript segments given a speaker by the "
             "proposed rule; wrong: segments given the wrong speaker, over all segments.", "",
             "| Candidate | Input | Speakers found / true | DER | Confusion | Labelled | Wrong | RTF | CPU s | Peak MiB |",
             "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"]
    for row in report["runs"]:
        score = row.get("score", {})
        segments = row.get("segments", {})
        truth = report["inputs"][row["input"]]["speakers"]
        status = row.get("skipped") or (f"exit {row['exit']}" if row.get("exit") not in (0, None) else "")
        lines.append(f"| {row['candidate']} | {row['input']} | {row.get('found_speakers', status or '-')} / "
                     f"{truth if truth is not None else '?'} | {percent(score.get('der'))} | "
                     f"{percent(score.get('confusion'))} | {percent(segments.get('labelled_share'))} | "
                     f"{percent(segments.get('wrong_share_of_all'))} | {row.get('rtf', 'n/a')} | "
                     f"{row.get('cpu_s', 'n/a')} | {row.get('peak_rss_mib', 'n/a')} |")
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------------------
# Command line
# ---------------------------------------------------------------------------

def main(argv: Sequence[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    voices = commands.add_parser("fetch-voices", help="download the extra pinned Kokoro voices")
    voices.add_argument("--assets", type=Path, required=True)
    synth = commands.add_parser("synthesize", help="speak every distinct turn with Kokoro")
    synth.add_argument("--assets", type=Path, required=True)
    synth.add_argument("--output-dir", type=Path, required=True)
    build = commands.add_parser("assemble", help="lay out, mix and resample the conversations; write the truth")
    build.add_argument("--output-dir", type=Path, required=True)
    build.add_argument("--ffmpeg", type=Path, required=True)
    media = commands.add_parser("media", help="extract 16 kHz mono PCM from fetched real-media items")
    media.add_argument("--media", type=Path, required=True)
    media.add_argument("--ffmpeg", type=Path, required=True)
    media.add_argument("--output-dir", type=Path, required=True)
    media.add_argument("--item", action="append", default=[])
    staging = commands.add_parser("stage", help="download and verify the pinned engines and models")
    staging.add_argument("--dest", type=Path, required=True)
    staging.add_argument("--everything", action="store_true")
    run = commands.add_parser("evaluate", help="run and score every candidate on every input")
    run.add_argument("--tools", type=Path, required=True)
    run.add_argument("--inputs", type=Path, required=True)
    run.add_argument("--whisper-cli", type=Path)
    run.add_argument("--whisper-model", type=Path)
    run.add_argument("--output-dir", type=Path, required=True)
    run.add_argument("--everything", action="store_true")
    run.add_argument("--rm06-reference", type=Path)
    network = commands.add_parser("network-check", help="trace the default engines for network calls (Linux)")
    network.add_argument("--tools", type=Path, required=True)
    network.add_argument("--wav", type=Path, required=True)
    network.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args(argv)
    if arguments.command == "fetch-voices":
        print(json.dumps(fetch_voices(arguments.assets), indent=2))
    elif arguments.command == "synthesize":
        record = synthesize(arguments.assets, arguments.output_dir)
        print(f"{len(record['utterances'])} utterances synthesized")
    elif arguments.command == "assemble":
        print(json.dumps(assemble(arguments.output_dir, arguments.ffmpeg), indent=2))
    elif arguments.command == "media":
        print(extract_media(arguments.media, arguments.ffmpeg, arguments.output_dir, arguments.item))
    elif arguments.command == "stage":
        print(json.dumps(stage(arguments.dest, arguments.everything), indent=2))
    elif arguments.command == "evaluate":
        report = evaluate(arguments.tools, arguments.inputs, arguments.whisper_cli, arguments.whisper_model,
                          arguments.output_dir, arguments.everything, arguments.rm06_reference)
        print(summary(report))
    elif arguments.command == "network-check":
        results = network_check(arguments.tools, arguments.wav, arguments.output)
        print(json.dumps(results, indent=2))
        if any(result["internet_family_calls"] for result in results.values()):
            raise EvaluationError("an engine opened an internet socket while diarizing")


if __name__ == "__main__":
    try:
        main()
    except (EvaluationError, recipe.GenerationError, OSError, ValueError, KeyError, tarfile.TarError,
            subprocess.SubprocessError) as error:
        print(f"speaker-label evaluation failed: {error}", file=sys.stderr)
        sys.exit(1)
