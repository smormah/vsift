"""Independently verify P07 speech fixtures against manifest truth and their record.

This verifier shares no code with `tools/generate_p07_speech.py`. It derives the set
of speech-bearing fixtures, their offsets and their speech windows from
`fixtures/corpus/manifest.json` itself, checks every recorded byte count and SHA-256,
reads the utterance WAV headers with the standard library, inspects each speech
variant with FFprobe, proves the video stream is packet-identical to the P04 source
fixture, and decodes the audio to check that speech energy lies where the record says
and inside the manifest's speech window. It needs no text-to-speech model.

Run it against a second generation's output with the first run's record
(`--provenance`) to prove that two runs produced identical bytes.
"""

from __future__ import annotations

import argparse
import array
from decimal import Decimal
import hashlib
import json
from pathlib import Path, PurePosixPath
import subprocess
import sys
import wave


ROOT = Path(__file__).resolve().parents[1]
CORPUS = ROOT / "fixtures" / "corpus"
GENERATED = CORPUS / "generated"
MANIFEST = CORPUS / "manifest.json"
BASE_PROVENANCE = GENERATED / "provenance.json"

VERIFIER_ID = "tools/verify_p07_speech.py v1"
SPEECH_MODES = {"clean-synthetic": "clean", "noisy-synthetic": "noisy", "offset-synthetic": "clean"}
UTTERANCE_RATE = 24_000
DECODE_RATE = 16_000
WINDOW_SAMPLES = 320  # 20 ms at the decode rate.
WINDOW_US = 20_000
EDGE_TOLERANCE_US = 100_000  # AAC frames (64 ms at 16 kHz) spread energy at edges.
DURATION_TOLERANCE_US = 50_000
FULL_SCALE_POWER = 32_768.0 ** 2
ACTIVE_POWER = FULL_SCALE_POWER * 1e-5  # -50 dBFS: speech-active window in a clean track.
NOISE_FLOOR_POWER = FULL_SCALE_POWER * 1e-6  # -60 dBFS: noise must be present everywhere.
MINIMUM_ACTIVE_FRACTION = 0.25
MINIMUM_NOISY_CONTRAST = 10 ** 0.4  # Speech span must be at least 4 dB above the rest.
MINIMUM_NOISE_COVERAGE = 0.95


class VerificationError(AssertionError):
    """A speech fixture differs from its record or from manifest truth."""


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def run(argv: list[str], maximum: int) -> bytes:
    result = subprocess.run(argv, stdin=subprocess.DEVNULL, capture_output=True, timeout=60,
                            check=False, shell=False)
    if result.returncode != 0 or len(result.stdout) > maximum:
        raise VerificationError(f"independent provider check failed: code={result.returncode}, "
                                f"bytes={len(result.stdout)}")
    return result.stdout


def contained(root: Path, relative: str) -> Path:
    """Resolve a recorded relative path, refusing anything that escapes `root`."""
    posix = PurePosixPath(relative)
    if posix.is_absolute() or ".." in posix.parts or "\\" in relative or ":" in relative:
        raise VerificationError(f"recorded path {relative!r} is not a safe relative path")
    return root / Path(*posix.parts)


def check_file(path: Path, entry: dict) -> None:
    if not path.is_file() or path.stat().st_size != entry["bytes"] or sha256(path) != entry["sha256"]:
        raise VerificationError(f"{path.name} hash/size differs from the generation record")


def speech_fixtures(manifest: dict) -> dict[str, dict]:
    fixtures = {}
    for fixture in manifest["fixtures"]:
        mode = fixture["audio"]["mode"]
        if mode == "none":
            continue
        if mode not in SPEECH_MODES:
            raise VerificationError(f"{fixture['id']} has an audio mode this verifier does not know")
        fixtures[fixture["id"]] = fixture
    return fixtures


def speech_window(fixture: dict) -> tuple[int, int]:
    """The manifest's constraint on where speech may be: its speech event, else the clip."""
    events = [event for event in fixture["events"] if event["kind"] == "speech"]
    if len(events) > 1:
        raise VerificationError(f"{fixture['id']} has several speech events")
    if events:
        return events[0]["start_us"], events[0]["end_us"]
    return 0, fixture["duration_us"]


def check_utterance(path: Path, entry: dict, fixture: dict) -> dict:
    check_file(path, entry)
    with wave.open(str(path), "rb") as source:
        layout = (source.getnchannels(), source.getsampwidth(), source.getframerate())
        if layout != (1, 2, UTTERANCE_RATE):
            raise VerificationError(f"{path.name} is not mono 16-bit {UTTERANCE_RATE} Hz PCM")
        frames = source.getnframes()
        samples = array.array("h")
        samples.frombytes(source.readframes(frames))
    if sys.byteorder == "big":
        samples.byteswap()
    if frames != entry["frames"] or len(samples) != frames:
        raise VerificationError(f"{path.name} frame count differs from the generation record")
    if not any(samples):
        raise VerificationError(f"{path.name} is silent")
    spoken = " ".join(segment["text"] for segment in entry["segments"])
    if spoken != fixture["audio"]["script"]:
        raise VerificationError(f"{fixture['id']} utterance text differs from the frozen script")
    return {"fixture": fixture["id"], "file": entry["file"], "frames": frames,
            "duration_us": frames * 1_000_000 // UTTERANCE_RATE}


def probe(ffprobe: str, path: Path) -> dict:
    return json.loads(run([ffprobe, "-v", "error", "-show_entries",
                           "format=start_time,duration:stream=index,codec_type,codec_name,start_time,"
                           "width,height,sample_rate,channels", "-of", "json", str(path)], 128 * 1024))


def video_stream_hash(ffmpeg: str, path: Path) -> str:
    return run([ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-i", str(path),
                "-map", "0:v:0", "-c", "copy", "-f", "streamhash", "-hash", "sha256", "-"], 4096).decode().strip()


def decoded_audio(ffmpeg: str, path: Path, duration_us: int) -> array.array:
    maximum = (duration_us // 1_000_000 + 2) * DECODE_RATE * 2
    raw = run([ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-i", str(path), "-map", "0:a:0",
               "-f", "s16le", "-acodec", "pcm_s16le", "-ac", "1", "-ar", str(DECODE_RATE), "-"], maximum)
    samples = array.array("h")
    samples.frombytes(raw[:len(raw) - len(raw) % 2])
    if sys.byteorder == "big":
        samples.byteswap()
    return samples


def window_powers(samples: array.array) -> list[float]:
    return [sum(value * value for value in samples[start:start + WINDOW_SAMPLES]) / WINDOW_SAMPLES
            for start in range(0, len(samples) - WINDOW_SAMPLES + 1, WINDOW_SAMPLES)]


def check_clean_energy(fixture_id: str, powers: list[float], audio_start_us: int,
                       speech_start_us: int, speech_end_us: int) -> dict:
    """Every active window lies within the recorded span, which is substantially active."""
    active = [index for index, power in enumerate(powers) if power >= ACTIVE_POWER]
    if not active:
        raise VerificationError(f"{fixture_id} speech variant has no speech energy")
    first_us = audio_start_us + active[0] * WINDOW_US
    last_us = audio_start_us + (active[-1] + 1) * WINDOW_US
    if first_us < speech_start_us - EDGE_TOLERANCE_US or last_us > speech_end_us + EDGE_TOLERANCE_US:
        raise VerificationError(f"{fixture_id} speech energy {first_us}-{last_us} us lies outside "
                                f"its recorded span {speech_start_us}-{speech_end_us} us")
    fraction = len(active) * WINDOW_US / (speech_end_us - speech_start_us)
    if fraction < MINIMUM_ACTIVE_FRACTION:
        raise VerificationError(f"{fixture_id} recorded speech span is mostly silent")
    return {"first_active_us": first_us, "last_active_us": last_us, "active_fraction": round(fraction, 3)}


def check_noisy_energy(fixture_id: str, powers: list[float], audio_start_us: int,
                       speech_start_us: int, speech_end_us: int) -> dict:
    """Noise covers the whole track and the recorded span stands clearly above it."""
    coverage = sum(1 for power in powers if power >= NOISE_FLOOR_POWER) / len(powers)
    if coverage < MINIMUM_NOISE_COVERAGE:
        raise VerificationError(f"{fixture_id} noise does not cover the audio track")
    inside, outside = [], []
    for index, power in enumerate(powers):
        start = audio_start_us + index * WINDOW_US
        if start >= speech_start_us and start + WINDOW_US <= speech_end_us:
            inside.append(power)
        elif start + WINDOW_US <= speech_start_us or start >= speech_end_us:
            outside.append(power)
    if not inside or not outside:
        raise VerificationError(f"{fixture_id} has no noise-only region to compare with speech")
    contrast = (sum(inside) / len(inside)) / (sum(outside) / len(outside))
    if contrast < MINIMUM_NOISY_CONTRAST:
        raise VerificationError(f"{fixture_id} speech span does not stand above the noise floor")
    return {"noise_coverage": round(coverage, 3), "speech_to_noise_power_ratio": round(contrast, 2)}


def check_variant(ffmpeg: str, ffprobe: str, generated: Path, base_directory: Path, entry: dict,
                  fixture: dict, utterance: dict, base_entry: dict) -> dict:
    fixture_id = fixture["id"]
    base_name = PurePosixPath(base_entry["command_template"][-1].replace("\\", "/")).name
    if entry["base_file"] != base_name or entry["file"] != f"{fixture_id}-speech{PurePosixPath(base_name).suffix}":
        raise VerificationError(f"{fixture_id} speech variant is not named after its P04 source")
    path = contained(generated, entry["file"])
    check_file(path, entry)
    base_path = base_directory / base_name
    check_file(base_path, base_entry)
    info = probe(ffprobe, path)
    base_info = probe(ffprobe, base_path)
    streams = info["streams"]
    if [stream["codec_type"] for stream in streams] != ["video", "audio"]:
        raise VerificationError(f"{fixture_id} speech variant must hold exactly one video and one audio stream")
    video, audio = streams
    if (video["width"], video["height"]) != (fixture["resolution"]["width"], fixture["resolution"]["height"]):
        raise VerificationError(f"{fixture_id} dimensions differ from frozen truth")
    if video_stream_hash(ffmpeg, path) != video_stream_hash(ffmpeg, base_path):
        raise VerificationError(f"{fixture_id} video packets differ from the P04 source fixture")
    expected_codec = "pcm_s16le" if base_name.endswith(".mkv") else "aac"
    if (audio["codec_name"], int(audio["sample_rate"]), audio["channels"]) != (expected_codec, DECODE_RATE, 1):
        raise VerificationError(f"{fixture_id} audio is not mono {DECODE_RATE} Hz {expected_codec}")
    origin = Decimal(info["format"].get("start_time", "0"))
    if origin != Decimal(base_info["format"].get("start_time", "0")):
        raise VerificationError(f"{fixture_id} container origin differs from the P04 source")
    offset_us = int((Decimal(audio["start_time"]) - origin) * 1_000_000)
    if offset_us != fixture["audio"].get("offset_us", 0) or offset_us != entry["audio_offset_us"]:
        raise VerificationError(f"{fixture_id} audio offset {offset_us} us differs from frozen truth")
    # FFprobe reports a Matroska duration from time zero, an MP4 duration from its start.
    container_duration = Decimal(info["format"]["duration"])
    if base_name.endswith(".mkv"):
        container_duration -= origin
    duration_us = int(container_duration * 1_000_000)
    if abs(duration_us - fixture["duration_us"]) > DURATION_TOLERANCE_US:
        raise VerificationError(f"{fixture_id} duration {duration_us} us differs from frozen truth")
    start_us, end_us = entry["speech_start_us"], entry["speech_end_us"]
    window_start, window_end = speech_window(fixture)
    if not (max(window_start, offset_us) <= start_us < end_us <= min(window_end, fixture["duration_us"])):
        raise VerificationError(f"{fixture_id} recorded speech span lies outside the manifest window")
    expected_end = start_us - (-utterance["frames"] * 1_000_000 // UTTERANCE_RATE)
    if end_us != expected_end:
        raise VerificationError(f"{fixture_id} recorded speech span does not match the utterance length")
    noisy = SPEECH_MODES[fixture["audio"]["mode"]] == "noisy"
    if noisy != (entry["noise"] is not None) or entry["mode"] != fixture["audio"]["mode"]:
        raise VerificationError(f"{fixture_id} noise record does not match its manifest audio mode")
    powers = window_powers(decoded_audio(ffmpeg, path, fixture["duration_us"]))
    energy = (check_noisy_energy if noisy else check_clean_energy)(fixture_id, powers, offset_us, start_us, end_us)
    return {"fixture": fixture_id, "file": entry["file"], "sha256": entry["sha256"],
            "video_packets_match": base_name, "audio_codec": audio["codec_name"],
            "origin_us": int(origin * 1_000_000), "audio_offset_us": offset_us, "duration_us": duration_us,
            "speech_span_us": [start_us, end_us], "manifest_window_us": [window_start, window_end],
            "energy": energy}


def verify(ffmpeg: str, ffprobe: str, generated: Path, provenance_path: Path, base_directory: Path) -> dict:
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    record = json.loads(provenance_path.read_text(encoding="utf-8"))
    base = json.loads(BASE_PROVENANCE.read_text(encoding="utf-8"))
    if record["manifest_sha256"] != sha256(MANIFEST):
        raise VerificationError("speech record points at different frozen truth")
    if record["base_provenance_sha256"] != sha256(BASE_PROVENANCE):
        raise VerificationError("speech record points at a different P04 generation record")
    if record.get("assembly") is None:
        raise VerificationError("speech record has no assembly section")
    fixtures = speech_fixtures(manifest)
    utterances = {entry["fixture"]: entry for entry in record["synthesis"]["utterances"]}
    variants = {entry["fixture"]: entry for entry in record["assembly"]["variants"]}
    if utterances.keys() != fixtures.keys() or variants.keys() != fixtures.keys():
        raise VerificationError("speech record does not cover exactly the manifest's speech-bearing fixtures")
    base_entries = {entry["fixture"]: entry for entry in base["fixtures"]}
    utterance_checks, variant_checks = [], []
    for fixture_id in sorted(fixtures):
        entry = utterances[fixture_id]
        if entry["file"] != f"speech/{fixture_id}-utterance.wav":
            raise VerificationError(f"{fixture_id} utterance has an unexpected path")
        utterance_checks.append(check_utterance(contained(generated, entry["file"]), entry, fixtures[fixture_id]))
        variant_checks.append(check_variant(ffmpeg, ffprobe, generated, base_directory, variants[fixture_id],
                                            fixtures[fixture_id], entry, base_entries[fixture_id]))
    return {"verifier": VERIFIER_ID, "status": "passed", "manifest_sha256": record["manifest_sha256"],
            "provenance_sha256": sha256(provenance_path), "utterances": utterance_checks,
            "variants": variant_checks}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--ffmpeg", required=True)
    parser.add_argument("--ffprobe", required=True)
    parser.add_argument("--generated-dir", type=Path, default=GENERATED)
    parser.add_argument("--provenance", type=Path, help="defaults to <generated-dir>/speech-provenance.json")
    parser.add_argument("--base-dir", type=Path, default=GENERATED, help="directory holding the P04 fixtures")
    parser.add_argument("--report", type=Path, help="defaults to <generated-dir>/speech-verification.json")
    args = parser.parse_args()
    generated = args.generated_dir.resolve(strict=True)
    provenance = (args.provenance or generated / "speech-provenance.json").resolve(strict=True)
    report = verify(str(Path(args.ffmpeg).resolve(strict=True)), str(Path(args.ffprobe).resolve(strict=True)),
                    generated, provenance, args.base_dir.resolve(strict=True))
    destination = args.report or generated / "speech-verification.json"
    destination.write_bytes((json.dumps(report, indent=2) + "\n").encode("utf-8"))
    print(f"Verified {len(report['utterances'])} utterances and {len(report['variants'])} speech variants")


if __name__ == "__main__":
    main()
