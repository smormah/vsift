"""Does a misread of a clock persist, and do the clocks work end to end? A hosted runner only (ADR 0029).

Only a disposable GitHub Actions runner may run this (`P16 OCR runner`, dispatched with
`clock_tracks`). It is the second and last evidence run of time anchoring stage 2, synthetic only:

- `persistence`: holds each value of a clock for a real 60 s of video at CRF 32, reads four frames of
  each minute, and counts how often a misread repeats across them. The excursion rule of ADR 0029
  decision 13 rests on that: a minute clock's pixels hold, so one misread may fill a whole minute.
- `tracks`: runs the commit's own `vsift ocr` and `vsift clocks` over recordings of known clocks, and
  holds what they report to what was drawn.

Numbers go to a report file as well as the job summary, because a job summary can be read only when
signed in.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
from pathlib import Path
import shutil
import sys
from typing import Callable, Sequence

import p16_ocr_evaluation as evaluation
from p16_ocr_runner import RunnerCheckError
import p16_taskbar_clocks as taskbar

# 12 px is the hardest size the first run read, and CRF 32 the only quality that misread anything.
# The reads come just after each change, where a rollover search reads, and once much later: the
# run before this one read only from 5 s on and found no misread at all.
PERSISTENCE_SIZE = 12
PERSISTENCE_SCALE = "1080p"
PERSISTENCE_FACES = ("24h", "12h")
VALUES_PER_RECORDING = 30
HOLD_S = 60
FRAME_RATE = 10
# Frames after a change: 0.1, 0.5, 1 and 2 s, then 30 s as the same value settled.
FRAMES_AFTER_CHANGE = (1, 5, 10, 20, 300)
READS_PER_VALUE = len(FRAMES_AFTER_CHANGE)
MAX_OCR_CALLS = 8
MAX_CLOCKS_CALLS = 6
START_SECOND = 14 * 3_600 + 3 * 60


def consecutive_minutes(face: str, start: int, count: int) -> list[str]:
    """A clock's values, one a minute, as a person's taskbar shows them."""
    return [taskbar.format_clock(face, (start + index * 60) // 3_600 % 24, (start + index * 60) // 60 % 60, 0)
            for index in range(count)]


def tally_value(truth: str, verdicts: Sequence[taskbar.Verdict]) -> dict:
    """What the reads of one held value gave."""
    wrong = [verdict.read for verdict in verdicts if verdict.outcome == "parsed_wrong"]
    return {"truth": truth, "reads": len(verdicts), "wrong": len(wrong), "distinct_wrong": len(set(wrong)),
            "exact": sum(verdict.outcome == "parsed_exact" for verdict in verdicts),
            "not_found": sum(verdict.outcome == "not_found" for verdict in verdicts), "wrong_reads": wrong,
            "outcomes": [verdict.outcome for verdict in verdicts]}


def persistence_report(values: Sequence[dict]) -> dict:
    """How far a misread persists across the reads of one held value.

    `all_reads_same_wrong` is every read of a minute giving one wrong value, the case the excursion
    rule was first reasoned from, and `some_reads_wrong` a misread that came and went. `still_wrong_settled`
    counts values misread just after the change and still wrong at the last read. `by_time_after_change`
    says where the misreads fall, which answers whether they gather where a rollover search reads.
    """
    misread = [value for value in values if value["wrong"] > 0]
    reads = sum(value["reads"] for value in values)
    histogram = {str(count): sum(value["wrong"] == count for value in misread) for count in range(1, READS_PER_VALUE + 1)}
    same = sum(value["wrong"] == value["reads"] and value["distinct_wrong"] == 1 for value in misread)
    by_time = {}
    for position, frames in enumerate(FRAMES_AFTER_CHANGE):
        outcomes = [value["outcomes"][position] for value in values if position < len(value["outcomes"])]
        by_time[f"{frames / FRAME_RATE:g}s"] = {outcome: outcomes.count(outcome)
                                                for outcome in ("parsed_exact", "parsed_wrong", "not_found")}
    settled = sum(value["outcomes"][-1] == "parsed_wrong" and "parsed_wrong" in value["outcomes"][:-1]
                  for value in misread)
    return {"values": len(values), "reads": reads, "reads_exact": sum(value["exact"] for value in values),
            "reads_wrong": sum(value["wrong"] for value in values),
            "reads_not_found": sum(value["not_found"] for value in values), "values_misread": len(misread),
            "wrong_reads_per_misread_value": histogram, "all_reads_same_wrong": same,
            "some_reads_wrong": len(misread) - same, "still_wrong_settled": settled,
            "by_time_after_change": by_time,
            "samples": [{"truth": value["truth"], "wrong": sorted(set(value["wrong_reads"])), "of_reads": value["reads"]}
                        for value in misread[:15]]}


def render(ffmpeg: Path, combination: taskbar.Combination, font: Path, folder: Path, clocks: Sequence[dict],
           body: Sequence[str], run: Callable[[list[str], Path], None]) -> Path:
    """Encodes a recording of `clocks`, each {x_from_right, values: [(from_s, to_s, text)]}, over `body`."""
    (folder / "text").mkdir(parents=True)
    width, height, bar = taskbar.SCALES[combination.scale]
    size = taskbar.SIZES[combination.size][combination.scale]
    colours = taskbar.BARS[combination.bar]

    def text_file(name: str, text: str) -> Path:
        relative = Path("text") / name
        (folder / relative).write_text(text, encoding="utf-8")
        return relative

    parts = [taskbar.drawtext(font, text_file(f"body-{index}.txt", line), size + 2, colours["text"], "120",
                              str(120 + index * 3 * size)) for index, line in enumerate(body)]
    parts.append(f"drawbox=x=0:y={height - bar}:w={width}:h={bar}:color={colours['bar']}:t=fill")
    seconds = 0
    for number, clock in enumerate(clocks):
        for index, (start, end, text) in enumerate(clock["values"]):
            parts.append(taskbar.drawtext(font, text_file(f"clock-{number}-{index:03d}.txt", text), size,
                                          colours["text"], f"w-text_w-{clock['x_from_right']}",
                                          str(height - bar + size // 3), f"{start},{end}"))
            seconds = max(seconds, end)
    command = taskbar.render_command(ffmpeg, combination, ",".join(parts), "video.mp4", seconds)
    command[command.index(f"color=c={colours['screen']}:s={width}x{height}:r={taskbar.FRAME_RATE}:d={seconds}")] = (
        f"color=c={colours['screen']}:s={width}x{height}:r={FRAME_RATE}:d={seconds}")
    run(command, folder)
    return folder / "video.mp4"


def frame_select() -> str:
    """The frames at each time after a change, in every held minute."""
    held = HOLD_S * FRAME_RATE
    terms = "+".join(f"eq(mod(n\\,{held})\\,{frames})" for frames in FRAMES_AFTER_CHANGE)
    return f"select={terms}"


def read_persistence(combination: taskbar.Combination, font: Path, ffmpeg: Path, executable: Path, work: Path,
                     run: Callable[[list[str], Path], None]) -> list[dict]:
    """One recording's values, each held a real minute, four reads of each."""
    folder = work / combination.name()
    values = consecutive_minutes(combination.face, START_SECOND, VALUES_PER_RECORDING)
    clock = {"x_from_right": 2 * taskbar.SIZES[combination.size][combination.scale],
             "values": [(index * HOLD_S, (index + 1) * HOLD_S, value) for index, value in enumerate(values)]}
    render(ffmpeg, combination, font, folder, [clock], taskbar.BODY_LINES, run)
    (folder / "frames").mkdir()
    count = VALUES_PER_RECORDING * READS_PER_VALUE
    run([str(ffmpeg), "-nostdin", "-v", "error", "-i", "video.mp4", "-vf", frame_select(), "-fps_mode", "vfr",
         "-frames:v", str(count), "frames/frame-%04d.png"], folder)
    frames = sorted((folder / "frames").glob("frame-*.png"))
    if len(frames) != count:
        raise taskbar.TaskbarCheckError(f"{len(frames)} frames of {count} were extracted")
    readings = taskbar.read_frames(executable, frames)
    tallied = []
    for index, truth in enumerate(values):
        lines = readings[index * READS_PER_VALUE:(index + 1) * READS_PER_VALUE]
        tallied.append(tally_value(truth, [taskbar.judge_frame(truth, reading) for reading in lines]))
    shutil.rmtree(folder)
    return tallied


@dataclasses.dataclass(frozen=True)
class Scene:
    """A recording of known clocks, and what `clocks` should say of it."""

    name: str
    clocks: tuple
    body: tuple
    expected_clocks: int
    expected_gaps: int


def minute_values(start: int, from_minute: int, to_minute: int) -> list[tuple[int, int, str]]:
    """Values of a 24-hour clock that shows `start` at time 0, from one minute to another."""
    return [(minute * 60, (minute + 1) * 60, taskbar.format_clock("24h", (start + minute * 60) // 3_600 % 24,
                                                                  (start + minute * 60) // 60 % 60, 0))
            for minute in range(from_minute, to_minute)]


def scenes() -> list[Scene]:
    """A clock alone, a cut, two zones, and a screen of times that are not clocks."""
    right, left = {"x_from_right": 24}, {"x_from_right": 1500}
    one = {**right, "values": minute_values(START_SECOND, 0, 6)}
    cut = {**right, "values": [*minute_values(START_SECOND, 0, 3), *minute_values(START_SECOND + 420, 3, 6)]}
    other_zone = {**left, "values": minute_values(START_SECOND + 5 * 3_600, 0, 6)}
    static = ("Message sent at 10:32", "Meeting 09:00", "Build finished, see the report for details")
    turns = [{**right, "values": [*minute_values(START_SECOND, 0, 2), *minute_values(START_SECOND, 4, 6)]},
             {**right, "values": minute_values(START_SECOND + 5 * 3_600, 2, 4)}]
    return [Scene("one-clock", (one,), taskbar.BODY_LINES, 1, 0),
            Scene("a-cut", (cut,), taskbar.BODY_LINES, 1, 1),
            Scene("two-zones", (one, other_zone), taskbar.BODY_LINES, 2, 0),
            Scene("turns", tuple(turns), taskbar.BODY_LINES, 2, 0),
            Scene("static-times", (one,), static, 1, 0)]


def run_scene(scene: Scene, font: Path, ffmpeg: Path, runner_path: Path, vsift: evaluation.Vsift, work: Path,
              run: Callable[[list[str], Path], None]) -> dict:
    """`ocr` until read, then `clocks` until its search ends, and what they said against the truth."""
    combination = taskbar.Combination("1080p", 14, font.stem, "light", "24h", "crf23")
    video = render(ffmpeg, combination, font, work / scene.name, scene.clocks, scene.body, run)
    session = vsift(f"ingest {scene.name}", ["ingest", str(video)])["data"]["session_id"]
    for number in range(MAX_OCR_CALLS):
        answer = vsift(f"ocr {number + 1}", ["ocr", session, "--runner", str(runner_path), "--max-frames", "200"])
        if answer["status"] == "complete":
            break
    result = {}
    calls = 0
    for calls in range(1, MAX_CLOCKS_CALLS + 1):
        answer = vsift(f"clocks {calls}", ["clocks", session, "--runner", str(runner_path)])
        result = answer["data"]
        if result["reads_pending"] == 0:
            break
    clocks = [clock for clock in result.get("clocks", []) if clock["kind"] == "clock"]
    gaps = [jump for clock in clocks for jump in clock["jumps"] if jump["kind"] == "gap"]
    report = {"scene": scene.name, "state": result.get("state"), "clocks_calls": calls, "clocks": len(clocks),
              "expected_clocks": scene.expected_clocks, "gaps": len(gaps), "expected_gaps": scene.expected_gaps,
              "gap_jump_us": [gap["jump_us"] for gap in gaps], "gap_uncertainty_us": [gap["uncertainty_us"] for gap in gaps],
              "precision_us": [clock["offset"]["precision_us"] for clock in clocks if clock["offset"]],
              "excursions": sum(clock["excursions_total"] for clock in clocks), "freezes": sum(len(clock["freezes"]) for clock in clocks),
              "excursion_spans": [{"from_us": excursion["from_us"], "to_us": excursion["to_us"], "reads": excursion["reads"]}
                                  for clock in clocks for excursion in clock["excursions"]],
              "stopped_brackets": sum(len(clock["stopped_brackets"]) for clock in clocks),
              "refused": result.get("refused"), "reads_pending": result.get("reads_pending")}
    report["matches"] = report["clocks"] == scene.expected_clocks and report["gaps"] == scene.expected_gaps
    vsift(f"close {scene.name}", ["session", "close", session])
    shutil.rmtree(work / scene.name)
    return report


def markdown(report: dict) -> str:
    """The job summary: the same numbers as the report file."""
    persistence = report.get("persistence", {})
    lines = ["## Clock misreads held for a minute (synthetic, 12 px, CRF 32)", "",
             f"{persistence.get('values', 0)} values, {persistence.get('reads', 0)} reads: "
             f"{persistence.get('reads_exact', 0)} exact, {persistence.get('reads_wrong', 0)} wrong, "
             f"{persistence.get('reads_not_found', 0)} not found.", "",
             f"Values misread: {persistence.get('values_misread', 0)}; every read the same wrong value: "
             f"{persistence.get('all_reads_same_wrong', 0)}; only some reads wrong: {persistence.get('some_reads_wrong', 0)}.",
             f"Still wrong at the last read: {persistence.get('still_wrong_settled', 0)}.",
             f"Wrong reads per misread value: {persistence.get('wrong_reads_per_misread_value', {})}",
             f"By time after the change: {persistence.get('by_time_after_change', {})}", "",
             "## Clocks end to end", "", "| Scene | Matches | Clocks | Gaps | Precision (us) | Excursions | State |",
             "| --- | --- | --- | --- | --- | --- | --- |"]
    for scene in report.get("tracks", []):
        lines.append(f"| {scene['scene']} | {scene['matches']} | {scene['clocks']} of {scene['expected_clocks']} | "
                     f"{scene['gaps']} of {scene['expected_gaps']} | {scene['precision_us']} | {scene['excursions']} | "
                     f"{scene['state']} |")
    return "\n".join(lines) + "\n"


def write(report: dict, report_path: Path, summary: Path) -> None:
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=1) + "\n", encoding="utf-8")
    with summary.open("a", encoding="utf-8") as handle:
        handle.write(markdown(report))


def command_persistence(arguments: argparse.Namespace) -> int:
    fonts = {Path(font).stem: Path(font) for font in arguments.font}
    work = Path(arguments.work)
    work.mkdir(parents=True, exist_ok=False)
    values: list[dict] = []
    for font in sorted(fonts):
        for bar in taskbar.BARS:
            for face in PERSISTENCE_FACES:
                combination = taskbar.Combination(PERSISTENCE_SCALE, PERSISTENCE_SIZE, font, bar, face, "crf32")
                values.extend(read_persistence(combination, fonts[font], Path(arguments.ffmpeg), Path(arguments.runner),
                                               work, taskbar.run_tool))
                print(f"{combination.name()} read", flush=True)
    write({"persistence": persistence_report(values)}, Path(arguments.report), Path(arguments.summary))
    return 0


def command_tracks(arguments: argparse.Namespace) -> int:
    font = Path(arguments.font)
    work = Path(arguments.work)
    work.mkdir(parents=True, exist_ok=False)
    vsift = evaluation.Vsift(Path(arguments.vsift), work / "root", work / "user")
    vsift("setup configure ffmpeg", ["setup", "configure", "ffmpeg", "--executable", arguments.pinned_ffmpeg])
    vsift("setup configure ffprobe", ["setup", "configure", "ffprobe", "--executable", arguments.pinned_ffprobe])
    tracks = [run_scene(scene, font, Path(arguments.ffmpeg), Path(arguments.runner), vsift, work, taskbar.run_tool)
              for scene in scenes()]
    write({"tracks": tracks}, Path(arguments.report), Path(arguments.summary))
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    persistence = commands.add_parser("persistence")
    persistence.add_argument("--font", action="append", required=True)
    tracks = commands.add_parser("tracks")
    tracks.add_argument("--font", required=True)
    tracks.add_argument("--vsift", required=True)
    tracks.add_argument("--pinned-ffmpeg", required=True)
    tracks.add_argument("--pinned-ffprobe", required=True)
    for command, handler in ((persistence, command_persistence), (tracks, command_tracks)):
        command.add_argument("--work", required=True)
        command.add_argument("--ffmpeg", required=True)
        command.add_argument("--runner", required=True)
        command.add_argument("--report", required=True)
        command.add_argument("--summary", required=True)
        command.set_defaults(handler=handler)
    arguments = parser.parse_args(argv)
    try:
        return arguments.handler(arguments)
    except (taskbar.TaskbarCheckError, RunnerCheckError, evaluation.EvaluationError) as error:
        print(f"failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
