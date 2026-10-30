"""Can the OCR runner read taskbar-sized clocks? A hosted runner only (ADR 0029, stage 2).

Only a disposable GitHub Actions runner may run this (`P16 OCR runner`, dispatched
with `taskbar_clocks`). It answers one question before stage 2 is built, with the
existing runner and nothing new in VSift:

- `synthetic`: draws desktops with a clock in a taskbar at known times, encodes them
  as a screen recording is, hands whole frames to the runner as `vsift ocr` does, and
  reports how many clocks parse exactly, how many parse wrongly, and the scores of each.
- `real`: reads one frame every 10 s of RM01, RM04 and RM10 and counts the lines of each
  time-shaped class. Only those numbers leave the runner: no text, no frames, no times
  of day and no per-line rows, as in `P16 OCR evaluation`. A time-shaped line is not a
  clock, so a count says only whether a recording could exercise stage 2's track step.

The grammar here approximates ADR 0029 decision 10 for this measurement. It is not the
product's: that belongs in the domain, and is written after this run has said whether
the story holds.
"""

from __future__ import annotations

import argparse
import collections
import dataclasses
import json
import math
from pathlib import Path
import random
import re
import shutil
import subprocess
import sys
from typing import Callable, Iterable, Sequence

import p16_ocr_runner as runner
from p16_ocr_evaluation import Line
from p16_ocr_runner import IMAGES_PER_RUN, RunnerCheckError

# The screens. A 1440p screen is drawn at the same physical size, then scaled down.
SCALES = {"1080p": (1920, 1080, 48), "1440p-scaled": (2560, 1440, 64)}
SCALED_OUTPUT = (1920, 1080)
# Text sizes, keyed by the size at 1080p, as drawn on each screen.
SIZES = {12: {"1080p": 12, "1440p-scaled": 16}, 13: {"1080p": 13, "1440p-scaled": 17},
         14: {"1080p": 14, "1440p-scaled": 19}}
BARS = {"light": {"bar": "0xF3F3F3", "text": "0x1B1B1B", "screen": "white"},
        "dark": {"bar": "0x202020", "text": "white", "screen": "0x1e1e1e"}}
FACES = ("24h", "24h_seconds", "12h", "12h_seconds")
# CRF 23 is the judged rate; 1 Mbit/s is reported at native 1080p only, to keep the run under an hour.
ENCODINGS = {"crf23": ["-crf", "23"], "1mbit": ["-b:v", "1000k", "-maxrate", "1500k", "-bufsize", "2000k"]}
JUDGED_ENCODING = "crf23"
JUDGED_SCALE = "1080p"
GO_SHARE = 2 / 3

VALUES_PER_COMBINATION = 20
HOLD_S = 2
FRAME_RATE = 30
THREADS = 4
RUN_TIMEOUT_S = 300
DATE_TEXT = "10/21/2026"
# Ordinary screen text around the clock, so the encoder spends bits as it does on a real
# screen. None of it is time-shaped.
BODY_LINES = ("Build finished, see the report for details", "Deployment notes for the next release",
              "Open items from the review", "Customer feedback, first batch")
SCROLLING_LINE = "Scrolling text moves as it does on a real screen"
BACKGROUND_GRAIN = 6
_FILTER_SAFE_PATH = re.compile(r"^[A-Za-z0-9_./-]+$")

READ_PERIOD_S = 10
MAX_REAL_FRAMES = 60
SCORE_MARKS = (0.50, 0.70, 0.80, 0.90)
LINE_CLASSES = ("24h", "24h_seconds", "12h", "dated", "states_zone")

_MERIDIEM = r"(?:\s?(?P<meridiem>[AaPp])\.?\s?[Mm]\.?)?"
_TOKEN = re.compile(r"(?<![0-9A-Za-z:./-])(?P<hour>\d{1,2}):(?P<minute>\d{2})(?::(?P<second>\d{2}))?"
                    + _MERIDIEM + r"(?![0-9A-Za-z:/-])(?!\.\d)")
_ZONE = r"(?P<zone>\s?(?:[Zz]|UTC|GMT|[+-]\d{2}:?\d{2})(?![0-9A-Za-z]))?"
_ISO_DATED = re.compile(r"(?<![0-9])\d{4}-\d{2}-\d{2}[ Tt]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?" + _ZONE)
_MONTH = r"(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*"
_MONTH_DATED = re.compile(rf"(?:\d{{1,2}} {_MONTH}|{_MONTH} \d{{1,2}},?) \d{{4}},? \d{{1,2}}:\d{{2}}(?::\d{{2}})?"
                          r"(?:\s?[AaPp]\.?\s?[Mm]\.?)?" + _ZONE)


class TaskbarCheckError(Exception):
    """A step failed; the message is fixed prose plus counts, never recognised text."""


@dataclasses.dataclass(frozen=True)
class Combination:
    """One recording of the set."""

    scale: str
    size: int
    font: str
    bar: str
    face: str
    encoding: str

    def values_key(self) -> str:
        """What the clock's values depend on: not the encoding, so the two rates compare like for like."""
        return f"{self.scale}/{self.size}/{self.font}/{self.bar}/{self.face}"

    def name(self) -> str:
        return f"{self.values_key()}/{self.encoding}".replace("/", "-")


def combinations(fonts: Sequence[str]) -> list[Combination]:
    """Every combination at the judged rate, and the second rate at native 1080p only."""
    result = []
    for scale in SCALES:
        for size in SIZES:
            for font in fonts:
                for bar in BARS:
                    for face in FACES:
                        for encoding in ENCODINGS:
                            if encoding == JUDGED_ENCODING or scale == JUDGED_SCALE:
                                result.append(Combination(scale, size, font, bar, face, encoding))
    return result


def format_clock(face: str, hour: int, minute: int, second: int) -> str:
    """The clock's text in `face`, as a person's desktop shows it."""
    if face == "24h":
        return f"{hour:02d}:{minute:02d}"
    if face == "24h_seconds":
        return f"{hour:02d}:{minute:02d}:{second:02d}"
    meridiem = "PM" if hour >= 12 else "AM"
    shown = hour % 12 or 12
    if face == "12h":
        return f"{shown}:{minute:02d} {meridiem}"
    if face == "12h_seconds":
        return f"{shown}:{minute:02d}:{second:02d} {meridiem}"
    raise TaskbarCheckError("unknown clock face")


def clock_values(combination: Combination, count: int = VALUES_PER_COMBINATION) -> list[str]:
    """`count` distinct clock texts, the same on every run and for both rates."""
    generator = random.Random(combination.values_key())
    values: list[str] = []
    while len(values) < count:
        text = format_clock(combination.face, generator.randrange(24), generator.randrange(60),
                            generator.randrange(60))
        if text not in values:
            values.append(text)
    return values


def time_tokens(text: str) -> list[tuple[str, str]]:
    """The time-shaped tokens of a line, each as (normalised text, class).

    A token needs a real time. A one-digit hour needs AM or PM, because `1:23` is a duration
    far more often than a time, and the hour of a 12-hour face runs from 1 to 12.
    """
    tokens = []
    for match in _TOKEN.finditer(text):
        hour, minute = int(match["hour"]), int(match["minute"])
        second = match["second"]
        meridiem = match["meridiem"]
        if minute > 59 or (second is not None and int(second) > 59):
            continue
        if meridiem is not None:
            if not 1 <= hour <= 12:
                continue
            clock = f"{match['hour']}:{match['minute']}" + (f":{second}" if second else "")
            tokens.append((f"{clock} {meridiem.upper()}M", "12h"))
        elif len(match["hour"]) == 2 and hour <= 23:
            clock = f"{match['hour']}:{match['minute']}" + (f":{second}" if second else "")
            tokens.append((clock, "24h_seconds" if second else "24h"))
    return tokens


def line_classes(text: str) -> set[str]:
    """Which time-shaped classes a line belongs to; a line counts once in each."""
    classes = {token_class for _, token_class in time_tokens(text)}
    for pattern in (_ISO_DATED, _MONTH_DATED):
        match = pattern.search(text)
        if match:
            classes.add("dated")
            if match["zone"]:
                classes.add("states_zone")
    return classes


@dataclasses.dataclass(frozen=True)
class Verdict:
    """What one frame gave for its clock."""

    outcome: str
    score: float | None
    read: str | None


def judge_frame(truth: str, lines: list[Line] | None) -> Verdict:
    """`parsed_exact` (the true token and nothing else time-shaped), `parsed_wrong` or `not_found`.

    A longer line holding the token counts. A time-shaped token that is not the truth, beside
    the truth or alone, is `parsed_wrong`, the case that would mislead a reader.
    """
    if lines is None:
        return Verdict("not_found", None, None)
    found = [(token, line) for line in lines for token, _ in time_tokens(line.text)]
    if not found:
        return Verdict("not_found", None, None)
    if len(found) == 1 and found[0][0] == truth:
        return Verdict("parsed_exact", found[0][1].confidence, found[0][0])
    wrong = next((pair for pair in found if pair[0] != truth), found[0])
    return Verdict("parsed_wrong", wrong[1].confidence, wrong[0])


def percentile(ordered: Sequence[float], percent: int) -> float:
    """The nearest-rank percentile of an ascending list."""
    return ordered[max(math.ceil(percent / 100 * len(ordered)) - 1, 0)]


def score_spread(scores: Iterable[float]) -> dict | None:
    """The minimum, 10th, 50th and 90th percentile of `scores`; `None` without any."""
    ordered = sorted(scores)
    if not ordered:
        return None
    return {"count": len(ordered), "min": round(ordered[0], 4), "p10": round(percentile(ordered, 10), 4),
            "p50": round(percentile(ordered, 50), 4), "p90": round(percentile(ordered, 90), 4)}


def bar_text(lines: list[Line] | None) -> str:
    """What was read in the bottom of the frame, for a sample of a miss (synthetic text only)."""
    height = SCALED_OUTPUT[1]
    return " | ".join(line.text for line in lines or [] if line.box[1] + line.box[3] / 2 >= height - 60)


class Tally:
    """Frames of a group of recordings, by outcome, with their scores."""

    def __init__(self) -> None:
        self.outcomes: collections.Counter = collections.Counter()
        self.scores: dict[str, list[float]] = {"parsed_exact": [], "parsed_wrong": []}

    def add(self, verdict: Verdict) -> None:
        self.outcomes[verdict.outcome] += 1
        if verdict.score is not None:
            self.scores[verdict.outcome].append(verdict.score)

    def report(self) -> dict:
        frames = sum(self.outcomes.values())
        share = {outcome: round(self.outcomes[outcome] / frames, 4) if frames else 0.0
                 for outcome in ("parsed_exact", "parsed_wrong", "not_found")}
        return {"frames": frames, **share, "scores_exact": score_spread(self.scores["parsed_exact"]),
                "scores_wrong": score_spread(self.scores["parsed_wrong"])}


def tally_groups(frames: Sequence[tuple[Combination, str, Verdict, str]]) -> dict:
    """The report: the verdict line, then each factor, then the second rate."""
    groups: dict[str, Tally] = collections.defaultdict(Tally)
    confusions: collections.Counter = collections.Counter()
    samples: list[dict] = []
    for combination, truth, verdict, seen in frames:
        if combination.encoding == JUDGED_ENCODING:
            if combination.scale == JUDGED_SCALE:
                groups["verdict"].add(verdict)
                groups[f"verdict/size/{combination.size}"].add(verdict)
                groups[f"verdict/face/{combination.face}"].add(verdict)
                groups[f"verdict/bar/{combination.bar}"].add(verdict)
                groups[f"verdict/font/{combination.font}"].add(verdict)
            groups[f"scale/{combination.scale}"].add(verdict)
        else:
            groups["rate/1mbit"].add(verdict)
        if verdict.outcome != "parsed_exact":
            read = verdict.read or seen
            if verdict.read and len(verdict.read) == len(truth):
                confusions.update(f"{left}>{right}" for left, right in zip(truth, verdict.read) if left != right)
            if len(samples) < 15:
                samples.append({"recording": combination.name(), "truth": truth, "read": read})
    report = {name: tally.report() for name, tally in sorted(groups.items())}
    verdict_share = report.get("verdict", {}).get("parsed_exact", 0.0)
    return {"go_share": round(GO_SHARE, 4), "goes": verdict_share >= GO_SHARE, "groups": report,
            "confusions": confusions.most_common(15), "samples": samples}


def count_recording(readings: Sequence[list[Line] | None]) -> dict:
    """The numbers that may leave the runner for one real recording: integers only, never text."""
    classes = {name: {"lines": 0, **{f"score_{mark:.2f}": 0 for mark in SCORE_MARKS}} for name in LINE_CLASSES}
    counts = {"frames_read": 0, "frames_failed": 0, "lines_read": 0, "classes": classes}
    for lines in readings:
        if lines is None:
            counts["frames_failed"] += 1
            continue
        counts["frames_read"] += 1
        counts["lines_read"] += len(lines)
        for line in lines:
            for name in line_classes(line.text):
                classes[name]["lines"] += 1
                for mark in SCORE_MARKS:
                    if line.confidence is not None and line.confidence >= mark:
                        classes[name][f"score_{mark:.2f}"] += 1
    return counts


def checked_filter_path(path: Path) -> str:
    text = path.as_posix()
    if not _FILTER_SAFE_PATH.match(text):
        raise TaskbarCheckError("a font or text path holds a character a filter would interpret")
    return text


def drawtext(font: Path, text_file: Path, size: int, colour: str, x: str, y: str, enable: str | None = None) -> str:
    """One `drawtext`, from a text file that is never expanded."""
    part = (f"drawtext=fontfile={checked_filter_path(font)}:textfile={checked_filter_path(text_file)}"
            f":expansion=none:x={x}:y={y}:fontsize={size}:fontcolor={colour}")
    return part + (f":enable='between(t,{enable})'" if enable else "")


def screen_filter(combination: Combination, font: Path, value_files: Sequence[Path], body_files: Sequence[Path],
                  scrolling_file: Path, date_file: Path) -> str:
    """The desktop: body text, a line that scrolls, the bar, the clock and its date, scaled if asked."""
    width, height, bar = SCALES[combination.scale]
    size = SIZES[combination.size][combination.scale]
    colours = BARS[combination.bar]
    # A still desktop costs the encoder almost nothing, so a fixed rate would never be felt.
    # Moving grain behind the text keeps it busy; the bar is drawn after it and stays clean.
    parts = [f"noise=alls={BACKGROUND_GRAIN}:allf=t"]
    parts += [drawtext(font, file, size + 2, colours["text"], "120", str(120 + index * 3 * size))
              for index, file in enumerate(body_files)]
    parts.append(drawtext(font, scrolling_file, size + 2, colours["text"], "120", f"{height // 2}+8*t"))
    parts.append(f"drawbox=x=0:y={height - bar}:w={width}:h={bar}:color={colours['bar']}:t=fill")
    clock_y, date_y = height - bar + size // 3, height - bar + size // 3 + size + size // 3
    for index, file in enumerate(value_files):
        parts.append(drawtext(font, file, size, colours["text"], f"w-text_w-{size * 2}", str(clock_y),
                              f"{index * HOLD_S},{(index + 1) * HOLD_S}"))
    parts.append(drawtext(font, date_file, size, colours["text"], f"w-text_w-{size * 2}", str(date_y)))
    if (width, height) != SCALED_OUTPUT:
        parts.append(f"scale={SCALED_OUTPUT[0]}:{SCALED_OUTPUT[1]}:flags=bicubic")
    return ",".join(parts)


def render_command(ffmpeg: Path, combination: Combination, screen: str, video: str, seconds: int) -> list[str]:
    """Encodes one recording, as screen recorders do: `veryfast`, 4:2:0, at the combination's rate."""
    width, height, _ = SCALES[combination.scale]
    background = BARS[combination.bar]["screen"]
    return [str(ffmpeg), "-nostdin", "-v", "error", "-f", "lavfi", "-i",
            f"color=c={background}:s={width}x{height}:r={FRAME_RATE}:d={seconds}", "-vf", screen,
            "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p", *ENCODINGS[combination.encoding], video]


def frames_command(ffmpeg: Path, video: str, count: int) -> list[str]:
    """The frame in the middle of each hold, whole, as PNG."""
    return [str(ffmpeg), "-nostdin", "-v", "error", "-ss", str(HOLD_S // 2), "-i", video, "-vf", f"fps=1/{HOLD_S}",
            "-frames:v", str(count), "frames/frame-%03d.png"]


def run_tool(command: list[str], cwd: Path) -> None:
    """A tool with an explicit argument list and no shell; a failure names no text."""
    completed = subprocess.run(command, cwd=cwd, capture_output=True, text=True, check=False)
    if completed.returncode != 0:
        print(completed.stderr[-2000:], file=sys.stderr)
        raise TaskbarCheckError(f"{Path(command[0]).name} exited {completed.returncode}")


def read_frames(executable: Path, frames: Sequence[Path]) -> list[list[Line] | None]:
    """The runner over `frames`, eight a run; whole frames, as `vsift ocr` hands them over."""
    readings: list[list[Line] | None] = []
    for start in range(0, len(frames), IMAGES_PER_RUN):
        batch = frames[start:start + IMAGES_PER_RUN]
        status, stdout, _seconds = runner.run_runner(
            executable, ["read", "--threads", str(THREADS), "--max-lines", "256", *[str(path) for path in batch]],
            timeout=RUN_TIMEOUT_S)
        if status != 0:
            raise TaskbarCheckError(f"read exited {status}")
        readings.extend(runner.parse_read(stdout, len(batch)))
    return readings


def measure(combination: Combination, font: Path, ffmpeg: Path, executable: Path, work: Path,
            run: Callable[[list[str], Path], None]) -> list[tuple[Combination, str, Verdict, str]]:
    """Renders, encodes and reads one recording, and removes its frames."""
    values = clock_values(combination)
    folder = work / combination.name()
    (folder / "text").mkdir(parents=True)
    (folder / "frames").mkdir()

    def text_file(name: str, text: str) -> Path:
        relative = Path("text") / name
        (folder / relative).write_text(text, encoding="utf-8")
        return relative

    value_files = [text_file(f"value-{index:02d}.txt", value) for index, value in enumerate(values)]
    body_files = [text_file(f"body-{index}.txt", line) for index, line in enumerate(BODY_LINES)]
    screen = screen_filter(combination, font, value_files, body_files, text_file("scrolling.txt", SCROLLING_LINE),
                           text_file("date.txt", DATE_TEXT))
    run(render_command(ffmpeg, combination, screen, "video.mp4", len(values) * HOLD_S), folder)
    run(frames_command(ffmpeg, "video.mp4", len(values)), folder)
    frames = sorted((folder / "frames").glob("frame-*.png"))
    if len(frames) != len(values):
        raise TaskbarCheckError(f"{len(frames)} frames of {len(values)} were extracted")
    readings = read_frames(executable, frames)
    measured = [(combination, truth, judge_frame(truth, lines), bar_text(lines))
                for truth, lines in zip(values, readings)]
    shutil.rmtree(folder)
    return measured


def percent(share: float) -> str:
    return f"{share * 100:.1f}%"


def spread_text(spread: dict | None) -> str:
    if spread is None:
        return "none"
    return f"{spread['min']} / {spread['p10']} / {spread['p50']} / {spread['p90']} (n={spread['count']})"


def synthetic_markdown(report: dict) -> str:
    """The job summary: the verdict line first."""
    verdict = report["groups"].get("verdict", {})
    lines = ["## Taskbar clocks (synthetic)", "",
             f"**Verdict line, native 1080p at CRF 23: {percent(verdict.get('parsed_exact', 0.0))} parse exactly, "
             f"{percent(verdict.get('parsed_wrong', 0.0))} parse wrongly, {percent(verdict.get('not_found', 0.0))} "
             f"not found. Two thirds needed: {'met' if report['goes'] else 'NOT met'}.**", "",
             "| Group | Frames | Exact | Wrong | Not found | Exact scores (min/p10/p50/p90) | Wrong scores |",
             "| --- | --- | --- | --- | --- | --- | --- |"]
    for name, group in report["groups"].items():
        lines.append(f"| {name} | {group['frames']} | {percent(group['parsed_exact'])} | "
                     f"{percent(group['parsed_wrong'])} | {percent(group['not_found'])} | "
                     f"{spread_text(group['scores_exact'])} | {spread_text(group['scores_wrong'])} |")
    lines += ["", f"Most common character confusions (truth>read): {report['confusions']}", "",
              "Samples of misses (synthetic text):", "```json", json.dumps(report["samples"], indent=1), "```", ""]
    return "\n".join(lines)


def real_markdown(counts: dict) -> str:
    """Numbers only. A time-shaped line is not a clock: this says whether the track step could run at all."""
    lines = ["## Time-shaped lines on real recordings (counts only)", "",
             "A time-shaped line is not a clock. These counts say only whether a recording could exercise "
             "the track step at all.", "",
             "| Recording | Frames read | Failed | Lines | Class | Lines of it | >=0.50 | >=0.70 | >=0.80 | >=0.90 |",
             "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"]
    for item, recording in counts.items():
        for name in LINE_CLASSES:
            row = recording["classes"][name]
            marks = " | ".join(str(row[f"score_{mark:.2f}"]) for mark in SCORE_MARKS)
            lines.append(f"| {item} | {recording['frames_read']} | {recording['frames_failed']} | "
                         f"{recording['lines_read']} | {name} | {row['lines']} | {marks} |")
    return "\n".join(lines) + "\n"


def command_synthetic(arguments: argparse.Namespace) -> int:
    fonts = {Path(font).stem: Path(font) for font in arguments.font}
    work = Path(arguments.work)
    work.mkdir(parents=True, exist_ok=False)
    frames = []
    chosen = combinations(sorted(fonts))
    for index, combination in enumerate(chosen):
        frames.extend(measure(combination, fonts[combination.font], Path(arguments.ffmpeg), Path(arguments.runner),
                              work, run_tool))
        print(f"{index + 1} of {len(chosen)} recordings read", flush=True)
    report = tally_groups(frames)
    Path(arguments.report).write_text(json.dumps(report, indent=1) + "\n", encoding="utf-8")
    with Path(arguments.summary).open("a", encoding="utf-8") as summary:
        summary.write(synthetic_markdown(report))
    return 0


def command_real(arguments: argparse.Namespace) -> int:
    work = Path(arguments.work)
    work.mkdir(parents=True, exist_ok=False)
    counts = {}
    for entry in arguments.real:
        item, video = entry.split("=", 1)
        folder = work / item
        (folder / "frames").mkdir(parents=True)
        run_tool([arguments.ffmpeg, "-nostdin", "-v", "error", "-i", video, "-vf", f"fps=1/{READ_PERIOD_S}",
                  "-frames:v", str(MAX_REAL_FRAMES), "frames/frame-%03d.png"], folder)
        counts[item] = count_recording(read_frames(Path(arguments.runner), sorted((folder / "frames").glob("*.png"))))
        shutil.rmtree(folder)
    with Path(arguments.summary).open("a", encoding="utf-8") as summary:
        summary.write(real_markdown(counts))
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    synthetic = commands.add_parser("synthetic")
    synthetic.add_argument("--work", required=True)
    synthetic.add_argument("--ffmpeg", required=True)
    synthetic.add_argument("--runner", required=True)
    synthetic.add_argument("--font", action="append", required=True)
    synthetic.add_argument("--report", required=True)
    synthetic.add_argument("--summary", required=True)
    synthetic.set_defaults(handler=command_synthetic)
    real = commands.add_parser("real")
    real.add_argument("--work", required=True)
    real.add_argument("--ffmpeg", required=True)
    real.add_argument("--runner", required=True)
    real.add_argument("--real", action="append", required=True)
    real.add_argument("--summary", required=True)
    real.set_defaults(handler=command_real)
    arguments = parser.parse_args(argv)
    try:
        return arguments.handler(arguments)
    except (TaskbarCheckError, RunnerCheckError) as error:
        print(f"failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
