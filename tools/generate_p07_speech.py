"""Generate P07 speech fixture variants from the frozen corpus scripts.

Test-fixture tooling only. The Kokoro text-to-speech model, PyTorch and their Python
dependencies are never VSift runtime dependencies, never shipped and never named in a
Cargo manifest. They run on a disposable GitHub Actions runner
(`.github/workflows/p07-speech-fixtures.yml`); the maintainer's machine needs neither.
The licence review is `docs/planning/p07-speech-fixtures.md`.

The work is split so that only one stage needs the model:

``fetch``       Download the pinned Kokoro revision's model, config and voice files and
                the spaCy English pipeline wheel. Standard library only; every byte count
                is bounded and every file is checked against a reviewed pin.
``synthesize``  Speak each speech-bearing fixture's frozen script with Kokoro and write
                one clean utterance WAV per fixture plus the synthesis record. Needs the
                pinned environment from ``tools/p07-speech-requirements.txt``.
``assemble``    Place each committed utterance on its fixture's audio timeline, add the
                documented deterministic noise to noisy fixtures, and mux the result with
                the untouched P04 video stream into a new ``<id>-speech`` variant.
                Standard library plus an explicitly selected FFmpeg.

``manifest.json`` stays the only truth source. Scripts, modes, offsets and speech
windows are read from it; nothing here derives an expected timestamp by listening to
the output. Existing P04 fixture bytes are read, verified against their provenance and
never written.
"""

from __future__ import annotations

import argparse
import array
from dataclasses import dataclass
from decimal import Decimal
import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import platform
import random
import re
import subprocess
import sys
import tempfile
from typing import Iterable, Protocol, Sequence
import urllib.error
import urllib.parse
import urllib.request
import wave


ROOT = Path(__file__).resolve().parents[1]
CORPUS = ROOT / "fixtures" / "corpus"
GENERATED = CORPUS / "generated"
MANIFEST = CORPUS / "manifest.json"
BASE_PROVENANCE = GENERATED / "provenance.json"

GENERATOR_ID = "tools/generate_p07_speech.py v1"
PROVENANCE_NAME = "speech-provenance.json"
UTTERANCE_DIRECTORY = "speech"
FETCH_RECORD_NAME = "fetch-record.json"

MICROSECONDS = 1_000_000
SAMPLE_RATE = 24_000  # Kokoro's native output rate; utterances are stored losslessly at it.
OUTPUT_AUDIO_RATE = 16_000  # Matches the P04 tone tracks and whisper.cpp's input rate.
INT16_SCALE = 32_767.0

# Synthesis parameters. The seed is reset before every segment so a clip never depends
# on the order in which other clips were generated.
SPEED = 1.0
SEED = 731
TORCH_THREADS = 1
INTER_SEGMENT_PAUSE_US = 400_000

# Placement policy for fixtures whose manifest has no speech event: speak after a fixed
# lead-in and finish before a fixed tail. This is a generation recipe, not truth.
LEAD_IN_US = 500_000
TAIL_US = 250_000

# Noisy-fixture recipe "office-v1": pink noise (Paul Kellet's economy filter over
# uniform white noise) plus sparse decaying keyboard-like clicks, from Python's
# `random.Random(seed).random()` sequence, which the language guarantees to be stable.
# Only +, *, / and sqrt are used, all correctly rounded IEEE operations, so the noise
# is bit-identical on every platform. The level is set by a power ratio so no libm
# transcendental function is involved.
NOISE_RECIPE = "office-v1"
NOISE_SEED = 408
SNR_DB = 10
SNR_POWER_RATIO = 10.0
CLICK_INTERVAL_MIN_US = 150_000
CLICK_INTERVAL_SPAN_US = 450_000
CLICK_LENGTH_US = 10_000
CLICK_DECAY_PER_SAMPLE = 0.99
CLICK_AMPLITUDE = 3.0

SPEECH_MODES = frozenset({"clean-synthetic", "noisy-synthetic", "offset-synthetic"})
SILENT_MODES = frozenset({"none"})
NOISY_MODES = frozenset({"noisy-synthetic"})
OFFSET_MODES = frozenset({"offset-synthetic"})

KOKORO_REPOSITORY = "hexgrad/Kokoro-82M"
KOKORO_REVISION = "f3ff3571791e39611d31c381e3a41a3af07b4987"
KOKORO_BASE_URL = f"https://huggingface.co/{KOKORO_REPOSITORY}/resolve/{KOKORO_REVISION}/"
KOKORO_DIRECTORY = "kokoro"
WHEEL_DIRECTORY = "wheels"


class GenerationError(Exception):
    """A reviewed precondition of the speech-fixture recipe does not hold."""


@dataclass(frozen=True)
class PinnedFile:
    """A reviewed download: exact byte count plus a digest published by its source.

    Hugging Face publishes SHA-256 for LFS files and a git blob SHA-1 for small files.
    GitHub publishes no digest for the spaCy model asset, so that pin is size-only and
    the observed SHA-256 is recorded in the provenance for review.
    """

    path: str
    url: str
    size: int
    sha256: str | None = None
    git_blob_sha1: str | None = None


def _kokoro_file(path: str, size: int, sha256: str | None = None,
                 git_blob_sha1: str | None = None) -> PinnedFile:
    return PinnedFile(f"{KOKORO_DIRECTORY}/{path}", KOKORO_BASE_URL + path, size, sha256, git_blob_sha1)


KOKORO_CONFIG = _kokoro_file("config.json", 2_351, git_blob_sha1="14a726edd3718279eac426630879ff743955b16a")
KOKORO_WEIGHTS = _kokoro_file("kokoro-v1_0.pth", 327_212_226,
                              "496dba118d1a58f5f3db2efc88dbdc216e0483fc89fe6e47ee1f2c53f18ad1e4")
SPACY_MODEL_WHEEL = PinnedFile(
    f"{WHEEL_DIRECTORY}/en_core_web_sm-3.8.0-py3-none-any.whl",
    "https://github.com/explosion/spacy-models/releases/download/en_core_web_sm-3.8.0/"
    "en_core_web_sm-3.8.0-py3-none-any.whl",
    12_806_118,
)


@dataclass(frozen=True)
class Voice:
    """One Kokoro voice. `language` is recorded; `lang_code` selects Kokoro's G2P."""

    language: str
    lang_code: str
    voice_id: str
    file: PinnedFile


VOICES = {
    "en-US": Voice("en-US", "a", "af_heart", _kokoro_file(
        "voices/af_heart.pt", 523_425, "0ab5709b8ffab19bfd849cd11d98f75b60af7733253ad0d67b12382a102cb4ff")),
    "es": Voice("es", "e", "ef_dora", _kokoro_file(
        "voices/ef_dora.pt", 523_420, "d9d69b0f8a2b87a345f269d89639f89dfbd1a6c9da0c498ae36dd34afcf35530")),
}
DEFAULT_LANGUAGE = "en-US"
# Per-sentence languages for multilingual scripts, validated against the manifest text.
SENTENCE_LANGUAGES = {"F08": ("en-US", "es")}

MODEL_FILES = (KOKORO_CONFIG, KOKORO_WEIGHTS) + tuple(voice.file for voice in VOICES.values())
FETCHED_FILES = MODEL_FILES + (SPACY_MODEL_WHEEL,)

# Exact distributions the synthesis stage must run with. The requirements file must
# agree (a unit test enforces it); torch comes from the PyTorch CPU index.
PINNED_DISTRIBUTIONS = {
    "espeakng-loader": "0.2.4",
    "en-core-web-sm": "3.8.0",
    "huggingface-hub": "1.33.0",
    "kokoro": "0.9.4",
    "misaki": "0.9.4",
    "num2words": "0.5.14",
    "numpy": "2.2.6",
    "phonemizer-fork": "3.3.2",
    "spacy": "3.8.7",
    "tokenizers": "0.23.2",
    "torch": "2.14.0+cpu",
    "transformers": "5.17.0",
}

SENTENCE_BOUNDARY = re.compile(r"(?<=[.!?])\s+")
MAX_DOWNLOAD_REDIRECTS = 5


@dataclass(frozen=True)
class Segment:
    """A run of consecutive sentences spoken in one language by one voice."""

    language: str
    text: str


@dataclass(frozen=True)
class SpeechPlan:
    """What one fixture's speech must say and where it may be placed, from the manifest."""

    fixture_id: str
    mode: str
    script: str
    segments: tuple[Segment, ...]
    duration_us: int
    audio_offset_us: int
    window_start_us: int
    window_end_us: int
    window_source: str

    @property
    def noisy(self) -> bool:
        return self.mode in NOISY_MODES


@dataclass(frozen=True)
class Placement:
    """Where an utterance lands. Times are on the normalized presentation timeline."""

    speech_start_us: int
    speech_end_us: int
    track_start_frame: int
    track_frames: int


@dataclass(frozen=True)
class Word:
    """An engine-reported word interval, relative to the start of its segment audio."""

    text: str
    start_us: int
    end_us: int


@dataclass(frozen=True)
class SynthesizedSegment:
    """Raw engine output for one segment. `words` is None when the engine gives none."""

    samples: tuple[float, ...]
    phonemes: tuple[str, ...]
    words: tuple[Word, ...] | None


class Synthesizer(Protocol):
    """The only boundary the recipe needs from a text-to-speech engine."""

    def synthesize(self, text: str, voice: Voice) -> SynthesizedSegment:
        ...


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def json_bytes(value: object) -> bytes:
    return (json.dumps(value, indent=2, ensure_ascii=False) + "\n").encode("utf-8")


def format_seconds(microseconds: int) -> str:
    """Exact decimal seconds for FFmpeg from integer microseconds, without floats."""
    if microseconds < 0:
        raise GenerationError("negative FFmpeg time offsets are not part of the recipe")
    return f"{microseconds // MICROSECONDS}.{microseconds % MICROSECONDS:06d}"


def exact_frames(microseconds: int, rate: int = SAMPLE_RATE) -> int:
    """Convert a time to a whole sample count, refusing a time between samples."""
    frames, remainder = divmod(microseconds * rate, MICROSECONDS)
    if remainder:
        raise GenerationError(f"{microseconds} us is not on the {rate} Hz sample grid")
    return frames


def frames_to_us_ceiling(frames: int, rate: int = SAMPLE_RATE) -> int:
    return -(-frames * MICROSECONDS // rate)


def split_sentences(script: str) -> list[str]:
    """Split on whitespace after terminal punctuation; decimals such as 2.5 stay whole."""
    sentences = [part for part in SENTENCE_BOUNDARY.split(script.strip()) if part]
    if " ".join(sentences) != script:
        raise GenerationError("script sentence split is not lossless; expected single spaces")
    return sentences


def language_segments(fixture_id: str, script: str) -> tuple[Segment, ...]:
    sentences = split_sentences(script)
    languages = SENTENCE_LANGUAGES.get(fixture_id, (DEFAULT_LANGUAGE,) * len(sentences))
    if len(languages) != len(sentences):
        raise GenerationError(f"{fixture_id} has {len(sentences)} sentences but "
                              f"{len(languages)} reviewed sentence languages")
    segments: list[Segment] = []
    for language, sentence in zip(languages, sentences):
        if language not in VOICES:
            raise GenerationError(f"{fixture_id} names unreviewed language {language}")
        if segments and segments[-1].language == language:
            segments[-1] = Segment(language, f"{segments[-1].text} {sentence}")
        else:
            segments.append(Segment(language, sentence))
    return tuple(segments)


def speech_plan(fixture: dict) -> SpeechPlan | None:
    """Derive one fixture's plan from the manifest; None when truth says no audio."""
    fixture_id = fixture["id"]
    audio = fixture["audio"]
    mode = audio["mode"]
    if mode in SILENT_MODES:
        return None
    if mode not in SPEECH_MODES:
        raise GenerationError(f"{fixture_id} has unreviewed audio mode {mode}")
    script = audio["script"]
    if not script.strip():
        raise GenerationError(f"{fixture_id} is speech-bearing but has an empty script")
    duration = fixture["duration_us"]
    offset = audio.get("offset_us", 0)
    if offset < 0 or offset >= duration:
        raise GenerationError(f"{fixture_id} audio offset lies outside the fixture")
    if mode in OFFSET_MODES and offset == 0:
        raise GenerationError(f"{fixture_id} is offset-synthetic without an offset")
    speech_events = [event for event in fixture["events"] if event["kind"] == "speech"]
    if len(speech_events) > 1:
        raise GenerationError(f"{fixture_id} has several speech events; the recipe places one utterance")
    if speech_events:
        event = speech_events[0]
        window = (event["start_us"], event["end_us"], f"manifest event {event['id']}")
    else:
        window = (LEAD_IN_US, duration - TAIL_US, "lead-in policy")
    start, end, source = window
    if not offset <= start < end <= duration:
        raise GenerationError(f"{fixture_id} speech window lies outside its audio track")
    return SpeechPlan(fixture_id, mode, script, language_segments(fixture_id, script), duration,
                      offset, start, end, source)


def speech_plans(manifest: dict) -> list[SpeechPlan]:
    plans = [speech_plan(fixture) for fixture in manifest["fixtures"]]
    return [plan for plan in plans if plan is not None]


def place(plan: SpeechPlan, utterance_frames: int) -> Placement:
    """Start the utterance at its window start and require it to finish inside it."""
    if utterance_frames <= 0:
        raise GenerationError(f"{plan.fixture_id} utterance is empty")
    start_frame = exact_frames(plan.window_start_us - plan.audio_offset_us)
    track_frames = exact_frames(plan.duration_us - plan.audio_offset_us)
    end_us = plan.window_start_us + frames_to_us_ceiling(utterance_frames)
    if end_us > plan.window_end_us or start_frame + utterance_frames > track_frames:
        raise GenerationError(
            f"{plan.fixture_id} utterance ({frames_to_us_ceiling(utterance_frames)} us) does not fit "
            f"its {plan.window_source} window {plan.window_start_us}-{plan.window_end_us} us; "
            "a reviewed recipe change is required")
    return Placement(plan.window_start_us, end_us, start_frame, track_frames)


def quantize(samples: Iterable[float]) -> tuple[array.array, int]:
    """Round floats in [-1, 1] to signed 16-bit PCM, counting clipped samples."""
    output = array.array("h")
    clipped = 0
    for sample in samples:
        value = round(sample * INT16_SCALE)
        if value > 32_767:
            value, clipped = 32_767, clipped + 1
        elif value < -32_768:
            value, clipped = -32_768, clipped + 1
        output.append(value)
    return output, clipped


def write_wav(path: Path, samples: array.array, rate: int = SAMPLE_RATE) -> None:
    """Write mono 16-bit little-endian PCM, refusing to replace an existing file."""
    data = array.array("h", samples)
    if sys.byteorder == "big":
        data.byteswap()
    with path.open("xb") as handle, wave.open(handle, "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(rate)
        output.writeframes(data.tobytes())


def read_wav(path: Path, rate: int = SAMPLE_RATE, maximum_frames: int = 60 * SAMPLE_RATE) -> array.array:
    with wave.open(str(path), "rb") as source:
        if (source.getnchannels(), source.getsampwidth(), source.getframerate()) != (1, 2, rate):
            raise GenerationError(f"{path.name} is not mono 16-bit {rate} Hz PCM")
        frames = source.getnframes()
        if frames > maximum_frames:
            raise GenerationError(f"{path.name} exceeds the bounded utterance length")
        data = array.array("h")
        data.frombytes(source.readframes(frames))
    if sys.byteorder == "big":
        data.byteswap()
    if len(data) != frames:
        raise GenerationError(f"{path.name} is truncated")
    return data


def rms(samples: Sequence[float]) -> float:
    return math.sqrt(math.fsum(sample * sample for sample in samples) / len(samples))


def office_noise(frames: int, seed: int = NOISE_SEED, rate: int = SAMPLE_RATE) -> list[float]:
    """Deterministic unscaled office noise: pink broadband floor plus keyboard clicks."""
    generator = random.Random(seed)
    noise = [0.0] * frames
    b0 = b1 = b2 = 0.0
    for index in range(frames):
        white = generator.random() * 2.0 - 1.0
        b0 = 0.99765 * b0 + white * 0.0990460
        b1 = 0.96300 * b1 + white * 0.2965164
        b2 = 0.57000 * b2 + white * 1.0526913
        noise[index] = b0 + b1 + b2 + white * 0.1848
    click_length = CLICK_LENGTH_US * rate // MICROSECONDS
    minimum = CLICK_INTERVAL_MIN_US * rate // MICROSECONDS
    span = CLICK_INTERVAL_SPAN_US * rate // MICROSECONDS
    position = minimum + int(generator.random() * span)
    while position + click_length <= frames:
        amplitude = CLICK_AMPLITUDE * (0.5 + generator.random())
        envelope = 1.0
        for offset in range(click_length):
            noise[position + offset] += (generator.random() * 2.0 - 1.0) * amplitude * envelope
            envelope *= CLICK_DECAY_PER_SAMPLE
        position += minimum + int(generator.random() * span)
    return noise


def build_track(plan: SpeechPlan, placement: Placement,
                utterance: array.array) -> tuple[array.array, dict | None]:
    """Lay the utterance on the stream-local audio timeline, adding noise when noisy."""
    start = placement.track_start_frame
    if not plan.noisy:
        track = array.array("h", bytes(2 * placement.track_frames))
        track[start:start + len(utterance)] = utterance
        return track, None
    speech = [sample / INT16_SCALE for sample in utterance]
    speech_level = rms(speech)
    if speech_level == 0.0:
        raise GenerationError(f"{plan.fixture_id} utterance is silent")
    noise = office_noise(placement.track_frames)
    noise_level = rms(noise)
    noise_gain = speech_level / (math.sqrt(SNR_POWER_RATIO) * noise_level)
    mixed = [value * noise_gain for value in noise]
    for index, value in enumerate(speech):
        mixed[start + index] += value
    peak = max(abs(value) for value in mixed)
    mix_gain = 1.0 if peak <= 1.0 else 1.0 / peak
    if mix_gain != 1.0:
        mixed = [value * mix_gain for value in mixed]
    track, clipped = quantize(mixed)
    if clipped:
        raise GenerationError(f"{plan.fixture_id} noisy mix clipped after peak limiting")
    record = {
        "recipe": NOISE_RECIPE,
        "seed": NOISE_SEED,
        "snr_db": SNR_DB,
        "snr_definition": "RMS of the whole clean utterance over RMS of the scaled noise track",
        "pink_filter": "Paul Kellet economy filter over uniform white noise",
        "click_interval_us": [CLICK_INTERVAL_MIN_US, CLICK_INTERVAL_MIN_US + CLICK_INTERVAL_SPAN_US],
        "click_length_us": CLICK_LENGTH_US,
        "click_decay_per_sample": CLICK_DECAY_PER_SAMPLE,
        "covers": "whole audio track",
        "speech_rms": speech_level,
        "unscaled_noise_rms": noise_level,
        "noise_gain": noise_gain,
        "mix_gain": mix_gain,
    }
    return track, record


def synthesize_utterance(plan: SpeechPlan, synthesizer: Synthesizer) -> tuple[array.array, int, list[dict]]:
    """Speak every segment, joined by a fixed pause, and record engine-side facts."""
    pause = exact_frames(INTER_SEGMENT_PAUSE_US)
    samples: list[float] = []
    records: list[dict] = []
    for index, segment in enumerate(plan.segments):
        if index:
            samples.extend([0.0] * pause)
        voice = VOICES[segment.language]
        result = synthesizer.synthesize(segment.text, voice)
        if not result.samples:
            raise GenerationError(f"{plan.fixture_id} segment {index} produced no audio")
        start_frame = len(samples)
        samples.extend(result.samples)
        records.append({
            "language": segment.language,
            "voice": voice.voice_id,
            "text": segment.text,
            "start_frame": start_frame,
            "frames": len(result.samples),
            "phonemes": list(result.phonemes),
            "words": None if result.words is None else [
                {"text": word.text, "start_us": word.start_us, "end_us": word.end_us} for word in result.words],
        })
    utterance, clipped = quantize(samples)
    if not any(utterance):
        raise GenerationError(f"{plan.fixture_id} utterance quantized to silence")
    return utterance, clipped, records


def utterance_path(fixture_id: str) -> str:
    return f"{UTTERANCE_DIRECTORY}/{fixture_id}-utterance.wav"


def resolved_distributions(reports: Sequence[Path]) -> list[dict]:
    """Summarize `pip install --report` files: every installed artifact and its digest."""
    resolved: dict[str, dict] = {}
    for report_path in reports:
        report = load_json(report_path)
        for item in report.get("install", []):
            metadata = item.get("metadata", {})
            download = item.get("download_info", {})
            hashes = download.get("archive_info", {}).get("hashes", {})
            digest = hashes.get("sha256")
            if digest is None:
                legacy = download.get("archive_info", {}).get("hash", "")
                digest = legacy.removeprefix("sha256=") if legacy.startswith("sha256=") else None
            name = str(metadata.get("name", "")).lower().replace("_", "-")
            if not name:
                raise GenerationError(f"{report_path.name} has an install entry without a name")
            resolved[name] = {"name": name, "version": metadata.get("version"),
                              "url": download.get("url"), "sha256": digest}
    return [resolved[name] for name in sorted(resolved)]


def synthesize(output: Path, synthesizer: Synthesizer, synthesis_facts: dict) -> dict:
    """Write every utterance WAV and a provenance file holding the synthesis section."""
    manifest = load_json(MANIFEST)
    (output / UTTERANCE_DIRECTORY).mkdir(parents=True, exist_ok=True)
    utterances = []
    for plan in speech_plans(manifest):
        utterance, clipped, segments = synthesize_utterance(plan, synthesizer)
        place(plan, len(utterance))  # Fail before writing anything that cannot be placed.
        relative = utterance_path(plan.fixture_id)
        write_wav(output / relative, utterance)
        utterances.append({"fixture": plan.fixture_id, "file": relative,
                           "sha256": sha256_file(output / relative),
                           "bytes": (output / relative).stat().st_size,
                           "frames": len(utterance), "clipped_samples": clipped, "segments": segments})
    record = {
        "generator": GENERATOR_ID,
        "purpose": "Test fixtures only. Kokoro is never a VSift runtime dependency.",
        "manifest_sha256": sha256_file(MANIFEST),
        "base_provenance_sha256": sha256_file(BASE_PROVENANCE),
        "synthesis": dict(synthesis_facts, utterances=utterances),
        "assembly": None,
    }
    with (output / PROVENANCE_NAME).open("xb") as handle:
        handle.write(json_bytes(record))
    return record


def base_fixtures(base_provenance: dict) -> dict[str, tuple[str, str, int, int]]:
    """Map fixture id to (file name, SHA-256, bytes, origin us) from the P04 record."""
    fixtures = {}
    for entry in base_provenance["fixtures"]:
        template = entry["command_template"]
        name = re.split(r"[\\/]", template[-1])[-1]
        origin = 0
        if "-output_ts_offset" in template:
            origin = int(Decimal(template[template.index("-output_ts_offset") + 1]) * MICROSECONDS)
        fixtures[entry["fixture"]] = (name, entry["sha256"], entry["bytes"], origin)
    return fixtures


def variant_name(fixture_id: str, base_name: str) -> str:
    return f"{fixture_id}-speech{PurePosixPath(base_name).suffix}"


def mux_argv(ffmpeg: str, base: str, track: str, output: str, container: str,
             audio_offset_us: int, origin_us: int) -> list[str]:
    """Copy the P04 video stream bit for bit and encode the placed speech track."""
    argv = [ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-n", "-i", base]
    if audio_offset_us:
        argv += ["-itsoffset", format_seconds(audio_offset_us)]
    argv += ["-i", track, "-map", "0:v:0", "-map", "1:a:0", "-c:v", "copy"]
    if container == ".mp4":
        argv += ["-c:a", "aac", "-b:a", "64k"]
    elif container == ".mkv":
        argv += ["-c:a", "pcm_s16le"]
    else:
        raise GenerationError(f"unreviewed container {container}")
    argv += ["-ar", str(OUTPUT_AUDIO_RATE), "-ac", "1", "-fflags", "+bitexact"]
    if origin_us:
        argv += ["-output_ts_offset", format_seconds(origin_us)]
    if container == ".mp4":
        argv += ["-movflags", "+faststart"]
    return argv + [output]


def run_ffmpeg(argv: list[str]) -> None:
    result = subprocess.run(argv, stdin=subprocess.DEVNULL, capture_output=True, timeout=120,
                            check=False, shell=False)
    if result.returncode != 0:
        raise GenerationError(f"FFmpeg returned {result.returncode}: "
                              f"{result.stderr[-2048:].decode(errors='replace')}")


def ffmpeg_version(ffmpeg: str) -> str:
    result = subprocess.run([ffmpeg, "-version"], stdin=subprocess.DEVNULL, capture_output=True,
                            timeout=15, check=True, shell=False)
    return result.stdout.decode(errors="replace").splitlines()[0][:250]


def assemble(output: Path, ffmpeg: str, base_directory: Path = GENERATED) -> dict:
    """Place, mix and mux every utterance recorded in the synthesis section."""
    manifest = load_json(MANIFEST)
    provenance_path = output / PROVENANCE_NAME
    record = load_json(provenance_path)
    if record["manifest_sha256"] != sha256_file(MANIFEST):
        raise GenerationError("synthesis record was made from different manifest truth")
    if record["base_provenance_sha256"] != sha256_file(BASE_PROVENANCE):
        raise GenerationError("synthesis record was made against a different P04 record")
    if record.get("assembly") is not None:
        raise GenerationError("assembly already recorded; assemble into a fresh output directory")
    plans = {plan.fixture_id: plan for plan in speech_plans(manifest)}
    utterances = {entry["fixture"]: entry for entry in record["synthesis"]["utterances"]}
    if utterances.keys() != plans.keys():
        raise GenerationError("synthesis record does not cover exactly the speech-bearing fixtures")
    bases = base_fixtures(load_json(BASE_PROVENANCE))
    variants = []
    with tempfile.TemporaryDirectory(prefix="vsift-p07-speech-") as temporary:
        for fixture_id, plan in plans.items():
            entry = utterances[fixture_id]
            if entry["file"] != utterance_path(fixture_id):
                raise GenerationError(f"{fixture_id} synthesis record names an unexpected utterance path")
            if [segment["text"] for segment in entry["segments"]] != [segment.text for segment in plan.segments]:
                raise GenerationError(f"{fixture_id} synthesized text differs from the manifest script")
            utterance_file = output / entry["file"]
            if sha256_file(utterance_file) != entry["sha256"]:
                raise GenerationError(f"{fixture_id} utterance differs from its synthesis record")
            utterance = read_wav(utterance_file)
            placement = place(plan, len(utterance))
            track, noise = build_track(plan, placement, utterance)
            track_path = Path(temporary) / f"{fixture_id}-track.wav"
            write_wav(track_path, track)
            base_name, base_sha256, base_bytes, origin = bases[fixture_id]
            base_path = base_directory / base_name
            if base_path.stat().st_size != base_bytes or sha256_file(base_path) != base_sha256:
                raise GenerationError(f"{base_name} differs from its P04 provenance")
            container = PurePosixPath(base_name).suffix
            name = variant_name(fixture_id, base_name)
            if name in {entry_name for entry_name, _, _, _ in bases.values()}:
                raise GenerationError(f"{name} would replace a P04 fixture")
            run_ffmpeg(mux_argv(ffmpeg, str(base_path), str(track_path), str(output / name), container,
                                plan.audio_offset_us, origin))
            words = []
            segments = []
            for segment in entry["segments"]:
                segment_start_us = placement.speech_start_us + segment["start_frame"] * MICROSECONDS // SAMPLE_RATE
                segments.append({"language": segment["language"], "start_us": segment_start_us,
                                 "end_us": segment_start_us + frames_to_us_ceiling(segment["frames"])})
                for word in segment["words"] or []:
                    words.append({"text": word["text"], "start_us": segment_start_us + word["start_us"],
                                  "end_us": segment_start_us + word["end_us"]})
            variants.append({
                "fixture": fixture_id,
                "mode": plan.mode,
                "file": name,
                "sha256": sha256_file(output / name),
                "bytes": (output / name).stat().st_size,
                "base_file": base_name,
                "base_sha256": base_sha256,
                "container_origin_us": origin,
                "audio_offset_us": plan.audio_offset_us,
                "speech_window": {"source": plan.window_source, "start_us": plan.window_start_us,
                                  "end_us": plan.window_end_us},
                "speech_start_us": placement.speech_start_us,
                "speech_end_us": placement.speech_end_us,
                "segments": segments,
                "tts_word_timings": words,
                "track_frames": placement.track_frames,
                "track_sha256": sha256_file(track_path),
                "noise": noise,
                "command_template": mux_argv("<FFMPEG>", f"<BASE>/{base_name}", "<TRACK>",
                                             f"<OUTPUT>/{name}", container, plan.audio_offset_us, origin),
            })
    record["assembly"] = {
        "ffmpeg_version": ffmpeg_version(ffmpeg),
        "ffmpeg_sha256": sha256_file(Path(ffmpeg)),
        "track_sample_rate": SAMPLE_RATE,
        "encoded_sample_rate": OUTPUT_AUDIO_RATE,
        "placement_policy": {"with_manifest_speech_event": "start at the event start; must end inside it",
                             "without_speech_event": {"lead_in_us": LEAD_IN_US, "tail_us": TAIL_US}},
        "timing_note": ("speech_start_us/speech_end_us are exact by construction; tts_word_timings come "
                        "from Kokoro's duration predictor (12.5 ms steps) and are engine-reported "
                        "generation facts, not independent truth; manifest.json remains the truth source"),
        "variants": variants,
    }
    replacement = provenance_path.with_suffix(".json.tmp")
    with replacement.open("xb") as handle:
        handle.write(json_bytes(record))
    os.replace(replacement, provenance_path)
    return record


class HttpsOnlyRedirects(urllib.request.HTTPRedirectHandler):
    """Follow a bounded number of redirects, and only to credential-free HTTPS URLs."""

    max_redirections = MAX_DOWNLOAD_REDIRECTS

    def redirect_request(self, request, fp, code, msg, headers, newurl):
        parsed = urllib.parse.urlparse(newurl)
        if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
            raise GenerationError("download redirected to a non-HTTPS or credential-bearing URL")
        return super().redirect_request(request, fp, code, msg, headers, newurl)


def check_pin(pin: PinnedFile, size: int, sha256: str, git_blob_sha1: str) -> None:
    if size != pin.size:
        raise GenerationError(f"{pin.path} size {size} differs from the reviewed {pin.size}")
    if pin.sha256 is not None and sha256 != pin.sha256:
        raise GenerationError(f"{pin.path} SHA-256 differs from the reviewed pin")
    if pin.git_blob_sha1 is not None and git_blob_sha1 != pin.git_blob_sha1:
        raise GenerationError(f"{pin.path} git blob SHA-1 differs from the reviewed pin")


def digests(path: Path, size: int) -> tuple[int, str, str]:
    """SHA-256 and git blob SHA-1 (the id Hugging Face publishes for small files)."""
    sha256 = hashlib.sha256()
    blob = hashlib.sha1(f"blob {size}\0".encode("ascii"))
    total = 0
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            total += len(block)
            sha256.update(block)
            blob.update(block)
    return total, sha256.hexdigest(), blob.hexdigest()


def verified_file(root: Path, pin: PinnedFile) -> tuple[Path, str]:
    path = root / pin.path
    size, sha256, blob = digests(path, pin.size)
    check_pin(pin, size, sha256, blob)
    return path, sha256


def download(root: Path, pin: PinnedFile) -> str:
    """Stream at most the reviewed byte count over HTTPS, then check the pin."""
    destination = root / pin.path
    destination.parent.mkdir(parents=True, exist_ok=True)
    opener = urllib.request.build_opener(HttpsOnlyRedirects())
    request = urllib.request.Request(pin.url, headers={"User-Agent": "vsift-p07-speech-fixtures/1"})
    received = 0
    try:
        with opener.open(request, timeout=60) as response, destination.open("xb") as sink:
            if urllib.parse.urlparse(response.geturl()).scheme != "https":
                raise GenerationError("download ended outside HTTPS")
            while chunk := response.read(1024 * 1024):
                received += len(chunk)
                if received > pin.size:
                    raise GenerationError(f"{pin.path} exceeds its reviewed size")
                sink.write(chunk)
    except urllib.error.URLError as error:
        raise GenerationError(f"HTTPS transfer of {pin.path} failed") from error
    return verified_file(root, pin)[1]


def fetch(assets: Path) -> dict:
    assets.mkdir(parents=True, exist_ok=True)
    files = [{"path": pin.path, "url": pin.url, "bytes": pin.size, "sha256": download(assets, pin),
              "pinned_by": "sha256" if pin.sha256 else "git-blob-sha1" if pin.git_blob_sha1 else
              "size only; digest recorded on first use"} for pin in FETCHED_FILES]
    record = {"kokoro_repository": KOKORO_REPOSITORY, "kokoro_revision": KOKORO_REVISION, "files": files}
    with (assets / FETCH_RECORD_NAME).open("xb") as handle:
        handle.write(json_bytes(record))
    return record


class KokoroSynthesizer:
    """Kokoro adapter. Imports PyTorch lazily so everything else stays standard library."""

    def __init__(self, assets: Path) -> None:
        import torch
        from kokoro import KModel

        self._torch = torch
        torch.set_num_threads(TORCH_THREADS)
        torch.use_deterministic_algorithms(True, warn_only=True)
        config_path, _ = verified_file(assets, KOKORO_CONFIG)
        weights_path, _ = verified_file(assets, KOKORO_WEIGHTS)
        self._voice_paths = {language: verified_file(assets, voice.file)[0] for language, voice in VOICES.items()}
        config = json.loads(config_path.read_text(encoding="utf-8"))
        # eval() matters: nn.Module starts in training mode, where dropout is random.
        self._model = KModel(repo_id=KOKORO_REPOSITORY, config=config, model=str(weights_path)).to("cpu").eval()
        self._pipelines: dict[str, object] = {}

    def synthesize(self, text: str, voice: Voice) -> SynthesizedSegment:
        from kokoro import KPipeline

        pipeline = self._pipelines.get(voice.lang_code)
        if pipeline is None:
            pipeline = KPipeline(lang_code=voice.lang_code, repo_id=KOKORO_REPOSITORY, model=self._model)
            self._pipelines[voice.lang_code] = pipeline
        self._torch.manual_seed(SEED)
        samples: list[float] = []
        phonemes: list[str] = []
        words: list[Word] = []
        timed = True
        for result in pipeline(text, voice=str(self._voice_paths[voice.language]), speed=SPEED,
                               split_pattern=None):
            audio = result.output.audio if result.output is not None else None
            if audio is None:
                raise GenerationError("Kokoro returned a chunk without audio")
            chunk_offset_us = len(samples) * MICROSECONDS // SAMPLE_RATE
            phonemes.append(str(result.phonemes))
            if result.tokens is None:
                timed = False
            else:
                for token in result.tokens:
                    spoken = any(character.isalnum() for character in token.text)
                    if spoken and token.start_ts is not None and token.end_ts is not None:
                        words.append(Word(token.text, chunk_offset_us + round(token.start_ts * MICROSECONDS),
                                          chunk_offset_us + round(token.end_ts * MICROSECONDS)))
            samples.extend(float(value) for value in audio.detach().cpu().reshape(-1).tolist())
        return SynthesizedSegment(tuple(samples), tuple(phonemes), tuple(words) if timed else None)


def installed_distributions() -> dict[str, str]:
    from importlib import metadata

    installed = {}
    for name, expected in PINNED_DISTRIBUTIONS.items():
        try:
            installed[name] = metadata.version(name)
        except metadata.PackageNotFoundError as error:
            raise GenerationError(f"pinned distribution {name} is not installed") from error
        if installed[name] != expected:
            raise GenerationError(f"{name} {installed[name]} is installed; the recipe pins {expected}")
    return installed


def espeak_version() -> str:
    """Best-effort phonemizer backend version for provenance; never affects output."""
    try:
        import misaki.espeak  # noqa: F401 - points phonemizer at the bundled eSpeak NG library.
        from phonemizer.backend import EspeakBackend

        version = EspeakBackend.version()
        return ".".join(str(part) for part in version) if isinstance(version, tuple) else str(version)
    except Exception as error:  # Recorded, not hidden: this probe is provenance only.
        return f"unavailable ({type(error).__name__})"


def synthesis_facts(assets: Path, reports: Sequence[Path]) -> dict:
    fetch_record = load_json(assets / FETCH_RECORD_NAME)
    return {
        "engine": "Kokoro text-to-speech (test fixture generation only)",
        "model": {"repository": KOKORO_REPOSITORY, "revision": KOKORO_REVISION, "licence": "Apache-2.0",
                  "files": [entry for entry in fetch_record["files"] if entry["path"].startswith(KOKORO_DIRECTORY)]},
        "voices": {language: {"voice": voice.voice_id, "kokoro_lang_code": voice.lang_code}
                   for language, voice in VOICES.items()},
        "sentence_languages": {fixture: list(languages) for fixture, languages in SENTENCE_LANGUAGES.items()},
        "speed": SPEED,
        "seed": SEED,
        "seed_policy": "torch.manual_seed reset before every segment",
        "torch_threads": TORCH_THREADS,
        "deterministic_algorithms": True,
        "sample_rate": SAMPLE_RATE,
        "sample_format": "mono signed 16-bit little-endian PCM WAV, round(x * 32767)",
        "inter_segment_pause_us": INTER_SEGMENT_PAUSE_US,
        "environment": {
            "python": platform.python_version(),
            "platform": platform.platform(),
            "machine": platform.machine(),
            "distributions": installed_distributions(),
            "espeak_ng": espeak_version(),
            "spacy_model_wheel": next(entry for entry in fetch_record["files"]
                                      if entry["path"] == SPACY_MODEL_WHEEL.path),
        },
        "resolved_distributions": resolved_distributions(reports),
    }


def main(argv: Sequence[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    fetch_command = commands.add_parser("fetch", help="download pinned model, voices and spaCy wheel")
    fetch_command.add_argument("--assets", required=True, type=Path)
    synthesize_command = commands.add_parser("synthesize", help="speak the frozen scripts with Kokoro")
    synthesize_command.add_argument("--assets", required=True, type=Path)
    synthesize_command.add_argument("--output-dir", required=True, type=Path)
    synthesize_command.add_argument("--pip-report", action="append", default=[], type=Path)
    assemble_command = commands.add_parser("assemble", help="place, mix and mux the utterances")
    assemble_command.add_argument("--output-dir", required=True, type=Path)
    assemble_command.add_argument("--ffmpeg", required=True, help="explicit trusted FFmpeg executable path")
    assemble_command.add_argument("--base-dir", type=Path, default=GENERATED)
    args = parser.parse_args(argv)
    if args.command == "fetch":
        fetch(args.assets.resolve())
        print(f"Fetched {len(FETCHED_FILES)} pinned files")
    elif args.command == "synthesize":
        assets = args.assets.resolve(strict=True)
        facts = synthesis_facts(assets, args.pip_report)  # Checks the pinned environment first.
        record = synthesize(args.output_dir.resolve(), KokoroSynthesizer(assets), facts)
        print(f"Synthesized {len(record['synthesis']['utterances'])} utterances")
    else:
        ffmpeg = str(Path(args.ffmpeg).resolve(strict=True))
        record = assemble(args.output_dir.resolve(strict=True), ffmpeg, args.base_dir.resolve(strict=True))
        print(f"Assembled {len(record['assembly']['variants'])} speech variants")


if __name__ == "__main__":
    main()
