"""Independently verify generated media against frozen corpus times and pixels."""

from __future__ import annotations

import argparse
from decimal import Decimal
import hashlib
import json
import re
from pathlib import Path
import subprocess


ROOT = Path(__file__).resolve().parents[1]
CORPUS = ROOT / "fixtures" / "corpus"
GENERATED = CORPUS / "generated"


def run(argv: list[str], maximum: int) -> bytes:
    result = subprocess.run(argv, stdin=subprocess.DEVNULL, capture_output=True, timeout=30, check=False)
    if result.returncode != 0 or len(result.stdout) > maximum:
        raise RuntimeError(f"independent provider check failed: code={result.returncode}, bytes={len(result.stdout)}")
    return result.stdout


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def media_info(ffprobe: str, path: Path) -> dict:
    return json.loads(run([ffprobe, "-v", "error", "-show_entries",
                           "format=start_time,duration:stream=index,codec_type,start_time,width,height:stream_side_data=rotation:stream_tags=language",
                           "-of", "json", str(path)], 128 * 1024))


def frame_times(ffprobe: str, path: Path) -> list[Decimal]:
    output = run([ffprobe, "-v", "error", "-select_streams", "v:0", "-show_entries",
                  "frame=pts_time", "-of", "csv=p=0", str(path)], 128 * 1024)
    return [Decimal(line.strip().split(",")[0]) for line in output.decode().splitlines() if line.strip()]


def decoded_frame(ffmpeg: str, path: Path, time: Decimal, width: int, height: int) -> bytes:
    frame = run([ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-i", str(path),
                 "-ss", str(time), "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"],
                width * height * 3)
    if len(frame) != width * height * 3:
        raise AssertionError(f"decoded frame byte count differs for {path.name} at {time}")
    return frame


def pixel(frame: bytes, width: int, x: int, y: int) -> tuple[int, int, int]:
    offset = (y * width + x) * 3
    return tuple(frame[offset:offset + 3])


def region_difference(first: bytes, second: bytes, width: int, left: int, top: int, right: int, bottom: int) -> float:
    offsets = [(y * width + x) * 3 + channel for y in range(top, bottom, 4)
               for x in range(left, right, 4) for channel in range(3)]
    return sum(abs(first[offset] - second[offset]) for offset in offsets) / len(offsets)


def verify(ffmpeg: str, ffprobe: str) -> None:
    manifest_path = CORPUS / "manifest.json"
    truth = json.loads(manifest_path.read_text(encoding="utf-8"))
    provenance = json.loads((GENERATED / "provenance.json").read_text(encoding="utf-8"))
    if provenance["manifest_sha256"] != sha256(manifest_path):
        raise AssertionError("generator provenance points at different frozen truth")
    entries = {entry["fixture"]: entry for entry in provenance["fixtures"]}
    checks = []
    for fixture in truth["fixtures"]:
        fixture_id = fixture["id"]
        if fixture_id == "F11":
            continue
        entry = entries[fixture_id]
        path = GENERATED / f"{fixture_id}.{'mkv' if fixture_id == 'F09' else 'mp4'}"
        if sha256(path) != entry["sha256"] or path.stat().st_size != entry["bytes"]:
            raise AssertionError(f"{fixture_id} hash/size differs from generation record")
        info = media_info(ffprobe, path)
        video = next(stream for stream in info["streams"] if stream["codec_type"] == "video")
        if (video["width"], video["height"]) != (fixture["resolution"]["width"], fixture["resolution"]["height"]):
            raise AssertionError(f"{fixture_id} dimensions differ from frozen truth")
        origin = Decimal(info["format"].get("start_time", "0"))
        if fixture_id == "F09":
            if origin != Decimal("2.000000"):
                raise AssertionError("F09 must retain its nonzero source start")
            audio = next(stream for stream in info["streams"] if stream["codec_type"] == "audio")
            if Decimal(audio["start_time"]) - origin != Decimal("0.750000"):
                raise AssertionError("F09 audio offset differs from frozen truth")
        times = frame_times(ffprobe, path)
        if not times or times[0] != origin:
            raise AssertionError(f"{fixture_id} has an unexpected first presentation time")
        normalized = {time - origin for time in times}
        if fixture_id == "F09":
            if Decimal("3.250000") not in normalized or Decimal("7.250000") not in normalized:
                raise AssertionError("F09 VFR marker boundaries are absent")
            gaps = {times[index + 1] - times[index] for index in range(len(times) - 1)}
            if len(gaps) < 2:
                raise AssertionError("F09 must have variable presentation gaps")
            duration = Decimal(info["format"]["duration"]) - origin
        else:
            for event in fixture["events"]:
                if event["kind"] != "speech" and event["start_us"] < fixture["duration_us"]:
                    if Decimal(event["start_us"]) / 1_000_000 not in normalized:
                        raise AssertionError(f"{fixture_id} misses frozen boundary {event['id']}")
            duration = Decimal(info["format"]["duration"])
        if abs(duration - Decimal(fixture["duration_us"]) / 1_000_000) > Decimal("0.05"):
            raise AssertionError(f"{fixture_id} duration differs from frozen truth: {duration}")
        checks.append({"fixture": fixture_id, "sha256": entry["sha256"], "frames": len(times),
                       "duration_us": int(duration * 1_000_000), "origin_us": int(origin * 1_000_000),
                       "dimensions": [video["width"], video["height"]]})
    paths = {name: GENERATED / f"{name}.{'mkv' if name == 'F09' else 'mp4'}" for name in ("F02", "F03", "F06", "F09")}
    f02 = [decoded_frame(ffmpeg, paths["F02"], Decimal(time), 1280, 720) for time in ("1", "5", "9")]
    if region_difference(f02[0], f02[2], 1280, 140, 340, 300, 460) > 2 or \
            region_difference(f02[0], f02[1], 1280, 140, 340, 300, 460) < 10:
        raise AssertionError("F02 repeated slide states are not visually distinct/repeated")
    f03_before = decoded_frame(ffmpeg, paths["F03"], Decimal("2"), 1440, 900)
    f03_after = decoded_frame(ffmpeg, paths["F03"], Decimal("5"), 1440, 900)
    if pixel(f03_before, 1440, 860, 430) == pixel(f03_after, 1440, 860, 430):
        raise AssertionError("F03 G18 fill did not change")
    f06 = [decoded_frame(ffmpeg, paths["F06"], Decimal(time), 1280, 720) for time in ("4.0", "4.3", "4.8")]
    if pixel(f06[0], 1280, 340, 340) == pixel(f06[1], 1280, 340, 340) or \
            region_difference(f06[0], f06[2], 1280, 330, 330, 970, 440) > 2:
        raise AssertionError("F06 500 ms tooltip interval is not visible and transient")
    f09_before = decoded_frame(ffmpeg, paths["F09"], Decimal("3.00"), 1280, 720)
    f09_at = decoded_frame(ffmpeg, paths["F09"], Decimal("3.25"), 1280, 720)
    if f09_before == f09_at:
        raise AssertionError("F09 marker did not appear at its frozen visual boundary")
    for entry in provenance["F11_variants"]:
        path = GENERATED / entry["file"]
        if sha256(path) != entry["sha256"] or path.stat().st_size != entry["bytes"]:
            raise AssertionError(f"F11 {entry['variant']} differs from its recipe")
    for entry in provenance["media_variants"]:
        path = GENERATED / entry["file"]
        if sha256(path) != entry["sha256"] or path.stat().st_size != entry["bytes"]:
            raise AssertionError(f"media variant {entry['variant']} differs from its recipe")
        info = media_info(ffprobe, path)
        if entry["variant"] == "rotation-90":
            video = next(stream for stream in info["streams"] if stream["codec_type"] == "video")
            if video["side_data_list"][0]["rotation"] != 90:
                raise AssertionError("rotation variant lacks a 90 degree display matrix")
            image = run([ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-i", str(path),
                         "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"], 720 * 1280 * 3)
            if len(image) != 720 * 1280 * 3:
                raise AssertionError("rotation variant did not display as portrait")
        elif entry["variant"] == "audio-only":
            if [stream["codec_type"] for stream in info["streams"]] != ["audio"]:
                raise AssertionError("audio-only variant has an unexpected video stream")
            result = subprocess.run([ffmpeg, "-hide_banner", "-loglevel", "info", "-nostdin", "-ss", "0", "-i", str(path),
                                     "-map", "0:0", "-t", "1", "-af", "ashowinfo", "-f", "null", "-"],
                                    stdin=subprocess.DEVNULL, capture_output=True, timeout=30, check=False)
            match = re.search(rb"Parsed_ashowinfo_[^\n]*n:0[^\n]*pts_time:([0-9.]+)", result.stderr)
            if result.returncode != 0 or not match or int(Decimal(match.group(1).decode()) * 1_000_000) != entry["expected_actual_audio_start_us"]:
                raise AssertionError("audio-only variant decoder start differs from its declared priming gap")
        elif entry["variant"] == "multiple-audio":
            if [stream["codec_type"] for stream in info["streams"]] != ["video", "audio", "audio"]:
                raise AssertionError("multiple-audio variant stream order changed")
            if [stream["tags"]["language"] for stream in info["streams"][1:]] != ["eng", "spa"]:
                raise AssertionError("multiple-audio variant language tags changed")
    report = {"verifier": "tools/verify_p04_fixtures.py v1", "status": "passed", "manifest_sha256": sha256(manifest_path),
              "checks": checks, "pixel_checks": ["F02 repeated A/B/A", "F03 G18 fill", "F06 500 ms tooltip", "F09 3.25 s marker"],
              "F11_variant_hashes_verified": len(provenance["F11_variants"]),
              "media_variant_hashes_verified": len(provenance["media_variants"])}
    (GENERATED / "verification.json").write_bytes((json.dumps(report, indent=2) + "\n").encode("utf-8"))
    print(f"Verified {len(checks)} source clips and {len(provenance['F11_variants'])} malformed variants")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ffmpeg", required=True)
    parser.add_argument("--ffprobe", required=True)
    args = parser.parse_args()
    verify(str(Path(args.ffmpeg).resolve(strict=True)), str(Path(args.ffprobe).resolve(strict=True)))


if __name__ == "__main__":
    main()
