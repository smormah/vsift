"""The OCR runner's hosted checks for 0.3.0 (ADR 0030): a hosted runner only.

Only a disposable GitHub Actions runner may run this (`P16 OCR runner`). It
stages VSift's own OCR runner with the pinned ONNX Runtime and models, and
measures it on frames VSift itself extracts:

- `stage`: the runner folder (`vsift-ocr-runner`, `onnxruntime/`, `models/`,
  `LICENSES/`) from the built runner, Microsoft's ONNX Runtime archive and the
  two PP-OCRv5 models, each download held to its pinned size and SHA-256
  before anything is kept; writes the folder's inventory (every file's size
  and SHA-256), the data a managed catalogue entry needs.
- `describe`: runs the runner's `describe` and holds it to the pins.
- `recognise`: runs the runner over the frames of `p16_ocr_evaluation.py
  extract`, in the evaluation's reading format (`ocr-vsift-ocr-runner.jsonl`).
- `parity`: compares those readings with RapidOCR's (the evaluated reference
  pipeline, `ocr-ppocr-v5-mobile-latin.jsonl`) line by line, and scores the
  runner against the synthetic truth as the evaluation scored the engines.
- `end-to-end`: `vsift ocr` over a whole video until its coverage is complete,
  then `vsift ocr search` for what the screen showed: the synthetic numbers (with
  their truth times) or a real recording's interface labels.
- `compare`: the runner's readings on another platform against Linux's.

As in the evaluation, the real recordings' frames and recognised text stay on
the runner: the report keeps numbers and which interface labels were found.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys
import tarfile
import time
import zipfile
from dataclasses import dataclass
from typing import Callable, Sequence

import p16_ocr_evaluation as evaluation
from p16_ocr_evaluation import Line, Recognition

SECOND = 1_000_000
RUNNER_NAME = "vsift-ocr-runner"
ENGINE_NAME = "vsift-ocr-runner"
REFERENCE_ENGINE = "ppocr-v5-mobile-latin"
RUNTIME_VERSION = "1.31.0"
RUNTIME_RELEASE = f"https://github.com/microsoft/onnxruntime/releases/download/v{RUNTIME_VERSION}"
MAX_RUNTIME_ARCHIVE_BYTES = 128 * 1024 * 1024
MAX_MODEL_BYTES = 64 * 1024 * 1024
MAX_MEMBER_BYTES = 256 * 1024 * 1024
IMAGES_PER_RUN = 8
RUN_TIMEOUT_S = 600

# Microsoft's 1.31.0 archives, by GitHub's asset digest. `library` is the file
# the runner loads (`crates/vsift-ocr-runner/src/onnx.rs`, RUNTIME_LIBRARY);
# `stage` lists the archive's `lib` folder when the member is not where pinned.
RUNTIMES = {
    "linux-x64": {"asset": f"onnxruntime-linux-x64-{RUNTIME_VERSION}.tgz", "bytes": 11_408_003,
                  "sha256": "cc5c72baf5ae5c8238a6841f0897227be2d02826b9cf98eaf02fdefaeeb03a57",
                  "library": f"libonnxruntime.so.{RUNTIME_VERSION}"},
    "osx-arm64": {"asset": f"onnxruntime-osx-arm64-{RUNTIME_VERSION}.tgz", "bytes": 36_549_009,
                  "sha256": "1031b36dd3bdaa8d976a6130d9cf2a05e9be320b93ae804f8427b91dc3432afc",
                  "library": f"libonnxruntime.{RUNTIME_VERSION}.dylib"},
    "win-x64": {"asset": f"onnxruntime-win-x64-{RUNTIME_VERSION}.zip", "bytes": 82_887_349,
                "sha256": "3958d8a44984160692c1b4ffd0de3cb48f8c39f9d34bb31e433db06e30b82195",
                "library": "onnxruntime.dll"},
}
RUNTIME_LICENCES = ("LICENSE", "ThirdPartyNotices.txt")

# RapidOCR's ModelScope list at v3.10.0, the files the evaluation measured
# (docs/planning/ocr-runner-build-and-catalogue.md, section 4).
MODEL_BASE = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5"
MODELS = {
    "ch_PP-OCRv5_det_mobile.onnx": (f"{MODEL_BASE}/det/ch_PP-OCRv5_det_mobile.onnx", 4_819_576,
                                    "4d97c44a20d30a81aad087d6a396b08f786c4635742afc391f6621f5c6ae78ae"),
    "latin_PP-OCRv5_rec_mobile.onnx": (f"{MODEL_BASE}/rec/latin_PP-OCRv5_rec_mobile.onnx", 7_904_513,
                                       "b20bd37c168a570f583afbc8cd7925603890efbcdc000a59e22c269d160b5f5a"),
}
DETECTOR = "ch_PP-OCRv5_det_mobile.onnx"
RECOGNIZER = "latin_PP-OCRv5_rec_mobile.onnx"
MODELS_NOTICE = (
    "The two models in models/ are PaddleOCR's PP-OCRv5 mobile text detector and Latin text\n"
    "recognizer (Apache-2.0, https://github.com/PaddlePaddle/PaddleOCR), as converted to ONNX\n"
    "by RapidOCR and published in its ModelScope list at v3.10.0, unchanged:\n"
    + "".join(f"  {name}: {size} bytes, SHA-256 {digest}\n" for name, (_url, size, digest) in MODELS.items())
)

# What `parity` requires of the runner on the synthetic screens, where the
# reference is the evaluated pipeline on the same frames. The two box shapes
# differ (RapidOCR unclips a rotated rectangle, the runner an upright one), so
# lines are paired by overlap, and the text is what must agree.
PAIRING_IOU = 0.5
MIN_SYNTHETIC_TEXT_AGREEMENT = 0.97
MIN_PLATFORM_TEXT_AGREEMENT = 0.99


class RunnerCheckError(Exception):
    """A hosted check failed; the message is fixed prose plus counts, never recognised text."""


def sha256_bytes(content: bytes) -> str:
    return hashlib.sha256(content).hexdigest()


def checked_download(url: str, size: int, digest: str, limit: int,
                     get: Callable[[str, int], bytes]) -> bytes:
    """The body of `url`, refused unless it is exactly `size` bytes with SHA-256 `digest`."""
    content = get(url, limit)
    if len(content) != size or sha256_bytes(content) != digest:
        raise RunnerCheckError(f"{url.rsplit('/', 1)[-1]} differs from its pin")
    return content


def archive_lib_listing(content: bytes, asset: str) -> list[tuple[str, int, str]]:
    """Every entry of the archive's `lib` folder: name, size (0 for a link) and its kind."""
    listing = []
    if asset.endswith(".zip"):
        with zipfile.ZipFile(io.BytesIO(content)) as archive:
            for info in archive.infolist():
                if "/lib/" in info.filename:
                    listing.append((info.filename, info.file_size, "dir" if info.is_dir() else "file"))
    else:
        with tarfile.open(fileobj=io.BytesIO(content), mode="r:gz") as archive:
            for member in archive.getmembers():
                if "/lib/" in member.name:
                    kind = "file" if member.isfile() else "link" if member.issym() or member.islnk() else "other"
                    listing.append((member.name, member.size if member.isfile() else 0, kind))
    return sorted(listing)


def archive_member(content: bytes, asset: str, name: str, limit: int = MAX_MEMBER_BYTES) -> bytes:
    """One regular file of the archive, by its exact name; a link, a folder or a missing name is refused.

    Nothing is extracted to disk by the archive library: the member's bytes are
    read and written by the caller, so no archive path decides where a file goes.
    """
    if asset.endswith(".zip"):
        with zipfile.ZipFile(io.BytesIO(content)) as archive:
            try:
                info = archive.getinfo(name)
            except KeyError as error:
                raise RunnerCheckError(f"{asset} has no {name}") from error
            if info.is_dir() or info.file_size > limit:
                raise RunnerCheckError(f"{asset}'s {name} is not a bounded regular file")
            with archive.open(info) as handle:
                data = handle.read(limit + 1)
    else:
        with tarfile.open(fileobj=io.BytesIO(content), mode="r:gz") as archive:
            # macOS's archive names its members `./<root>/...`, Linux's `<root>/...`.
            names = set(archive.getnames())
            found = next((candidate for candidate in (name, f"./{name}") if candidate in names), None)
            if found is None:
                raise RunnerCheckError(f"{asset} has no {name}")
            member = archive.getmember(found)
            if not member.isfile() or member.size > limit:
                raise RunnerCheckError(f"{asset}'s {name} is not a bounded regular file")
            handle = archive.extractfile(member)
            if handle is None:
                raise RunnerCheckError(f"{asset}'s {name} cannot be read")
            data = handle.read(limit + 1)
    if len(data) > limit:
        raise RunnerCheckError(f"{asset}'s {name} exceeds its bound")
    return data


def runtime_root(asset: str) -> str:
    """The archive's top folder: its asset name without the extension."""
    return asset.removesuffix(".tgz").removesuffix(".zip")


def write_new(path: Path, content: bytes, executable: bool = False) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("xb") as handle:
        handle.write(content)
    if executable:
        path.chmod(0o755)


def stage(out: Path, runner: Path, platform: str, get: Callable[[str, int], bytes]) -> dict:
    """The runner folder in `out`, and its inventory: every file's path, size and SHA-256."""
    if platform not in RUNTIMES:
        raise RunnerCheckError(f"unknown platform {platform}")
    pin = RUNTIMES[platform]
    out.mkdir(parents=True, exist_ok=False)
    executable = RUNNER_NAME + (".exe" if platform.startswith("win") else "")
    write_new(out / executable, runner.read_bytes(), executable=True)
    archive = checked_download(f"{RUNTIME_RELEASE}/{pin['asset']}", pin["bytes"], pin["sha256"],
                               MAX_RUNTIME_ARCHIVE_BYTES, get)
    root = runtime_root(pin["asset"])
    listing = archive_lib_listing(archive, pin["asset"])
    try:
        library = archive_member(archive, pin["asset"], f"{root}/lib/{pin['library']}")
    except RunnerCheckError as error:
        shown = ", ".join(f"{name} ({kind}, {size} bytes)" for name, size, kind in listing)
        raise RunnerCheckError(f"{error}; its lib folder holds: {shown}") from error
    write_new(out / "onnxruntime" / pin["library"], library)
    for licence in RUNTIME_LICENCES:
        write_new(out / "LICENSES" / f"onnxruntime-{licence}", archive_member(archive, pin["asset"], f"{root}/{licence}"))
    for name, (url, size, digest) in MODELS.items():
        write_new(out / "models" / name, checked_download(url, size, digest, MAX_MODEL_BYTES, get))
    write_new(out / "LICENSES" / "models-NOTICE.txt", MODELS_NOTICE.encode("utf-8"))
    files = []
    for path in sorted(item for item in out.rglob("*") if item.is_file()):
        files.append({"path": path.relative_to(out).as_posix(), "bytes": path.stat().st_size,
                      "sha256": evaluation.sha256_of(path)})
    return {"platform": platform, "runtime_asset": pin["asset"], "runtime_lib_listing": listing, "files": files}


def run_runner(executable: Path, arguments: list[str], timeout: int = RUN_TIMEOUT_S) -> tuple[int, str, float]:
    """The runner with an explicit argument list, an empty environment, and no shell.

    Its standard error (fixed diagnostics, never recognised text) goes to the log.
    """
    started = time.monotonic()
    environment = {"SystemRoot": os.environ["SystemRoot"]} if "SystemRoot" in os.environ else {}
    completed = subprocess.run([str(executable), *arguments], capture_output=True, text=True, timeout=timeout,
                               check=False, env=environment, encoding="utf-8")
    if completed.stderr.strip():
        print(completed.stderr.strip()[-2000:], file=sys.stderr)
    return completed.returncode, completed.stdout, time.monotonic() - started


def check_description(status: int, stdout: str) -> dict:
    """`describe`'s one record, held to the pinned runtime and models."""
    if status != 0:
        raise RunnerCheckError(f"describe exited {status}: {stdout.strip()[:200]}")
    records = [json.loads(line) for line in stdout.splitlines() if line.strip()]
    if len(records) != 1:
        raise RunnerCheckError(f"describe wrote {len(records)} records")
    record = records[0]
    expected = {"record": "describe", "protocol": 1, "runtime_version": RUNTIME_VERSION,
                "profile": "ppocrv5-latin-v1", "detector_sha256": MODELS[DETECTOR][2],
                "recognizer_sha256": MODELS[RECOGNIZER][2]}
    wrong = sorted(key for key, value in expected.items() if record.get(key) != value)
    if wrong:
        raise RunnerCheckError(f"describe differs from the pins in {', '.join(wrong)}")
    return record


def parse_read(stdout: str, count: int) -> list[list[Line] | None]:
    """Per image of a `read`, its lines, or `None` for an `error` record; the protocol is checked."""
    records = [json.loads(line) for line in stdout.splitlines() if line.strip()]
    if not records or records[0].get("record") != "runner" or records[0].get("protocol") != 1:
        raise RunnerCheckError("read did not start with protocol 1's runner record")
    readings: list[list[Line] | None] = []
    current: list[Line] = []
    for record in records[1:]:
        kind = record.get("record")
        if kind == "line" and record.get("image") == len(readings):
            current.append(Line(str(record["text"]), (int(record["x"]), int(record["y"]), int(record["width"]),
                                                      int(record["height"])), float(record["confidence"])))
        elif kind == "image" and record.get("image") == len(readings) and record.get("lines") == len(current):
            readings.append(current)
            current = []
        elif kind == "error" and record.get("image") == len(readings) and not current:
            readings.append(None)
        else:
            raise RunnerCheckError(f"read wrote an unexpected {kind} record")
    if len(readings) != count or current:
        raise RunnerCheckError(f"read answered {len(readings)} of {count} images")
    return readings


def recognise(executable: Path, frames_dir: Path, threads: int) -> list[Recognition]:
    """The runner over every frame of `frames_dir`, eight a run, in the evaluation's format."""
    record = json.loads((frames_dir / "frames.json").read_text(encoding="utf-8"))
    files = [frame["file"] for frame in record["frames"]]
    readings: list[Recognition] = []
    out = frames_dir / f"ocr-{ENGINE_NAME}.jsonl"
    with out.open("w", encoding="utf-8") as handle:
        for start in range(0, len(files), IMAGES_PER_RUN):
            batch = files[start:start + IMAGES_PER_RUN]
            status, stdout, seconds = run_runner(executable, ["read", "--threads", str(threads), "--max-lines", "256",
                                                              *[str(frames_dir / name) for name in batch]])
            if status != 0:
                raise RunnerCheckError(f"read exited {status} on {frames_dir.name}")
            for name, lines in zip(batch, parse_read(stdout, len(batch))):
                if lines is None:
                    raise RunnerCheckError(f"the runner refused a frame of {frames_dir.name}")
                reading = Recognition(name, lines, seconds / len(batch))
                readings.append(reading)
                handle.write(json.dumps({"file": name, "seconds": round(reading.seconds, 4), "cpu_seconds": None,
                                         "lines": [{"text": line.text, "box": list(line.box),
                                                    "confidence": line.confidence} for line in lines]},
                                        ensure_ascii=False) + "\n")
    return readings


def iou(left: tuple[int, int, int, int], right: tuple[int, int, int, int]) -> float:
    """Intersection over union of two (x, y, width, height) boxes."""
    lx, ly, lw, lh = left
    rx, ry, rw, rh = right
    width = max(0, min(lx + lw, rx + rw) - max(lx, rx))
    height = max(0, min(ly + lh, ry + rh) - max(ly, ry))
    inter = width * height
    union = lw * lh + rw * rh - inter
    return inter / union if union > 0 else 0.0


def parity_of_frame(reference: list[Line], runner: list[Line]) -> dict:
    """Pairs each reference line with the runner's line it overlaps most (at least PAIRING_IOU)."""
    used: set[int] = set()
    paired = text_equal = 0
    offsets = []
    for line in reference:
        best, best_iou = None, PAIRING_IOU
        for index, candidate in enumerate(runner):
            if index in used:
                continue
            overlap = iou(line.box, candidate.box)
            if overlap >= best_iou:
                best, best_iou = index, overlap
        if best is None:
            continue
        used.add(best)
        paired += 1
        if evaluation.normalise(line.text) == evaluation.normalise(runner[best].text):
            text_equal += 1
        offsets.append(max(abs(a - b) for a, b in zip(line.box, runner[best].box)))
    return {"reference_lines": len(reference), "runner_lines": len(runner), "paired": paired,
            "text_equal": text_equal, "unpaired_runner_lines": len(runner) - len(used), "box_offsets": offsets}


def parity(reference: list[Recognition], runner: list[Recognition]) -> dict:
    """Line-by-line agreement of the runner with the reference over the same frames."""
    by_file = {reading.file: reading for reading in runner}
    totals = {"frames": 0, "reference_lines": 0, "runner_lines": 0, "paired": 0, "text_equal": 0,
              "unpaired_runner_lines": 0}
    offsets: list[int] = []
    for reading in reference:
        own = by_file.get(reading.file)
        if own is None:
            raise RunnerCheckError("the runner has no reading of a reference frame")
        frame = parity_of_frame(reading.lines, own.lines)
        totals["frames"] += 1
        for key in ("reference_lines", "runner_lines", "paired", "text_equal", "unpaired_runner_lines"):
            totals[key] += frame[key]
        offsets.extend(frame["box_offsets"])
    lines = totals["reference_lines"]
    return {**totals,
            "paired_share": round(totals["paired"] / lines, 4) if lines else None,
            "text_agreement": round(totals["text_equal"] / lines, 4) if lines else None,
            "median_box_offset_px": statistics.median(offsets) if offsets else None,
            "max_box_offset_px": max(offsets) if offsets else None}


# --- where the runner and the reference differ (counts only) -----------------------------------

def text_difference(reference: str, runner: str) -> str:
    """The kind of difference between two readings of one line, never the text itself.

    `case_or_space`: equal once case and whitespace are folded; `shorter` and
    `longer`: the runner's text is a strict part of the reference's, or holds it
    (characters lost or gained at a box edge); `near`: within two edits;
    `far`: anything else.
    """
    left = evaluation.normalise(reference)
    right = evaluation.normalise(runner)
    if left == right:
        return "equal"
    if "".join(left.casefold().split()) == "".join(right.casefold().split()):
        return "case_or_space"
    if right and right in left:
        return "shorter"
    if left and left in right:
        return "longer"
    if evaluation.edit_distance(left, right) <= 2:
        return "near"
    return "far"


def ratio_bucket(value: float) -> str:
    """A box size ratio, runner over reference, in a few coarse buckets."""
    if value < 0.8:
        return "<0.8"
    if value < 0.95:
        return "0.8-0.95"
    if value <= 1.05:
        return "0.95-1.05"
    if value <= 1.25:
        return "1.05-1.25"
    return ">1.25"


def diagnose_frame(reference: list[Line], runner: list[Line], tally: dict) -> None:
    """Adds one frame's pairing to `tally`: what became of each reference line, and why it may differ."""
    used: set[int] = set()
    for line in reference:
        best, best_iou = None, 0.0
        for index, candidate in enumerate(runner):
            if index in used:
                continue
            overlap = iou(line.box, candidate.box)
            if overlap > best_iou:
                best, best_iou = index, overlap
        confidence = "low" if (line.confidence or 0.0) < 0.8 else "high"
        if best is None or best_iou < PAIRING_IOU:
            kind = "unpaired_overlapping" if best is not None else "unpaired_missed"
            tally["outcomes"][kind] = tally["outcomes"].get(kind, 0) + 1
            tally["reference_confidence"][f"{kind}/{confidence}"] = (
                tally["reference_confidence"].get(f"{kind}/{confidence}", 0) + 1)
            continue
        used.add(best)
        own = runner[best]
        kind = text_difference(line.text, own.text)
        tally["outcomes"][kind] = tally["outcomes"].get(kind, 0) + 1
        tally["reference_confidence"][f"{kind}/{confidence}"] = (
            tally["reference_confidence"].get(f"{kind}/{confidence}", 0) + 1)
        group = "equal" if kind == "equal" else "differing"
        width = own.box[2] / line.box[2] if line.box[2] else 0.0
        height = own.box[3] / line.box[3] if line.box[3] else 0.0
        tally["width_ratio"][group][ratio_bucket(width)] = tally["width_ratio"][group].get(ratio_bucket(width), 0) + 1
        tally["height_ratio"][group][ratio_bucket(height)] = (
            tally["height_ratio"][group].get(ratio_bucket(height), 0) + 1)
        tall = "tall" if line.box[3] > 1.5 * max(1, line.box[2]) else "wide"
        tally["shape"][f"{group}/{tall}"] = tally["shape"].get(f"{group}/{tall}", 0) + 1
        size = "small" if line.box[3] < 16 else "medium" if line.box[3] < 28 else "large"
        tally["line_height"][f"{group}/{size}"] = tally["line_height"].get(f"{group}/{size}", 0) + 1
    tally["outcomes"]["runner_only"] = tally["outcomes"].get("runner_only", 0) + len(runner) - len(used)


def diagnose(reference: list[Recognition], runner: list[Recognition]) -> dict:
    """Counts of how the runner's lines relate to the reference's, over the same frames."""
    by_file = {reading.file: reading for reading in runner}
    tally: dict = {"frames": 0, "reference_lines": 0, "outcomes": {}, "reference_confidence": {},
                   "width_ratio": {"equal": {}, "differing": {}}, "height_ratio": {"equal": {}, "differing": {}},
                   "shape": {}, "line_height": {}}
    for reading in reference:
        own = by_file.get(reading.file)
        if own is None:
            raise RunnerCheckError("the runner has no reading of a reference frame")
        tally["frames"] += 1
        tally["reference_lines"] += len(reading.lines)
        diagnose_frame(reading.lines, own.lines, tally)
    return tally


def agreement(linux: list[Recognition], other: list[Recognition]) -> dict:
    """Whether another platform's runner read the same lines as Linux's, frame by frame."""
    by_file = {reading.file: reading for reading in other}
    lines = equal = 0
    for reading in linux:
        theirs = by_file.get(reading.file)
        if theirs is None:
            raise RunnerCheckError("the other platform has no reading of a frame")
        ours = [evaluation.normalise(line.text) for line in reading.lines]
        their_text = [evaluation.normalise(line.text) for line in theirs.lines]
        lines += max(len(ours), len(their_text))
        equal += sum(1 for left, right in zip(ours, their_text) if left == right)
    return {"lines": lines, "equal": equal, "agreement": round(equal / lines, 4) if lines else None}


# --- end to end: vsift ocr and ocr search ------------------------------------------------------------

@dataclass
class OcrRun:
    """`vsift ocr` called until its coverage is complete, or `max_calls` times."""

    session: str
    calls: list[dict]
    complete: bool
    coverage: dict


def ocr_until_complete(vsift: evaluation.Vsift, session: str, runner: Path, max_calls: int,
                       period_us: int | None = None) -> OcrRun:
    calls = []
    coverage: dict = {}
    for number in range(max_calls):
        arguments = ["ocr", session, "--runner", str(runner), "--max-frames", "200"]
        if period_us is not None:
            arguments += ["--period-us", str(period_us)]
        started = time.monotonic()
        answer = vsift(f"ocr call {number + 1}", arguments)
        data = answer["data"]
        coverage = data["coverage"]
        calls.append({"status": answer["status"], "read_now": data["read_now"], "stopped": data["stopped"],
                      "seconds": round(time.monotonic() - started, 2), "coverage": coverage})
        if answer["status"] == "complete":
            return OcrRun(session, calls, True, coverage)
        if data["read_now"] == 0 and coverage["pending"] == 0:
            break
    return OcrRun(session, calls, False, coverage)


def screen_text_hits(vsift: evaluation.Vsift, session: str, query: str) -> list[dict]:
    """Every on-screen hit of `ocr search`, page after page (no transcript needed)."""
    hits: list[dict] = []
    cursor = None
    while True:
        # `--query=` keeps a query that starts with a hyphen a value, not an option.
        arguments = ["ocr", "search", session, f"--query={query}", "--limit", "100"]
        if cursor:
            arguments += ["--cursor", cursor]
        data = vsift(f"ocr search {len(query)} chars", arguments)["data"]
        hits.extend(data["hits"])
        cursor = data.get("next_cursor")
        if not cursor:
            return hits


def synthetic_found(truth: dict, hits_by_number: dict[str, list[dict]]) -> dict:
    """For each synthetic number, whether a hit lies within a screen that showed it."""
    hold = truth["hold_us"]
    numbers = found = 0
    for screen_index, screen in enumerate(truth["screens"]):
        for line in screen["lines"]:
            for token in evaluation.tokens(line["text"]):
                if not evaluation.is_number(token):
                    continue
                numbers += 1
                start, end = screen_index * hold, (screen_index + 1) * hold
                if any(start <= hit["frame"]["time_us"] < end for hit in hits_by_number.get(token, [])):
                    found += 1
    return {"numbers": numbers, "found": found, "found_share": round(found / numbers, 4) if numbers else None}


def ingest(vsift: evaluation.Vsift, video: Path) -> str:
    """A session without any transcript: `ocr search` needs none."""
    return vsift("ingest", ["ingest", str(video)])["data"]["session_id"]


def summarise_run(run: OcrRun) -> dict:
    return {"calls": len(run.calls), "complete": run.complete, "coverage": run.coverage,
            "seconds": round(sum(call["seconds"] for call in run.calls), 1),
            "frames_read": sum(call["read_now"] for call in run.calls)}


def end_to_end_synthetic(vsift: evaluation.Vsift, video: Path, truth: dict, runner: Path, max_calls: int) -> dict:
    session = ingest(vsift, video)
    run = ocr_until_complete(vsift, session, runner, max_calls)
    numbers = sorted({token for screen in truth["screens"] for line in screen["lines"]
                      for token in evaluation.tokens(line["text"]) if evaluation.is_number(token)})
    hits = {number: screen_text_hits(vsift, session, number) for number in numbers}
    vsift("session close", ["session", "close", session])
    return {"video": video.name, **summarise_run(run), "search": synthetic_found(truth, hits)}


def end_to_end_real(vsift: evaluation.Vsift, item: str, video: Path, runner: Path, max_calls: int) -> dict:
    session = ingest(vsift, video)
    run = ocr_until_complete(vsift, session, runner, max_calls)
    labels = []
    for spellings in evaluation.LEXICON.get(item, []):
        hits = screen_text_hits(vsift, session, spellings[0])
        labels.append({"label": spellings[0], "found": bool(hits)})
    vsift("session close", ["session", "close", session])
    found = sum(label["found"] for label in labels)
    return {"item": item, **summarise_run(run), "labels": len(labels), "labels_found": found, "per_label": labels}


# --- the command line ---------------------------------------------------------------------------

def command_stage(arguments: argparse.Namespace) -> int:
    inventory = stage(Path(arguments.out), Path(arguments.runner), arguments.platform, evaluation.bounded_https_get)
    Path(arguments.inventory).write_text(json.dumps(inventory, indent=1) + "\n", encoding="utf-8")
    for entry in inventory["files"]:
        print(f"{entry['path']}: {entry['bytes']} bytes, sha256 {entry['sha256']}")
    return 0


def command_describe(arguments: argparse.Namespace) -> int:
    status, stdout, _seconds = run_runner(Path(arguments.runner), ["describe"])
    record = check_description(status, stdout)
    print(json.dumps(record))
    return 0


def command_recognise(arguments: argparse.Namespace) -> int:
    for folder in arguments.frames:
        readings = recognise(Path(arguments.runner), Path(folder), int(arguments.threads))
        print(f"{ENGINE_NAME} on {Path(folder).name}: {len(readings)} frames, {evaluation.timing(readings)}")
    return 0


def command_parity(arguments: argparse.Namespace) -> int:
    work = Path(arguments.work)
    truth = json.loads((Path(arguments.synthetic) / "truth.json").read_text(encoding="utf-8"))
    report: dict = {"thresholds": {"pairing_iou": PAIRING_IOU,
                                   "min_synthetic_text_agreement": MIN_SYNTHETIC_TEXT_AGREEMENT},
                    "videos": {}}
    failures = []
    for folder in arguments.frames:
        frames_dir = work / folder
        reference = evaluation.read_recognitions(frames_dir / f"ocr-{REFERENCE_ENGINE}.jsonl")
        own = evaluation.read_recognitions(frames_dir / f"ocr-{ENGINE_NAME}.jsonl")
        entry = {"parity": parity(reference, own), "seconds_per_frame": evaluation.timing(own)}
        if folder.startswith("synthetic-"):
            frames = json.loads((frames_dir / "frames.json").read_text(encoding="utf-8"))
            entry["truth"] = {ENGINE_NAME: evaluation.score_synthetic(truth, frames, own)["overall"],
                              REFERENCE_ENGINE: evaluation.score_synthetic(truth, frames, reference)["overall"]}
            agreement_share = entry["parity"]["text_agreement"]
            if agreement_share is None or agreement_share < MIN_SYNTHETIC_TEXT_AGREEMENT:
                failures.append(f"{folder}: text agreement {agreement_share}")
        else:
            real = evaluation.score_real(folder, {ENGINE_NAME: own, REFERENCE_ENGINE: reference},
                                         {ENGINE_NAME: "runner", REFERENCE_ENGINE: "ppocr"})
            entry["labels"] = {engine: values["labels_found"] for engine, values in real["engines"].items()}
            entry["labels_total"] = real["labels"]
        report["videos"][folder] = entry
    Path(arguments.report).write_text(json.dumps(report, indent=1) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=1))
    if failures:
        print("parity below its threshold: " + "; ".join(failures), file=sys.stderr)
        return 1
    return 0


def vsift_for(work: Path, executable: Path, ffmpeg: str, ffprobe: str) -> evaluation.Vsift:
    vsift = evaluation.Vsift(executable, work / "root", work / "user")
    vsift.user.mkdir(parents=True, exist_ok=True)
    vsift("setup configure ffmpeg", ["setup", "configure", "ffmpeg", "--executable", ffmpeg])
    vsift("setup configure ffprobe", ["setup", "configure", "ffprobe", "--executable", ffprobe])
    return vsift


def command_end_to_end(arguments: argparse.Namespace) -> int:
    vsift = vsift_for(Path(arguments.work), Path(arguments.vsift), arguments.ffmpeg, arguments.ffprobe)
    runner = Path(arguments.runner)
    truth = json.loads((Path(arguments.synthetic) / "truth.json").read_text(encoding="utf-8"))
    report: dict = {"synthetic": [], "real": [], "steps": vsift.steps}
    for video in arguments.synthetic_video:
        report["synthetic"].append(end_to_end_synthetic(vsift, Path(video), truth, runner, int(arguments.max_calls)))
    for pair in arguments.real:
        item, _, video = pair.partition("=")
        report["real"].append(end_to_end_real(vsift, item, Path(video), runner, int(arguments.max_calls)))
    Path(arguments.report).write_text(json.dumps(report, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps({key: value for key, value in report.items() if key != "steps"}, indent=1,
                     ensure_ascii=False))
    incomplete = [entry.get("video") or entry.get("item") for entry in report["synthetic"] + report["real"]
                  if not entry["complete"]]
    if incomplete:
        print(f"ocr did not complete on {', '.join(incomplete)}", file=sys.stderr)
        return 1
    return 0


def command_diagnose(arguments: argparse.Namespace) -> int:
    work = Path(arguments.work)
    report = {}
    for folder in arguments.frames:
        frames_dir = work / folder
        report[folder] = diagnose(evaluation.read_recognitions(frames_dir / f"ocr-{REFERENCE_ENGINE}.jsonl"),
                                  evaluation.read_recognitions(frames_dir / f"ocr-{ENGINE_NAME}.jsonl"))
    Path(arguments.report).write_text(json.dumps(report, indent=1) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=1))
    return 0


def crosscheck_frames(frames_dir: Path, recognise_crop: Callable[[object], str], read_image: Callable[[Path], object],
                      crop: Callable[[object, tuple[int, int, int, int]], object]) -> dict:
    """The reference's recogniser on the runner's own boxes, against the runner's text and the reference's.

    Counts only. `same_as_runner` near 1 means that, given the same box, the two
    recognisers agree, so the differences come from the boxes and crops; low
    means they come from recognition (its input or its decoding).
    """
    reference = {reading.file: reading for reading in
                 evaluation.read_recognitions(frames_dir / f"ocr-{REFERENCE_ENGINE}.jsonl")}
    own = evaluation.read_recognitions(frames_dir / f"ocr-{ENGINE_NAME}.jsonl")
    counts = {"lines": 0, "same_as_runner": 0, "same_as_reference_pipeline": 0,
              "reference_pipeline_lines_paired": 0, "kinds_against_runner": {}}
    for reading in own:
        image = read_image(frames_dir / reading.file)
        theirs = reference.get(reading.file)
        for line in reading.lines:
            text = recognise_crop(crop(image, line.box))
            counts["lines"] += 1
            kind = text_difference(line.text, text)
            counts["kinds_against_runner"][kind] = counts["kinds_against_runner"].get(kind, 0) + 1
            if kind == "equal":
                counts["same_as_runner"] += 1
            if theirs is not None:
                best = max(theirs.lines, key=lambda other: iou(other.box, line.box), default=None)
                if best is not None and iou(best.box, line.box) >= PAIRING_IOU:
                    counts["reference_pipeline_lines_paired"] += 1
                    if evaluation.normalise(best.text) == evaluation.normalise(text):
                        counts["same_as_reference_pipeline"] += 1
    return counts


def command_crosscheck(arguments: argparse.Namespace) -> int:
    """Runs in the reference's virtual environment (RapidOCR, OpenCV): hosted only."""
    import cv2  # noqa: PLC0415 - the reference's environment only
    from rapidocr import LangDet, LangRec, ModelType, OCRVersion, RapidOCR  # noqa: PLC0415
    import rapidocr  # noqa: PLC0415

    config = Path(rapidocr.__file__).parent / "config.yaml"
    if config.is_file():
        print("RapidOCR's own settings (its packaged config.yaml, Det and Rec sections):")
        section = ""
        for row in config.read_text(encoding="utf-8").splitlines():
            if row and not row.startswith(" "):
                section = row.rstrip(":")
            if section in {"Det", "Rec", "Global"}:
                print("  " + row)
    engine = RapidOCR(params={
        "Global.use_cls": False, "Global.log_level": "warning",
        "Det.ocr_version": OCRVersion("PP-OCRv5"), "Det.model_type": ModelType("mobile"),
        "Det.lang_type": LangDet("multi"),
        "Rec.ocr_version": OCRVersion("PP-OCRv5"), "Rec.model_type": ModelType("mobile"),
        "Rec.lang_type": LangRec("latin"),
    })

    def recognise_crop(image: object) -> str:
        result = engine(image, use_det=False, use_cls=False, use_rec=True)
        texts = getattr(result, "txts", None) or ()
        return str(texts[0]).strip() if texts else ""

    def read_image(path: Path) -> object:
        return cv2.imread(str(path), cv2.IMREAD_COLOR)

    def crop(image: object, box: tuple[int, int, int, int]) -> object:
        x, y, width, height = box
        return image[y:y + height, x:x + width]  # type: ignore[index]

    report = {folder: crosscheck_frames(Path(arguments.work) / folder, recognise_crop, read_image, crop)
              for folder in arguments.frames}
    Path(arguments.report).write_text(json.dumps(report, indent=1) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=1))
    return 0


def command_compare(arguments: argparse.Namespace) -> int:
    linux = evaluation.read_recognitions(Path(arguments.linux))
    other = evaluation.read_recognitions(Path(arguments.other))
    result = agreement(linux, other)
    print(json.dumps(result))
    if result["agreement"] is None or result["agreement"] < MIN_PLATFORM_TEXT_AGREEMENT:
        print("the platforms read differently", file=sys.stderr)
        return 1
    return 0


def main(argv: Sequence[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    staging = commands.add_parser("stage")
    for option in ("--out", "--runner", "--inventory"):
        staging.add_argument(option, required=True)
    staging.add_argument("--platform", required=True, choices=sorted(RUNTIMES))
    staging.set_defaults(handler=command_stage)
    describing = commands.add_parser("describe")
    describing.add_argument("--runner", required=True)
    describing.set_defaults(handler=command_describe)
    recognising = commands.add_parser("recognise")
    recognising.add_argument("--runner", required=True)
    recognising.add_argument("--frames", action="append", required=True)
    recognising.add_argument("--threads", default="4")
    recognising.set_defaults(handler=command_recognise)
    comparing = commands.add_parser("parity")
    for option in ("--work", "--synthetic", "--report"):
        comparing.add_argument(option, required=True)
    comparing.add_argument("--frames", action="append", required=True)
    comparing.set_defaults(handler=command_parity)
    whole = commands.add_parser("end-to-end")
    for option in ("--vsift", "--ffmpeg", "--ffprobe", "--work", "--runner", "--synthetic", "--report"):
        whole.add_argument(option, required=True)
    whole.add_argument("--synthetic-video", action="append", default=[])
    whole.add_argument("--real", action="append", default=[])
    whole.add_argument("--max-calls", default="20")
    whole.set_defaults(handler=command_end_to_end)
    diagnosing = commands.add_parser("diagnose")
    for option in ("--work", "--report"):
        diagnosing.add_argument(option, required=True)
    diagnosing.add_argument("--frames", action="append", required=True)
    diagnosing.set_defaults(handler=command_diagnose)
    crossing = commands.add_parser("crosscheck")
    for option in ("--work", "--report"):
        crossing.add_argument(option, required=True)
    crossing.add_argument("--frames", action="append", required=True)
    crossing.set_defaults(handler=command_crosscheck)
    platform = commands.add_parser("compare")
    platform.add_argument("--linux", required=True)
    platform.add_argument("--other", required=True)
    platform.set_defaults(handler=command_compare)
    arguments = parser.parse_args(argv)
    try:
        return arguments.handler(arguments)
    except (RunnerCheckError, evaluation.EvaluationError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
