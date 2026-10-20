"""Independently verify the P07 noisy-speech set (issue #150) against its truth and record.

This verifier shares no code with `tools/generate_p07_noisy_speech.py` or
`tools/generate_p07_speech.py`. It reads the passages and levels from
`fixtures/corpus/noisy-speech/manifest.json` itself, checks every recorded byte count
and SHA-256, reads the utterance WAV headers with the standard library, inspects each
clip with FFprobe, and decodes each clip to check, from the audio alone, that:

- a clean clip is silent outside its recorded speech span;
- a noisy clip has noise over its whole length and a signal-to-noise ratio, estimated
  from its noise-only lead-in and tail, within 3 dB of its level; one passage's
  estimates fall level by level;
- a noise-only clip has noise over its whole length at the noise power its level
  implies for speech of the set's pooled level, within 3 dB.

It needs no text-to-speech model. Run it against a second generation's output with the
first run's record (`--provenance`) to prove that two runs produced identical bytes.

The utterance WAVs are not committed (maintainer decision, 2026-10-20): fetch them with
`tools/fetch_p07_noisy_utterances.py` and pass their folder as `--utterances-dir`.
"""

from __future__ import annotations

import argparse
import array
from decimal import Decimal
import hashlib
import json
import math
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import wave


ROOT = Path(__file__).resolve().parents[1]
SET_DIRECTORY = ROOT / "fixtures" / "corpus" / "noisy-speech"
MANIFEST = SET_DIRECTORY / "manifest.json"
GENERATED = SET_DIRECTORY / "generated"

VERIFIER_ID = "tools/verify_p07_noisy_speech.py v1"
PROVENANCE_NAME = "noisy-speech-provenance.json"
REPORT_NAME = "noisy-speech-verification.json"
UTTERANCE_RATE = 24_000
DECODE_RATE = 16_000
WINDOW_SAMPLES = 320  # 20 ms at the decode rate.
WINDOW_US = 20_000
EDGE_TOLERANCE_US = 100_000  # AAC frames spread energy at the edges of the speech span.
DURATION_TOLERANCE_US = 100_000
ONE_WINDOW_US = 30_000_000  # The R0 recognition window every clip must fit.
FULL_SCALE_POWER = 32_768.0 ** 2
ACTIVE_POWER = FULL_SCALE_POWER * 1e-5  # -50 dBFS: speech-active window in a clean track.
NOISE_FLOOR_POWER = FULL_SCALE_POWER * 1e-7  # -70 dBFS: noise must be present everywhere.
MINIMUM_NOISE_COVERAGE = 0.95
MINIMUM_ACTIVE_FRACTION = 0.25
SNR_TOLERANCE = 10 ** 0.3  # 3 dB either way.


class VerificationError(AssertionError):
    """A clip differs from its record or from the set's truth."""


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


def read_utterance(path: Path, entry: dict, passage: dict) -> array.array:
    check_file(path, entry)
    with wave.open(str(path), "rb") as source:
        if (source.getnchannels(), source.getsampwidth(), source.getframerate()) != (1, 2, UTTERANCE_RATE):
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
    if " ".join(sentence["text"] for sentence in entry["sentences"]) != passage["script"]:
        raise VerificationError(f"{passage['id']} utterance text differs from the frozen script")
    if entry["language"] != passage["language"]:
        raise VerificationError(f"{passage['id']} was spoken in a language its truth does not name")
    return samples


def probe(ffprobe: str, path: Path) -> dict:
    return json.loads(run([ffprobe, "-v", "error", "-show_entries",
                           "format=duration:stream=codec_type,codec_name,sample_rate,channels",
                           "-of", "json", str(path)], 64 * 1024))


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


def split_powers(powers: list[float], start_us: int, end_us: int) -> tuple[list[float], list[float]]:
    """Windows wholly inside the speech span and wholly outside it, edges excluded."""
    inside, outside = [], []
    for index, power in enumerate(powers):
        window_start = index * WINDOW_US
        window_end = window_start + WINDOW_US
        if window_start >= start_us + EDGE_TOLERANCE_US and window_end <= end_us - EDGE_TOLERANCE_US:
            inside.append(power)
        elif window_end <= start_us - EDGE_TOLERANCE_US or window_start >= end_us + EDGE_TOLERANCE_US:
            outside.append(power)
    return inside, outside


def check_coverage(clip: str, powers: list[float]) -> float:
    coverage = sum(1 for power in powers if power >= NOISE_FLOOR_POWER) / len(powers)
    if coverage < MINIMUM_NOISE_COVERAGE:
        raise VerificationError(f"{clip} noise does not cover the clip")
    return coverage


def check_clean(clip: str, powers: list[float], start_us: int, end_us: int) -> dict:
    active = [index for index, power in enumerate(powers) if power >= ACTIVE_POWER]
    if not active:
        raise VerificationError(f"{clip} has no speech energy")
    first_us, last_us = active[0] * WINDOW_US, (active[-1] + 1) * WINDOW_US
    if first_us < start_us - EDGE_TOLERANCE_US or last_us > end_us + EDGE_TOLERANCE_US:
        raise VerificationError(f"{clip} speech energy {first_us}-{last_us} us lies outside its recorded "
                                f"span {start_us}-{end_us} us")
    fraction = len(active) * WINDOW_US / (end_us - start_us)
    if fraction < MINIMUM_ACTIVE_FRACTION:
        raise VerificationError(f"{clip} recorded speech span is mostly silent")
    return {"first_active_us": first_us, "last_active_us": last_us, "active_fraction": round(fraction, 3)}


def check_noisy(clip: str, powers: list[float], start_us: int, end_us: int, snr_db: int) -> dict:
    coverage = check_coverage(clip, powers)
    inside, outside = split_powers(powers, start_us, end_us)
    if not inside or not outside:
        raise VerificationError(f"{clip} has no noise-only region to compare with speech")
    noise = sum(outside) / len(outside)
    estimate = (sum(inside) / len(inside) - noise) / noise
    expected = 10 ** (snr_db / 10)
    if not expected / SNR_TOLERANCE <= estimate <= expected * SNR_TOLERANCE:
        raise VerificationError(f"{clip} signal-to-noise estimate {10 * math.log10(max(estimate, 1e-12)):.1f} dB "
                                f"is not within 3 dB of {snr_db} dB")
    return {"noise_coverage": round(coverage, 3), "estimated_snr_db": round(10 * math.log10(estimate), 2)}


def check_noise_only(clip: str, powers: list[float], expected_power: float) -> dict:
    coverage = check_coverage(clip, powers)
    power = sum(powers) / len(powers)
    if not expected_power / SNR_TOLERANCE <= power <= expected_power * SNR_TOLERANCE:
        raise VerificationError(f"{clip} noise power is not within 3 dB of its level")
    return {"noise_coverage": round(coverage, 3),
            "noise_power_relative_db": round(10 * math.log10(power / expected_power), 2)}


def check_clip_container(ffprobe: str, path: Path, clip: str, track_frames: int) -> int:
    info = probe(ffprobe, path)
    streams = info["streams"]
    if [stream["codec_type"] for stream in streams] != ["audio"]:
        raise VerificationError(f"{clip} must hold exactly one audio stream and nothing else")
    audio = streams[0]
    if (audio["codec_name"], int(audio["sample_rate"]), audio["channels"]) != ("aac", DECODE_RATE, 1):
        raise VerificationError(f"{clip} is not mono {DECODE_RATE} Hz AAC")
    duration_us = int(Decimal(info["format"]["duration"]) * 1_000_000)
    expected_us = track_frames * 1_000_000 // UTTERANCE_RATE
    if abs(duration_us - expected_us) > DURATION_TOLERANCE_US or duration_us >= ONE_WINDOW_US:
        raise VerificationError(f"{clip} lasts {duration_us} us; its track lasts {expected_us} us")
    return duration_us


CLIP_NAME = re.compile(r"^[A-Za-z0-9-]+\.m4a$")


RELEASE_PREFIX = "https://github.com/smormah/vsift/releases/download/test-media-"


def check_release(record: dict, passages: dict) -> None:
    """When the record names release assets, they cover exactly the passages, safely named."""
    release = record.get("utterance_release")
    if release is None:
        return
    if not release["base_url"].startswith(RELEASE_PREFIX) or set(release["assets"]) != set(passages):
        raise VerificationError("the record's release assets do not name exactly the passages")
    for passage, asset in release["assets"].items():
        if asset != f"noisy-speech-{passage}-utterance.wav":
            raise VerificationError(f"{passage} release asset has an unexpected name")


def verify(ffmpeg: str, ffprobe: str, generated: Path, provenance_path: Path,
           utterances_dir: Path | None = None) -> dict:
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    record = json.loads(provenance_path.read_text(encoding="utf-8"))
    if record["manifest_sha256"] != sha256(MANIFEST) or record.get("set_id") != manifest["set_id"]:
        raise VerificationError("the record points at different truth")
    if record.get("assembly") is None:
        raise VerificationError("the record has no assembly section")
    passages = {passage["id"]: passage for passage in manifest["passages"]}
    levels = {level["id"]: level["snr_db"] for level in manifest["levels"]}
    noisy_levels = [level for level, snr in levels.items() if snr is not None]
    utterances = {entry["passage"]: entry for entry in record["synthesis"]["utterances"]}
    if list(utterances) != list(passages):
        raise VerificationError("the record does not cover exactly the manifest's passages")
    check_release(record, passages)
    speech_root = generated if utterances_dir is None else utterances_dir
    expected_clips = [f"{passage}-{level}" for passage in passages for level in levels]
    expected_clips += [f"noise-only-{level}" for level in noisy_levels]
    clips = {entry["clip"]: entry for entry in record["assembly"]["clips"]}
    if sorted(clips) != sorted(expected_clips) or len(clips) != len(record["assembly"]["clips"]):
        raise VerificationError("the record does not hold exactly one clip per passage and level, "
                                "and one noise-only clip per noisy level")
    samples = {}
    utterance_checks = []
    for passage_id, passage in passages.items():
        entry = utterances[passage_id]
        if entry["file"] != f"speech/{passage_id}-utterance.wav":
            raise VerificationError(f"{passage_id} utterance has an unexpected path")
        samples[passage_id] = read_utterance(contained(speech_root, entry["file"]), entry, passage)
        utterance_checks.append({"passage": passage_id, "language": passage["language"],
                                 "frames": entry["frames"],
                                 "duration_us": entry["frames"] * 1_000_000 // UTTERANCE_RATE})
    total_frames = sum(len(values) for values in samples.values())
    pooled_power = math.fsum(math.fsum(float(value) * value for value in values)
                             for values in samples.values()) / total_frames
    clip_checks = []
    estimates: dict[str, list[tuple[int, float]]] = {}
    for name in expected_clips:
        entry = clips[name]
        snr_db = levels[entry["level"]]
        if entry["snr_db"] != snr_db or entry["file"] != f"{name}.m4a" or not CLIP_NAME.match(entry["file"]):
            raise VerificationError(f"{name} names a file or level its truth does not")
        path = contained(generated, entry["file"])
        check_file(path, entry)
        duration_us = check_clip_container(ffprobe, path, name, entry["track_frames"])
        powers = window_powers(decoded_audio(ffmpeg, path, duration_us))
        if entry["passage"] is None:
            if snr_db is None or entry["noise"] is None:
                raise VerificationError(f"{name} is a noise-only clip without noise")
            # A peak limit scales the whole clip, so its recorded gain scales the expectation.
            gain = entry["noise"]["mix_gain"]
            if not 0.0 < gain <= 1.0:
                raise VerificationError(f"{name} records an impossible mix gain")
            energy = check_noise_only(name, powers, pooled_power * gain * gain / (10 ** (snr_db / 10)))
        else:
            passage_id = entry["passage"]
            frames = utterances[passage_id]["frames"]
            start_us, end_us = entry["speech_start_us"], entry["speech_end_us"]
            if end_us != start_us - (-frames * 1_000_000 // UTTERANCE_RATE) or start_us <= 0 or end_us >= duration_us:
                raise VerificationError(f"{name} recorded speech span does not match its utterance and lead-in")
            if entry["language"] != passages[passage_id]["language"]:
                raise VerificationError(f"{name} names a language its truth does not")
            if snr_db is None:
                if entry["noise"] is not None:
                    raise VerificationError(f"{name} is a clean clip with noise")
                energy = check_clean(name, powers, start_us, end_us)
            else:
                energy = check_noisy(name, powers, start_us, end_us, snr_db)
                estimates.setdefault(passage_id, []).append((snr_db, energy["estimated_snr_db"]))
        clip_checks.append({"clip": name, "file": entry["file"], "sha256": entry["sha256"],
                            "duration_us": duration_us, "level": entry["level"], "energy": energy})
    for passage_id, pairs in estimates.items():
        ordered = [estimate for _, estimate in sorted(pairs, reverse=True)]
        if ordered != sorted(ordered, reverse=True):
            raise VerificationError(f"{passage_id} signal-to-noise estimates do not fall level by level")
    return {"verifier": VERIFIER_ID, "status": "passed", "manifest_sha256": record["manifest_sha256"],
            "provenance_sha256": sha256(provenance_path), "utterances": utterance_checks, "clips": clip_checks}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--ffmpeg", required=True)
    parser.add_argument("--ffprobe", required=True)
    parser.add_argument("--generated-dir", type=Path, default=GENERATED)
    parser.add_argument("--provenance", type=Path, help=f"defaults to <generated-dir>/{PROVENANCE_NAME}")
    parser.add_argument("--report", type=Path, help=f"defaults to <generated-dir>/{REPORT_NAME}")
    parser.add_argument("--utterances-dir", type=Path,
                        help="folder holding speech/<passage>-utterance.wav; defaults to <generated-dir>")
    args = parser.parse_args()
    generated = args.generated_dir.resolve(strict=True)
    provenance = (args.provenance or generated / PROVENANCE_NAME).resolve(strict=True)
    utterances = None if args.utterances_dir is None else args.utterances_dir.resolve(strict=True)
    report = verify(str(Path(args.ffmpeg).resolve(strict=True)), str(Path(args.ffprobe).resolve(strict=True)),
                    generated, provenance, utterances)
    destination = args.report or generated / REPORT_NAME
    destination.write_bytes((json.dumps(report, indent=2) + "\n").encode("utf-8"))
    print(f"Verified {len(report['utterances'])} utterances and {len(report['clips'])} clips")


if __name__ == "__main__":
    main()
