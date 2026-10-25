"""The OCR engine evaluation for 0.3.0 (P16 enrichment, R-15): a hosted runner only.

Only a disposable GitHub Actions runner may run this (`P16 OCR evaluation`). It
compares candidate OCR engines on frames that VSift itself extracts, so that the
maintainer can choose the engine that a later change installs as a managed,
optional adapter. Nothing here ships and nothing here is an adapter.

The steps, each a subcommand the workflow runs in turn:

- `synthesize`: renders screens of interface-like text with known truth (labels
  and numbers at 8 to 22 px, light and dark), encodes them the way the real
  screen recordings are encoded (H.264 at RM10's bits per pixel, VP9 at RM04's),
  and writes the truth beside them. The screens themselves are the lossless
  control.
- `extract`: runs the checked-out commit's `vsift` over one video: `ingest`, the
  visual `candidates` (periodic ones omitted), then `frame get` for each, and
  keeps the frames' PNG files on the runner with their times.
- `recognise`: runs one engine over every extracted frame and records, per frame,
  its lines of text, their boxes, the engine's confidence and the time it took.
- `score`: measures each engine against the truth it can be held to and writes
  the report. Synthetic frames are scored line by line (character error rate,
  numbers read exactly, numbers misread). Real recordings are scored against
  interface labels whose presence a person can check in the product's own
  interface (`LEXICON`), never against a guess of what an engine should read.

The real recordings' recognised text stays on the runner, as their frames do: the
report keeps numbers, and for each label of `LEXICON` (our own list of an
interface's words) which engines found it. Synthetic text is ours and is kept.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import sys
import time
import unicodedata
from dataclasses import dataclass, field
from typing import Callable, Iterable, Sequence


SECOND = 1_000_000

# The synthetic screens (section 2 of docs/planning/ocr-engine-evaluation.md).
SCREEN_WIDTH = 1280
SCREEN_HEIGHT = 720
SCREEN_COUNT = 16
SCREEN_HOLD_S = 3
# Font sizes in pixels (FreeType's em size): RM04 and RM10 have a median text band
# of 8 px, which is about 10 px type, and a tenth of their text is 7 to 9 px type
# (small-text-real-media.md, section 6).
FONT_SIZES = (8, 9, 10, 11, 12, 14, 16, 18, 22)
COLUMN_X = (40, 660)
FIRST_LINE_Y = 28
# Each fourth screen is dark (light text on a dark background).
DARK_EVERY = 4

# The two encodings that imitate the real recordings, and their bit rates.
# RM10: H.264 1280x720 at 30 fps, under 0.016 bits per pixel, about 440 kbit/s.
# RM04: VP9 1280x720 at 24 fps, about 280 kbit/s with its audio.
ENCODINGS = {
    "synthetic-h264": {
        "file": "synthetic-h264.mp4",
        "video": ["-r", "30", "-c:v", "libx264", "-preset", "medium", "-b:v", "440k",
                  "-maxrate", "440k", "-bufsize", "880k", "-g", "150", "-pix_fmt", "yuv420p"],
        "audio": ["-c:a", "aac", "-b:a", "32k"],
    },
    "synthetic-h264-jitter": {
        "file": "synthetic-h264-jitter.mp4",
        "video": ["-vf", "crop=1272:712:x='4+4*sin(t*7)':y='4+4*cos(t*5)',pad=1280:720:4:4:color=white",
                  "-r", "30", "-c:v", "libx264", "-preset", "medium", "-b:v", "300k",
                  "-maxrate", "300k", "-bufsize", "600k", "-g", "60", "-pix_fmt", "yuv420p"],
        "audio": ["-c:a", "aac", "-b:a", "32k"],
    },
    "synthetic-vp9": {
        "file": "synthetic-vp9.webm",
        "video": ["-r", "24", "-c:v", "libvpx-vp9", "-b:v", "250k", "-g", "120",
                  "-deadline", "good", "-cpu-used", "4", "-row-mt", "1", "-pix_fmt", "yuv420p"],
        "audio": ["-c:a", "libopus", "-b:a", "24k"],
    },
}

# At most this many frames of one video are recognised: the candidates spread
# evenly. A session holds 384 evidence artifacts; each frame is one image and one
# record.
MAX_FRAMES_PER_VIDEO = 40

# Interface labels of each real recording's product, written from the products'
# own published interfaces, never from the recordings: MediaWiki's Vector skin of
# 2013 on the English Wikipedia (RM01), Wikimedia Commons in Brazilian Portuguese
# with its English fallbacks (RM04, 2019), and data.census.gov (RM10, 2022). Each
# entry is a list of spellings, any of which counts. A label no engine finds was
# probably not on any recognised frame; the report says so per label rather than
# hiding it, and its recall figures are relative to what some engine found.
LEXICON: dict[str, list[list[str]]] = {
    "RM01": [
        ["Wikipedia"], ["The Free Encyclopedia"], ["Main page"], ["Contents"],
        ["Featured content"], ["Current events"], ["Random article"],
        ["Donate to Wikipedia"], ["Interaction"], ["About Wikipedia"],
        ["Community portal"], ["Recent changes"], ["Contact Wikipedia", "Contact page"],
        ["Toolbox", "Tools"], ["What links here"], ["Related changes"], ["Upload file"],
        ["Special pages"], ["Permanent link"], ["Page information"], ["Print/export"],
        ["Languages"], ["Article"], ["Talk"], ["Read"], ["View history"], ["Search"],
        ["Watchlist"], ["Contributions"], ["Preferences"], ["Log out"],
        ["View and edit watchlist"], ["Edit raw watchlist"],
        ["From Wikipedia, the free encyclopedia"],
    ],
    "RM04": [
        ["Wikimedia Commons"], ["Página principal", "Main page"],
        ["Mudanças recentes", "Recent changes"], ["Carregar arquivo", "Upload file"],
        ["Ajuda", "Help"], ["Discussão", "Discussion"], ["Ler", "View"], ["Editar", "Edit"],
        ["Ver histórico", "View history"], ["Pesquisar", "Search"],
        ["Legendas", "Captions"], ["Dados estruturados", "Structured data"],
        ["Informações do arquivo", "File information"], ["Retrata", "depicts"],
        ["Publicar", "Publish"], ["Descrição", "Description"],
        ["Licenciamento", "Licensing"], ["Categorias", "Categories"],
        ["Português"], ["Wikidata"],
    ],
    "RM10": [
        ["census.gov"], ["Census Bureau"], ["Advanced Search"], ["Search"], ["Tables"],
        ["Maps"], ["Profiles"], ["Pages"], ["Filter"], ["Geography"], ["County"],
        ["Texas"], ["Customize Table"], ["Map"], ["Download"], ["Notes"],
        ["Margin of Error"], ["Estimate"], ["Total"], ["Population"], ["Topics"],
        ["Surveys"], ["Codes"], ["Years"], ["Geos"], ["Share"], ["Print"],
        ["American Community Survey"], ["United States"],
    ],
}

# Engine families: a word another family also read in the same frame is
# "corroborated". Two configurations of one engine family do not corroborate
# each other.
FAMILIES = {"tesseract": "tesseract", "ppocr": "ppocr", "ocrs": "ocrs"}

# Line text and words for the synthetic screens. Words are of the kind an
# interface shows; numbers are of the shapes where a misread digit is plausible.
LABELS = (
    "Harris County", "Total population", "Median income", "Rate", "Updated",
    "Bexar County", "Households", "Margin of error", "Last login", "Invoice",
    "Build", "Latency p95", "Disk used", "Requests", "Error budget", "Version",
    "Settings", "Search results", "Customize table", "Download", "Watchlist",
    "Recent changes", "Configurações", "Descrição", "Página principal",
    "Ação concluída", "Mudanças recentes", "Histórico", "Population estimate",
    "Payment due", "Open issues", "Memory", "Queue depth", "Uptime", "Balance",
    "Próxima página", "Último acesso", "Preço médio", "Total geral",
)


# Tesseract's trained models, pinned to a commit of their repositories and checked
# against the Git blob SHA-1 that the repository itself publishes for the file
# (`git hash-object`), before their SHA-256 is printed for a later pin. Both are
# Apache-2.0. `fast` is the integer-quantised LSTM, `best` the float one.
TESSDATA = {
    "fast": ("tesseract-ocr/tessdata_fast", "87416418657359cb625c412a48b6e1d6d41c29bd", {
        "eng": (4_113_088, "bbef4675053b5b468cdb477053e28b1c698ba08e"),
        "por": (1_982_756, "e9f373e95c66b4bf557c263721ef31f78e5bc301"),
    }),
    "best": ("tesseract-ocr/tessdata_best", "e12c65a915945e4c28e237a9b52bc4a8f39a0cec", {
        "eng": (15_400_601, "176dc3220de7db34d3b3aecbfa42043a6038348b"),
        "por": (8_159_939, "01743fdba931e706f79d86620ce18424036220f2"),
    }),
}
MAX_TESSDATA_BYTES = 32 * 1024 * 1024

# Tesseract and Leptonica sources, each a release tag whose commit is checked
# after the shallow clone. The build is the smallest that reads a frame: no image
# library at all (Leptonica reads PNM itself, and the evaluation hands Tesseract a
# PPM written by FFmpeg), no network (curl), no archives, no TIFF, no training
# tools, no graphics, no legacy engine and no OpenMP (one thread per process).
SOURCES = {
    "leptonica": ("https://github.com/DanBloomberg/leptonica.git", "1.87.0",
                  "13275a278eb55b5746e33f95fbf5a2c8f604b3ab"),
    "tesseract": ("https://github.com/tesseract-ocr/tesseract.git", "5.5.3",
                  "db0ec62f81b0737fbbe184d8fea40af5738f8eef"),
}
LEPTONICA_OPTIONS = ("-DBUILD_SHARED_LIBS=OFF", "-DSW_BUILD=OFF", "-DBUILD_PROG=OFF", "-DENABLE_ZLIB=OFF",
                     "-DENABLE_PNG=OFF", "-DENABLE_GIF=OFF", "-DENABLE_JPEG=OFF", "-DENABLE_TIFF=OFF",
                     "-DENABLE_WEBP=OFF", "-DENABLE_OPENJPEG=OFF")
TESSERACT_OPTIONS = ("-DBUILD_SHARED_LIBS=OFF", "-DBUILD_TRAINING_TOOLS=OFF", "-DDISABLE_CURL=ON",
                     "-DDISABLE_ARCHIVE=ON", "-DDISABLE_TIFF=ON", "-DGRAPHICS_DISABLED=ON",
                     "-DDISABLED_LEGACY_ENGINE=ON", "-DOPENMP_BUILD=OFF", "-DENABLE_CCACHE=OFF",
                     "-DINSTALL_CONFIGS=ON")


class EvaluationError(Exception):
    """An expected failure of the evaluation that stops the step with a message."""


# --- text normalisation and matching -------------------------------------------------------------

def normalise(text: str) -> str:
    """NFC, every run of whitespace one space, trimmed: case and punctuation kept."""
    return re.sub(r"\s+", " ", unicodedata.normalize("NFC", text)).strip()


def folded(text: str) -> str:
    """`normalise` and case-folded: for interface labels, whose case varies by skin."""
    return normalise(text).casefold()


_TOKEN_EDGE = "()[]{}\"'.,;:!?|«»“”‘’"


def tokens(text: str) -> list[str]:
    """Whitespace tokens with their edge punctuation removed (`15.2%,` -> `15.2%`)."""
    found = []
    for raw in normalise(text).split(" "):
        token = raw.strip(_TOKEN_EDGE)
        if token:
            found.append(token)
    return found


def is_number(token: str) -> bool:
    """A token that holds a digit: an amount, a count, a date, a time, a version."""
    return any(character.isdigit() for character in token)


def edit_distance(left: str, right: str) -> int:
    """Levenshtein distance (insertions, deletions and substitutions all cost one)."""
    if len(left) < len(right):
        left, right = right, left
    previous = list(range(len(right) + 1))
    for i, a in enumerate(left, start=1):
        current = [i]
        for j, b in enumerate(right, start=1):
            current.append(min(previous[j] + 1, current[j - 1] + 1, previous[j - 1] + (a != b)))
        previous = current
    return previous[-1]


def substring_distance(needle: str, haystack: str) -> int:
    """The least edit distance between `needle` and any substring of `haystack`.

    Semi-global alignment: a line of truth is found wherever the engine put it,
    whether the engine joined it to the column beside it or split it in two.
    """
    if not needle:
        return 0
    previous = [0] * (len(haystack) + 1)
    for i, a in enumerate(needle, start=1):
        current = [i]
        for j, b in enumerate(haystack, start=1):
            current.append(min(previous[j] + 1, current[j - 1] + 1, previous[j - 1] + (a != b)))
        previous = current
    return min(previous)


# --- engine output ------------------------------------------------------------------------------

@dataclass(frozen=True)
class Line:
    """One line an engine read: its text, its box (x, y, width, height) and its confidence."""

    text: str
    box: tuple[int, int, int, int]
    confidence: float | None


def parse_tesseract_tsv(tsv: str) -> list[Line]:
    """Lines of Tesseract's `tsv` output: words joined per (block, paragraph, line).

    A line's confidence is the mean of its words' confidences, scaled to 0..1;
    Tesseract writes -1 for a row that is not a word.
    """
    rows = tsv.splitlines()
    if not rows or not rows[0].startswith("level\t"):
        raise EvaluationError("Tesseract wrote no TSV header")
    header = rows[0].split("\t")
    lines: dict[tuple[int, int, int, int], list[tuple[str, int, int, int, int, float]]] = {}
    for row in rows[1:]:
        cells = row.split("\t")
        if len(cells) != len(header):
            continue
        record = dict(zip(header, cells))
        text = record.get("text", "").strip()
        if record.get("level") != "5" or not text:
            continue
        key = (int(record["page_num"]), int(record["block_num"]), int(record["par_num"]), int(record["line_num"]))
        lines.setdefault(key, []).append((text, int(record["left"]), int(record["top"]),
                                          int(record["width"]), int(record["height"]), float(record["conf"])))
    result = []
    for key in sorted(lines):
        words = lines[key]
        left = min(word[1] for word in words)
        top = min(word[2] for word in words)
        right = max(word[1] + word[3] for word in words)
        bottom = max(word[2] + word[4] for word in words)
        confidences = [word[5] for word in words if word[5] >= 0]
        confidence = statistics.fmean(confidences) / 100 if confidences else None
        result.append(Line(" ".join(word[0] for word in words), (left, top, right - left, bottom - top), confidence))
    return result


def box_of(points: Iterable[Sequence[float]]) -> tuple[int, int, int, int]:
    """The upright box around a polygon's points."""
    xs = []
    ys = []
    for point in points:
        xs.append(float(point[0]))
        ys.append(float(point[1]))
    if not xs:
        raise EvaluationError("a polygon without points")
    left = int(round(min(xs)))
    top = int(round(min(ys)))
    return (left, top, int(round(max(xs))) - left, int(round(max(ys))) - top)


def parse_ocrs_json(document: dict) -> list[Line]:
    """Lines of `ocrs --json`, which reports no confidence (HierText's layout)."""
    result = []
    for paragraph in document.get("paragraphs", []):
        for line in paragraph.get("lines", []):
            text = str(line.get("text", "")).strip()
            if text:
                result.append(Line(text, box_of(line.get("vertices", [])), None))
    return result


# --- synthetic screens --------------------------------------------------------------------------

def line_seed(screen: int, column: int, row: int) -> int:
    """A seed for one line, from a digest of its position, so that neither its label nor
    its number's shape follows its row (and so its font size) or its screen."""
    digest = hashlib.sha256(f"p16-ocr-v2/{screen}/{column}/{row}".encode("ascii")).digest()
    return int.from_bytes(digest[:8], "big")


def screen_lines(screen: int) -> list[dict]:
    """The deterministic lines of one synthetic screen: two columns, each size once."""
    lines = []
    for column, x in enumerate(COLUMN_X):
        y = FIRST_LINE_Y
        for row, size in enumerate(FONT_SIZES):
            seed = line_seed(screen, column, row)
            label = LABELS[seed % len(LABELS)]
            lines.append({"size": size, "column": column, "x": x, "y": y,
                          "text": f"{label} {number_text(seed // len(LABELS))}"})
            y += round(size * 1.9) + 10
    return lines


def number_text(seed: int) -> str:
    """One number of a shape where a misread digit is plausible, chosen by `seed`."""
    value = (seed // 8) % 10_000_000
    shape = seed % 8
    if shape == 0:
        return f"{value:,}"
    if shape == 1:
        return f"{(value % 1000) / 10:.1f}%"
    if shape == 2:
        return f"${value % 100_000:,}"
    if shape == 3:
        return f"2026-{value % 12 + 1:02d}-{value % 28 + 1:02d}"
    if shape == 4:
        return f"{value % 24:02d}:{value % 60:02d}"
    if shape == 5:
        return f"0.{value % 10_000:04d}"
    if shape == 6:
        return f"v{value % 10}.{value % 37}.{value % 101}"
    return f"#{value % 100_000}"


def screen_theme(screen: int) -> str:
    """`dark` for each fourth screen, else `light`."""
    return "dark" if screen % DARK_EVERY == DARK_EVERY - 1 else "light"


_FILTER_SAFE_PATH = re.compile(r"^[A-Za-z0-9_./-]+$")


def drawtext_filter(lines: list[dict], font: Path, text_files: list[Path], colour: str) -> str:
    """The filter chain that draws `lines` (each from its own text file, never expanded)."""
    for path in [font, *text_files]:
        if not _FILTER_SAFE_PATH.match(path.as_posix()):
            raise EvaluationError("a font or text file path holds a character a filter would interpret")
    parts = []
    for line, text_file in zip(lines, text_files):
        parts.append(
            f"drawtext=fontfile={font.as_posix()}:textfile={text_file.as_posix()}:expansion=none:"
            f"x={line['x']}:y={line['y']}:fontsize={line['size']}:fontcolor={colour}"
        )
    return ",".join(parts)


def synthesize(out: Path, ffmpeg: Path, fonts: list[Path], run: Callable[[list[str], Path], None]) -> dict:
    """Render the screens, encode both videos and write `truth.json`.

    FFmpeg runs in `out`, so the filter names each line's text file by a short
    relative path that holds no character a filter would interpret.
    """
    if not fonts:
        raise EvaluationError("at least one font is needed")
    screens_dir = out / "screens"
    text_dir = out / "text"
    screens_dir.mkdir(parents=True, exist_ok=False)
    text_dir.mkdir(parents=True, exist_ok=False)
    truth = {"width": SCREEN_WIDTH, "height": SCREEN_HEIGHT, "hold_us": SCREEN_HOLD_S * SECOND,
             "fonts": [font.name for font in fonts], "screens": []}
    for screen in range(SCREEN_COUNT):
        lines = screen_lines(screen)
        theme = screen_theme(screen)
        background, colour = ("0x1e1e1e", "0xe6e6e6") if theme == "dark" else ("white", "black")
        text_files = []
        for index, line in enumerate(lines):
            text_file = Path("text") / f"screen-{screen:02d}-line-{index:02d}.txt"
            (out / text_file).write_text(line["text"], encoding="utf-8")
            text_files.append(text_file)
        font = fonts[screen % len(fonts)]
        image = screens_dir / f"screen-{screen:02d}.png"
        run([str(ffmpeg), "-nostdin", "-v", "error", "-f", "lavfi", "-i",
             f"color=c={background}:s={SCREEN_WIDTH}x{SCREEN_HEIGHT}:d=1",
             "-vf", drawtext_filter(lines, font, text_files, colour), "-frames:v", "1", str(image)], out)
        truth["screens"].append({"index": screen, "theme": theme, "font": font.name,
                                 "start_us": screen * SCREEN_HOLD_S * SECOND,
                                 "end_us": (screen + 1) * SCREEN_HOLD_S * SECOND,
                                 "image": image.name, "lines": lines})
    duration = str(SCREEN_COUNT * SCREEN_HOLD_S)
    for name, encoding in ENCODINGS.items():
        run([str(ffmpeg), "-nostdin", "-v", "error", "-framerate", f"1/{SCREEN_HOLD_S}",
             "-i", str(screens_dir / "screen-%02d.png"), "-f", "lavfi", "-i", "anullsrc=r=48000:cl=mono",
             *encoding["video"], *encoding["audio"], "-t", duration, str(out / encoding["file"])], out)
        truth.setdefault("videos", {})[name] = encoding["file"]
    (out / "truth.json").write_text(json.dumps(truth, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    return truth


def write_control(synthetic: Path, out: Path) -> dict:
    """The lossless control: the rendered screens themselves, as a frames folder, one per screen."""
    truth = json.loads((synthetic / "truth.json").read_text(encoding="utf-8"))
    out.mkdir(parents=True, exist_ok=False)
    frames = []
    for screen in truth["screens"]:
        shutil.copyfile(synthetic / "screens" / screen["image"], out / screen["image"])
        frames.append({"file": screen["image"], "time_us": screen["start_us"] + truth["hold_us"] // 2,
                       "width": truth["width"], "height": truth["height"], "reasons": ["control"], "stability": None})
    record = {"video": "screens", "candidates": len(frames), "frames": frames, "gaps": []}
    (out / "frames.json").write_text(json.dumps(record, indent=1) + "\n", encoding="utf-8")
    return record


# --- extraction through vsift -------------------------------------------------------------------

@dataclass
class Vsift:
    """The commit's own `vsift`, run with a private session root and private user folders."""

    executable: Path
    root: Path
    user: Path
    log: Callable[[str], None] = print
    steps: list[dict] = field(default_factory=list)

    def environment(self) -> dict[str, str]:
        user = str(self.user)
        return {**os.environ, "HOME": user, "XDG_CONFIG_HOME": f"{user}/config", "XDG_CACHE_HOME": f"{user}/cache",
                "XDG_DATA_HOME": f"{user}/data", "LOCALAPPDATA": user, "APPDATA": f"{user}/roaming"}

    def __call__(self, step: str, arguments: list[str]) -> dict:
        command = [str(self.executable), "--session-root", str(self.root), *arguments, "--json"]
        started = time.monotonic()
        completed = subprocess.run(command, capture_output=True, text=True, env=self.environment(), timeout=900,
                                   check=False)
        seconds = time.monotonic() - started
        try:
            answer = json.loads(completed.stdout)
        except json.JSONDecodeError:
            answer = None
        code = answer.get("error", {}).get("code") if isinstance(answer, dict) and answer.get("error") else None
        self.steps.append({"step": step, "exit": completed.returncode, "seconds": round(seconds, 2), "code": code})
        self.log(f"  {step}: exit {completed.returncode} in {seconds:.1f} s{f' ({code})' if code else ''}")
        if completed.returncode != 0 or not isinstance(answer, dict):
            raise EvaluationError(f"vsift {step} failed with exit {completed.returncode} ({code or 'no JSON answer'})")
        return answer


def spread(items: list, limit: int) -> list:
    """At most `limit` of `items`, evenly spaced, the first and last kept."""
    if len(items) <= limit:
        return list(items)
    if limit <= 1:
        return items[:limit]
    step = (len(items) - 1) / (limit - 1)
    return [items[round(index * step)] for index in range(limit)]


def all_candidates(vsift: Vsift, session: str, duration_us: int, omit: Sequence[str]) -> tuple[list[dict], list[dict]]:
    """Every visual candidate of the session but the reasons in `omit`, with the coverage gaps."""
    items: list[dict] = []
    gaps: list[dict] = []
    window = 300 * SECOND
    for start in range(0, duration_us, window):
        end = min(duration_us, start + window)
        cursor = None
        while True:
            arguments = ["candidates", session, "--from", str(start), "--to", str(end), "--limit", "100"]
            for reason in omit:
                arguments += ["--omit", reason]
            if cursor:
                arguments += ["--cursor", cursor]
            label = f"candidates {start // SECOND}-{end // SECOND} s{' (omit ' + ','.join(omit) + ')' if omit else ''}"
            data = vsift(label, arguments)["data"]
            items.extend(data["items"])
            gaps.extend((data.get("coverage") or {}).get("gaps") or [])
            cursor = data.get("next_cursor")
            if not cursor:
                break
    return items, gaps


def extract(vsift: Vsift, video: Path, duration_us: int, out: Path, at_us: Sequence[int] = (),
            limit: int = MAX_FRAMES_PER_VIDEO) -> dict:
    """Ingest `video`, take its candidates' frames and copy them into `out` with their times.

    Every candidate counts, periodic ones included: on the first run the change
    rule alone gave 9 candidates for 16 screens whose text changed but whose
    layout did not, so the periodic candidates are what may reach such a screen.
    How many candidates there are without the periodic ones is recorded beside.

    With `at_us` (the synthetic videos: the middle of each screen), the frames
    are `frame get --at` those times instead, so that every screen is read; the
    candidates' times are still recorded, to say which screens they reached.
    """
    out.mkdir(parents=True, exist_ok=False)
    session = vsift("ingest", ["ingest", str(video)])["data"]["session_id"]
    without_periodic, _ = all_candidates(vsift, session, duration_us, ("periodic",))
    candidates, gaps = all_candidates(vsift, session, duration_us, ())
    if at_us:
        requests = [(["--at", str(time_us)], {"reasons": ["requested"], "stability": None}) for time_us in at_us]
    else:
        requests = [(["--candidate", candidate["candidate_id"]],
                     {"reasons": candidate.get("reasons", []), "stability": candidate.get("stability")})
                    for candidate in spread(candidates, limit)]
    frames = []
    for number, (selector, origin) in enumerate(requests):
        data = vsift("frame get", ["frame", "get", session, *selector])["data"]
        item = data["items"][0]
        source = Path(data["files"][0]["path"])
        target = out / f"frame-{number:03d}.png"
        shutil.copyfile(source, target)
        frames.append({"file": target.name, "time_us": item["frame"]["time_us"], "width": item["frame"]["width"],
                       "height": item["frame"]["height"], **origin})
    vsift("session close", ["session", "close", session])
    record = {"video": video.name, "candidates": len(candidates), "candidates_without_periodic": len(without_periodic),
              "candidate_times_us": sorted(candidate["representative_us"] for candidate in candidates),
              "frames": frames,
              "gaps": [{"reason": gap.get("reason"), "from_us": gap.get("from_us"), "to_us": gap.get("to_us")}
                       for gap in gaps]}
    (out / "frames.json").write_text(json.dumps(record, indent=1) + "\n", encoding="utf-8")
    return record


# --- recognition --------------------------------------------------------------------------------

@dataclass(frozen=True)
class Recognition:
    """One engine's reading of one frame, the wall time it took and the CPU time it used.

    The CPU time sums every thread: an engine that spreads one frame over four
    cores uses up to four CPU seconds a wall second. `None` where it cannot be read.
    """

    file: str
    lines: list[Line]
    seconds: float
    cpu_seconds: float | None = None


def child_cpu_seconds() -> float | None:
    """User and system CPU time of every finished child process so far (Linux and macOS)."""
    try:
        import resource  # noqa: PLC0415 - not on Windows, where the evaluation never runs
    except ImportError:
        return None
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    return usage.ru_utime + usage.ru_stime


def cpu_used(before: float | None, after: float | None) -> float | None:
    """The CPU time between two readings, or `None` when either is unknown."""
    return None if before is None or after is None else after - before


def scratch_file(scratch: Path, image: Path, suffix: str) -> Path:
    """A scratch file for one frame, named by its folder too: every video's frames are
    named `frame-000.png` and up, and a reading must never be of another video's frame."""
    return scratch / f"{image.parent.name}--{image.stem}{suffix}"


class Engine:
    """An engine configuration under evaluation."""

    name: str
    family: str

    def recognise(self, image: Path, language: str) -> tuple[list[Line], float, float | None]:
        raise NotImplementedError

    def describe(self) -> dict:
        return {"name": self.name, "family": self.family}


class TesseractEngine(Engine):
    """The Tesseract CLI over a PPM copy of the frame (the build reads no PNG: section 3).

    With `scale` 2 the copy is enlarged twice (Lanczos) for recognition only, the usual
    remedy for Tesseract on small screen text; the boxes are scaled back to the frame.
    """

    def __init__(self, name: str, executable: Path, tessdata: Path, ffmpeg: Path, scratch: Path,
                 scale: int = 1, page_mode: str = "3") -> None:
        self.name = name
        self.family = "tesseract"
        self.executable = executable
        self.tessdata = tessdata
        self.ffmpeg = ffmpeg
        self.scratch = scratch
        self.scale = scale
        # 3 is Tesseract's automatic page segmentation; 11 is "sparse text", which looks for
        # as much text as it can in no particular order, as a screen often holds.
        self.page_mode = page_mode

    def describe(self) -> dict:
        return {**super().describe(), "scale": self.scale, "page_mode": self.page_mode}

    def recognise(self, image: Path, language: str) -> tuple[list[Line], float, float | None]:
        self.scratch.mkdir(parents=True, exist_ok=True)
        ppm = scratch_file(self.scratch, image, ".ppm")
        if not ppm.exists():
            scaling = ["-vf", f"scale=iw*{self.scale}:ih*{self.scale}:flags=lanczos"] if self.scale != 1 else []
            subprocess.run([str(self.ffmpeg), "-nostdin", "-v", "error", "-i", str(image), "-frames:v", "1",
                            *scaling, "-c:v", "ppm", "-f", "image2", str(ppm)], check=True, timeout=120)
        languages = {"eng": "eng", "por": "por+eng"}[language]
        cpu_before = child_cpu_seconds()
        started = time.monotonic()
        completed = subprocess.run([str(self.executable), str(ppm), "stdout", "--tessdata-dir", str(self.tessdata),
                                    "-l", languages, "--oem", "1", "--psm", self.page_mode,
                                    "-c", "tessedit_create_tsv=1", "-c", "tessedit_create_txt=0"],
                                   capture_output=True, text=True, timeout=300, check=False,
                                   env={**os.environ, "OMP_THREAD_LIMIT": "1"})
        seconds = time.monotonic() - started
        if completed.returncode != 0:
            raise EvaluationError(f"{self.name} exited {completed.returncode}")
        cpu = cpu_used(cpu_before, child_cpu_seconds())
        return [unscaled(line, self.scale) for line in parse_tesseract_tsv(completed.stdout)], seconds, cpu


def unscaled(line: Line, scale: int) -> Line:
    """A line read from an enlarged copy, with its box in the frame's own pixels."""
    if scale == 1:
        return line
    x, y, width, height = line.box
    return Line(line.text, (x // scale, y // scale, -(-width // scale), -(-height // scale)), line.confidence)


class OcrsEngine(Engine):
    """The `ocrs` CLI with its default models, read from its cache."""

    def __init__(self, name: str, executable: Path, scratch: Path) -> None:
        self.name = name
        self.family = "ocrs"
        self.executable = executable
        self.scratch = scratch

    def recognise(self, image: Path, language: str) -> tuple[list[Line], float, float | None]:
        self.scratch.mkdir(parents=True, exist_ok=True)
        output = scratch_file(self.scratch, image, ".json")
        cpu_before = child_cpu_seconds()
        started = time.monotonic()
        completed = subprocess.run([str(self.executable), str(image), "--json", "-o", str(output)],
                                   capture_output=True, text=True, timeout=300, check=False)
        seconds = time.monotonic() - started
        if completed.returncode != 0:
            raise EvaluationError(f"{self.name} exited {completed.returncode}")
        cpu = cpu_used(cpu_before, child_cpu_seconds())
        return parse_ocrs_json(json.loads(output.read_text(encoding="utf-8"))), seconds, cpu


class RapidOcrEngine(Engine):
    """PP-OCR ONNX models through RapidOCR's reference pipeline, in this process.

    The evaluation measures the models; a shipped adapter would run them from a
    standalone runner (section 4). The model is loaded once; its load time is
    reported apart from the per-frame time.
    """

    def __init__(self, name: str, version: str, det_type: str, det_lang: str, rec_type: str, rec_lang: str,
                 threads: int = -1) -> None:
        self.name = name
        self.family = "ppocr"
        from rapidocr import LangDet, LangRec, ModelType, OCRVersion, RapidOCR  # noqa: PLC0415 - hosted only

        started = time.monotonic()
        self.engine = RapidOCR(params={
            "Global.use_cls": False, "Global.log_level": "warning",
            "Det.ocr_version": OCRVersion(version), "Det.model_type": ModelType(det_type),
            "Det.lang_type": LangDet(det_lang),
            "Rec.ocr_version": OCRVersion(version), "Rec.model_type": ModelType(rec_type),
            "Rec.lang_type": LangRec(rec_lang),
            # -1 is ONNX Runtime's default: one thread per core.
            "EngineConfig.onnxruntime.intra_op_num_threads": threads,
        })
        self.load_seconds = time.monotonic() - started
        self.settings = {"version": version, "det": f"{det_type}/{det_lang}", "rec": f"{rec_type}/{rec_lang}",
                         "threads": threads}

    def describe(self) -> dict:
        return {**super().describe(), **self.settings, "load_seconds": round(self.load_seconds, 2)}

    def recognise(self, image: Path, language: str) -> tuple[list[Line], float, float | None]:
        cpu_before = time.process_time()
        started = time.monotonic()
        result = self.engine(str(image), use_cls=False)
        seconds = time.monotonic() - started
        cpu = time.process_time() - cpu_before
        lines = []
        if result.txts is not None and result.boxes is not None and result.scores is not None:
            for text, box, score in zip(result.txts, result.boxes, result.scores):
                if str(text).strip():
                    lines.append(Line(str(text).strip(), box_of(box), float(score)))
        return lines, seconds, cpu


def recognise_all(engine: Engine, frames_dir: Path, language: str, out: Path) -> list[Recognition]:
    """Run `engine` over every frame of `frames_dir` and write `out` (one JSON object per frame)."""
    record = json.loads((frames_dir / "frames.json").read_text(encoding="utf-8"))
    readings = []
    with out.open("w", encoding="utf-8") as handle:
        for frame in record["frames"]:
            lines, seconds, cpu = engine.recognise(frames_dir / frame["file"], language)
            readings.append(Recognition(frame["file"], lines, seconds, cpu))
            handle.write(json.dumps({"file": frame["file"], "seconds": round(seconds, 4),
                                     "cpu_seconds": None if cpu is None else round(cpu, 4),
                                     "lines": [{"text": line.text, "box": list(line.box),
                                                "confidence": line.confidence} for line in lines]},
                                    ensure_ascii=False) + "\n")
    return readings


def read_recognitions(path: Path) -> list[Recognition]:
    """The readings `recognise_all` wrote."""
    readings = []
    for row in path.read_text(encoding="utf-8").splitlines():
        if row.strip():
            value = json.loads(row)
            readings.append(Recognition(value["file"], [Line(line["text"], tuple(line["box"]), line["confidence"])
                                                        for line in value["lines"]], value["seconds"],
                                        value.get("cpu_seconds")))
    return readings


# --- staging on the runner ---------------------------------------------------------------------

def git_blob_sha1(content: bytes) -> str:
    """The SHA-1 Git names a file's content by (`git hash-object`)."""
    return hashlib.sha1(b"blob %d\0" % len(content) + content).hexdigest()


def bounded_https_get(url: str, limit: int) -> bytes:
    """The body of an HTTPS GET, refused past `limit` bytes; redirects must stay on HTTPS."""
    import urllib.request  # noqa: PLC0415 - only the hosted staging needs the network

    from p06_windows_candidate_smoke import HttpsRedirectsOnly  # noqa: PLC0415

    if not url.startswith("https://"):
        raise EvaluationError("only HTTPS downloads are allowed")
    opener = urllib.request.build_opener(HttpsRedirectsOnly())
    request = urllib.request.Request(url, headers={"User-Agent": "vsift-p16-ocr-evaluation/1"})
    with opener.open(request, timeout=60) as response:
        if not response.geturl().startswith("https://"):
            raise EvaluationError("the download ended outside HTTPS")
        body = response.read(limit + 1)
    if len(body) > limit:
        raise EvaluationError("the download exceeds its bound")
    return body


def fetch_tessdata(out: Path, variants: Iterable[str], get: Callable[[str, int], bytes]) -> list[dict]:
    """Every pinned trained model of `variants` into `out/<variant>/`, each checked before it is written."""
    fetched = []
    for variant in variants:
        repository, commit, files = TESSDATA[variant]
        folder = out / variant
        folder.mkdir(parents=True, exist_ok=False)
        for language, (size, blob) in files.items():
            url = f"https://raw.githubusercontent.com/{repository}/{commit}/{language}.traineddata"
            content = get(url, MAX_TESSDATA_BYTES)
            if len(content) != size or git_blob_sha1(content) != blob:
                raise EvaluationError(f"{variant}/{language}.traineddata differs from its pin")
            (folder / f"{language}.traineddata").write_bytes(content)
            fetched.append({"variant": variant, "language": language, "url": url, "bytes": size,
                            "git_blob_sha1": blob, "sha256": hashlib.sha256(content).hexdigest()})
    return fetched


def build_commands(source_root: Path, build_root: Path, prefix: Path) -> list[list[str]]:
    """The clone, configure, build and install commands of the minimal Tesseract build."""
    commands: list[list[str]] = []
    for name, (url, tag, _commit) in SOURCES.items():
        commands.append(["git", "clone", "--quiet", "--depth", "1", "--branch", tag, url, str(source_root / name)])
    options = {"leptonica": LEPTONICA_OPTIONS,
               "tesseract": (*TESSERACT_OPTIONS, f"-DLeptonica_DIR={(prefix / 'lib' / 'cmake' / 'leptonica').as_posix()}")}
    for name in SOURCES:
        commands.append(["cmake", "-S", str(source_root / name), "-B", str(build_root / name),
                         "-DCMAKE_BUILD_TYPE=Release", f"-DCMAKE_INSTALL_PREFIX={prefix.as_posix()}",
                         f"-DCMAKE_PREFIX_PATH={prefix.as_posix()}", "-DCMAKE_INSTALL_LIBDIR=lib", *options[name]])
        commands.append(["cmake", "--build", str(build_root / name), "--config", "Release", "--parallel"])
        commands.append(["cmake", "--install", str(build_root / name), "--config", "Release"])
    return commands


def build_tesseract(source_root: Path, build_root: Path, prefix: Path,
                    run: Callable[[list[str], Path], None], head: Callable[[Path], str]) -> None:
    """Clone both pinned sources, check their commits, then build and install them into `prefix`."""
    commands = build_commands(source_root, build_root, prefix)
    clones = len(SOURCES)
    for command in commands[:clones]:
        run(command, source_root)
    for name, (_url, tag, commit) in SOURCES.items():
        if head(source_root / name) != commit:
            raise EvaluationError(f"{name} {tag} is not at its pinned commit")
    for command in commands[clones:]:
        run(command, build_root)


def git_head(folder: Path) -> str:
    """The commit a clone has checked out."""
    completed = subprocess.run(["git", "-C", str(folder), "rev-parse", "HEAD"], capture_output=True, text=True,
                               timeout=60, check=False)
    if completed.returncode != 0:
        raise EvaluationError("git rev-parse failed")
    return completed.stdout.strip()


# --- scoring ------------------------------------------------------------------------------------

def frame_text(lines: list[Line]) -> str:
    """Every line of a frame, one per row, as the matcher reads them."""
    return "\n".join(normalise(line.text) for line in lines)


def screen_of(time_us: int, hold_us: int, count: int) -> int | None:
    """The synthetic screen shown at `time_us`, or `None` past the last one."""
    index = time_us // hold_us
    return index if 0 <= index < count else None


def score_synthetic_frame(truth_lines: list[dict], lines: list[Line]) -> list[dict]:
    """Each truth line's character error rate, and whether each of its numbers was read exactly.

    A number is misread when the engine wrote a different number within two
    edits of it in the same frame (`4,731,14!` for `4,731,145` is a misread; a
    number missing altogether is not).
    """
    text = frame_text(lines)
    read_tokens = tokens(" ".join(line.text for line in lines))
    read_set = set(read_tokens)
    truth_numbers = {token for line in truth_lines for token in tokens(line["text"]) if is_number(token)}
    scores = []
    for line in truth_lines:
        expected = normalise(line["text"])
        distance = substring_distance(expected, text)
        numbers = [token for token in tokens(line["text"]) if is_number(token)]
        found = [number for number in numbers if number in read_set]
        misread = []
        for number in numbers:
            if number in read_set:
                continue
            close = [token for token in read_tokens if is_number(token) and token not in truth_numbers
                     and edit_distance(token, number) <= 2]
            if close:
                misread.append(number)
        scores.append({"size": line["size"], "cer": distance / len(expected), "exact": distance == 0,
                       "numbers": len(numbers), "numbers_found": len(found), "numbers_misread": len(misread)})
    return scores


def summarise_lines(scores: list[dict]) -> dict:
    """Totals over scored lines: exact share, median and mean CER, number recall and misreads."""
    if not scores:
        return {"lines": 0}
    numbers = sum(score["numbers"] for score in scores)
    return {
        "lines": len(scores),
        "exact_share": round(sum(score["exact"] for score in scores) / len(scores), 4),
        "median_cer": round(statistics.median(score["cer"] for score in scores), 4),
        "mean_cer": round(statistics.fmean(min(1.0, score["cer"]) for score in scores), 4),
        "numbers": numbers,
        "numbers_found_share": round(sum(score["numbers_found"] for score in scores) / numbers, 4) if numbers else None,
        "numbers_misread": sum(score["numbers_misread"] for score in scores),
    }


def score_synthetic(truth: dict, frames: dict, readings: list[Recognition]) -> dict:
    """One engine on one synthetic video: per font size, per theme and overall, one frame per screen."""
    by_file = {reading.file: reading for reading in readings}
    seen = set()
    all_scores = []
    for frame in frames["frames"]:
        screen = screen_of(frame["time_us"], truth["hold_us"], len(truth["screens"]))
        if screen is None or screen in seen or frame["file"] not in by_file:
            continue
        seen.add(screen)
        reading = by_file[frame["file"]]
        for score in score_synthetic_frame(truth["screens"][screen]["lines"], reading.lines):
            all_scores.append({**score, "theme": truth["screens"][screen]["theme"]})
    sizes = {}
    for size in FONT_SIZES:
        sizes[str(size)] = summarise_lines([score for score in all_scores if score["size"] == size])
    reached = sorted({screen for time_us in frames.get("candidate_times_us", [])
                      if (screen := screen_of(time_us, truth["hold_us"], len(truth["screens"]))) is not None})
    return {
        "screens_reached_by_candidates": reached,
        "screens_scored": len(seen),
        "screens": len(truth["screens"]),
        "overall": summarise_lines(all_scores),
        "dark": summarise_lines([score for score in all_scores if score["theme"] == "dark"]),
        "by_size": sizes,
        "seconds_per_frame": timing(readings),
    }


def timing(readings: list[Recognition]) -> dict:
    """Median and 90th-percentile wall seconds per frame, and the median CPU seconds."""
    seconds = sorted(reading.seconds for reading in readings)
    if not seconds:
        return {"frames": 0}
    p90 = seconds[min(len(seconds) - 1, int(round(0.9 * (len(seconds) - 1))))]
    cpu = [reading.cpu_seconds for reading in readings if reading.cpu_seconds is not None]
    return {"frames": len(seconds), "median": round(statistics.median(seconds), 3), "p90": round(p90, 3),
            "total": round(sum(seconds), 2), "cpu_median": round(statistics.median(cpu), 3) if cpu else None}


def word_set(lines: list[Line]) -> set[str]:
    """The frame's words of four letters or more, case-folded: what corroboration compares."""
    return {token.casefold() for token in tokens(" ".join(line.text for line in lines))
            if len(token) >= 4 and any(character.isalpha() for character in token)}


def contains_label(text: str, label: str) -> bool:
    """Whether the case-folded `text` holds `label` as whole words (`Ler` is not in `smaller`)."""
    return re.search(rf"(?<!\w){re.escape(folded(label))}(?!\w)", text) is not None


def score_real(item: str, readings_by_engine: dict[str, list[Recognition]], families: dict[str, str]) -> dict:
    """Engines on one real recording: interface labels found, corroborated words, confidence, time."""
    lexicon = LEXICON.get(item, [])
    texts = {engine: folded(" \n ".join(" ".join(line.text for line in reading.lines) for reading in readings))
             for engine, readings in readings_by_engine.items()}
    labels = []
    for spellings in lexicon:
        found_by = sorted(engine for engine, text in texts.items()
                          if any(contains_label(text, spelling) for spelling in spellings))
        labels.append({"label": spellings[0], "found_by": found_by})
    reachable = [label for label in labels if label["found_by"]]
    engines = {}
    for engine, readings in readings_by_engine.items():
        others = {other: read for other, read in readings_by_engine.items() if families[other] != families[engine]}
        own_words = 0
        corroborated = 0
        for index, reading in enumerate(readings):
            words = word_set(reading.lines)
            elsewhere = set()
            for read in others.values():
                if index < len(read) and read[index].file == reading.file:
                    elsewhere |= word_set(read[index].lines)
            own_words += len(words)
            corroborated += len(words & elsewhere)
        confidences = [line.confidence for reading in readings for line in reading.lines if line.confidence is not None]
        found = sum(engine in label["found_by"] for label in labels)
        engines[engine] = {
            "labels_found": found,
            "label_recall": round(found / len(labels), 4) if labels else None,
            "label_recall_of_reachable": round(found / len(reachable), 4) if reachable else None,
            "lines_per_frame": round(statistics.fmean(len(reading.lines) for reading in readings), 2) if readings else None,
            "words": own_words,
            "corroborated_share": round(corroborated / own_words, 4) if own_words else None,
            "median_line_confidence": round(statistics.median(confidences), 4) if confidences else None,
            "seconds_per_frame": timing(readings),
        }
    return {"labels": len(labels), "reachable_labels": len(reachable), "per_label": labels, "engines": engines}


# --- the command line ---------------------------------------------------------------------------

def sha256_of(path: Path) -> str:
    """The file's SHA-256, streamed."""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(1 << 20):
            digest.update(chunk)
    return digest.hexdigest()


def run_checked(command: list[str], cwd: Path) -> None:
    """Run a tool with explicit arguments (never a shell) in `cwd`; a failure stops the step."""
    completed = subprocess.run(command, capture_output=True, text=True, timeout=1800, check=False, cwd=cwd)
    if completed.returncode != 0:
        raise EvaluationError(f"{Path(command[0]).name} exited {completed.returncode}: {completed.stderr[-400:]}")


def build_engines(arguments: argparse.Namespace, scratch: Path) -> list[Engine]:
    """The configurations named on the command line, in a fixed order."""
    engines: list[Engine] = []
    for name in arguments.engine:
        if name.startswith("tesseract-"):
            variant, *options = name.removeprefix("tesseract-").split("-")
            if variant not in TESSDATA or not set(options) <= {"x2", "sparse"}:
                raise EvaluationError(f"unknown engine configuration {name}")
            engines.append(TesseractEngine(name, Path(arguments.tesseract), Path(arguments.tessdata) / variant,
                                           Path(arguments.ffmpeg), scratch / name, 2 if "x2" in options else 1,
                                           "11" if "sparse" in options else "3"))
        elif name == "ocrs":
            engines.append(OcrsEngine(name, Path(arguments.ocrs), scratch / name))
        elif name == "ppocr-v5-mobile-latin":
            engines.append(RapidOcrEngine(name, "PP-OCRv5", "mobile", "multi", "mobile", "latin"))
        elif name == "ppocr-v5-mobile-latin-1t":
            engines.append(RapidOcrEngine(name, "PP-OCRv5", "mobile", "multi", "mobile", "latin", threads=1))
        elif name == "ppocr-v6-small":
            # PP-OCRv6 has one multilingual model per size; RapidOCR routes it by a language it serves.
            engines.append(RapidOcrEngine(name, "PP-OCRv6", "small", "en", "small", "en"))
        else:
            raise EvaluationError(f"unknown engine configuration {name}")
    return engines


def command_fetch_tessdata(arguments: argparse.Namespace) -> int:
    fetched = fetch_tessdata(Path(arguments.out), arguments.variant, bounded_https_get)
    for entry in fetched:
        print(f"{entry['variant']}/{entry['language']}.traineddata {entry['bytes']} bytes sha256 {entry['sha256']}")
    return 0


def command_build_tesseract(arguments: argparse.Namespace) -> int:
    work = Path(arguments.work)
    source_root = work / "src"
    build_root = work / "build"
    source_root.mkdir(parents=True, exist_ok=False)
    build_root.mkdir(parents=True, exist_ok=False)
    build_tesseract(source_root, build_root, Path(arguments.prefix), run_checked, git_head)
    return 0


def command_stage_ffmpeg(arguments: argparse.Namespace) -> int:
    """The pinned FFmpeg and FFprobe of the P06 Ubuntu candidate, as VSift's media provider."""
    import p07_local_asr_tools as staging  # noqa: PLC0415 - hosted only

    work = Path(arguments.out)
    work.mkdir(mode=0o700, parents=True, exist_ok=False)
    ffmpeg_bin, _whisper = staging.stage_linux(work)
    with open(os.environ["GITHUB_ENV"], "a", encoding="utf-8") as env_file:
        env_file.write(f"VSIFT_EVAL_FFMPEG={ffmpeg_bin / 'ffmpeg'}\n")
        env_file.write(f"VSIFT_EVAL_FFPROBE={ffmpeg_bin / 'ffprobe'}\n")
    return 0


def command_synthesize(arguments: argparse.Namespace) -> int:
    truth = synthesize(Path(arguments.out), Path(arguments.ffmpeg), [Path(font) for font in arguments.font],
                       run_checked)
    write_control(Path(arguments.out), Path(arguments.control))
    print(f"{len(truth['screens'])} screens, {len(truth['videos'])} videos")
    return 0


def command_extract(arguments: argparse.Namespace) -> int:
    work = Path(arguments.work)
    vsift = Vsift(Path(arguments.vsift), work / "root", work / "user")
    vsift.user.mkdir(parents=True, exist_ok=True)
    if not (work / "configured").exists():
        vsift("setup configure ffmpeg", ["setup", "configure", "ffmpeg", "--executable", arguments.ffmpeg])
        vsift("setup configure ffprobe", ["setup", "configure", "ffprobe", "--executable", arguments.ffprobe])
        (work / "configured").write_text("yes\n", encoding="utf-8")
    record = extract(vsift, Path(arguments.video), int(arguments.duration_us), Path(arguments.out),
                     [int(value) for value in arguments.at_us])
    print(f"{record['candidates']} candidates, {len(record['frames'])} frames kept, {len(record['gaps'])} gaps")
    return 0


def command_recognise(arguments: argparse.Namespace) -> int:
    scratch = Path(arguments.scratch)
    engines = build_engines(arguments, scratch)
    described = []
    for engine in engines:
        for video in arguments.frames:
            frames_dir = Path(video)
            language = arguments.language.get(frames_dir.name, "eng")
            out = frames_dir / f"ocr-{engine.name}.jsonl"
            readings = recognise_all(engine, frames_dir, language, out)
            print(f"{engine.name} on {frames_dir.name}: {len(readings)} frames, {timing(readings)}")
        described.append(engine.describe())
    Path(arguments.describe).write_text(json.dumps(described, indent=1) + "\n", encoding="utf-8")
    return 0


def parse_languages(values: list[str]) -> dict[str, str]:
    """`NAME=lang` pairs: the frames folder and its language (`eng` or `por`)."""
    languages = {}
    for value in values:
        name, _, language = value.partition("=")
        if language not in {"eng", "por"} or not name:
            raise EvaluationError("a language is NAME=eng or NAME=por")
        languages[name] = language
    return languages


def command_score(arguments: argparse.Namespace) -> int:
    work = Path(arguments.work)
    truth = json.loads((Path(arguments.synthetic) / "truth.json").read_text(encoding="utf-8"))
    engines = json.loads(Path(arguments.describe).read_text(encoding="utf-8"))
    families = {engine["name"]: engine["family"] for engine in engines}
    report: dict = {"engines": engines, "synthetic": {}, "real": {}}
    for name in sorted(path.name for path in work.iterdir() if path.is_dir() and (path / "frames.json").exists()):
        folder = work / name
        frames = json.loads((folder / "frames.json").read_text(encoding="utf-8"))
        readings = {engine["name"]: read_recognitions(folder / f"ocr-{engine['name']}.jsonl") for engine in engines
                    if (folder / f"ocr-{engine['name']}.jsonl").exists()}
        summary = {"candidates": frames["candidates"], "candidates_without_periodic": frames.get("candidates_without_periodic"),
                   "frames": len(frames["frames"]), "gaps": frames["gaps"]}
        if name.startswith("synthetic-"):
            summary["engines"] = {engine: score_synthetic(truth, frames, read) for engine, read in readings.items()}
            report["synthetic"][name] = summary
        else:
            summary.update(score_real(name, readings, families))
            report["real"][name] = summary
    Path(arguments.report).write_text(json.dumps(report, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    Path(arguments.summary).write_text(markdown(report), encoding="utf-8")
    return 0


def markdown(report: dict) -> str:
    """The job summary: one table per synthetic video and one per real recording."""
    rows = ["# P16 OCR evaluation", ""]
    for name, video in report["synthetic"].items():
        reached = sorted({screen for scored in video["engines"].values()
                          for screen in scored.get("screens_reached_by_candidates", [])})
        rows += [f"## {name} ({video['frames']} frames; {video['candidates']} candidates, "
                 f"{video.get('candidates_without_periodic')} without the periodic ones; "
                 f"candidates reached {len(reached)} screens)", "",
                 "| Engine | Screens | Exact lines | Median CER | Numbers read | Numbers misread | "
                 + " | ".join(f"{size} px exact" for size in FONT_SIZES) + " | s/frame | CPU s/frame |",
                 "| --- | ---: | ---: | ---: | ---: | ---: | " + " | ".join("---:" for _ in FONT_SIZES) + " | ---: | ---: |"]
        for engine, scored in video["engines"].items():
            overall = scored["overall"]
            per_size = " | ".join(str(scored["by_size"][str(size)].get("exact_share", "-")) for size in FONT_SIZES)
            rows.append(f"| {engine} | {scored['screens_scored']}/{scored['screens']} | {overall.get('exact_share')} | "
                        f"{overall.get('median_cer')} | {overall.get('numbers_found_share')} | "
                        f"{overall.get('numbers_misread')} | {per_size} | {scored['seconds_per_frame'].get('median')} | "
                        f"{scored['seconds_per_frame'].get('cpu_median')} |")
        rows.append("")
    for name, item in report["real"].items():
        rows += [f"## {name} ({item['frames']} frames of {item['candidates']} candidates; "
                 f"{item['reachable_labels']} of {item['labels']} labels found by some engine)", "",
                 "| Engine | Labels found | Of reachable | Corroborated words | Median confidence | Lines/frame | s/frame | CPU s/frame |",
                 "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
        for engine, scored in item["engines"].items():
            rows.append(f"| {engine} | {scored['labels_found']} | {scored['label_recall_of_reachable']} | "
                        f"{scored['corroborated_share']} | {scored['median_line_confidence']} | "
                        f"{scored['lines_per_frame']} | {scored['seconds_per_frame'].get('median')} | "
                        f"{scored['seconds_per_frame'].get('cpu_median')} |")
        rows.append("")
    return "\n".join(rows) + "\n"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    tessdata = commands.add_parser("fetch-tessdata")
    tessdata.add_argument("--out", required=True)
    tessdata.add_argument("--variant", action="append", choices=sorted(TESSDATA), required=True)
    tessdata.set_defaults(handler=command_fetch_tessdata)
    building = commands.add_parser("build-tesseract")
    building.add_argument("--work", required=True)
    building.add_argument("--prefix", required=True)
    building.set_defaults(handler=command_build_tesseract)
    stage = commands.add_parser("stage-ffmpeg")
    stage.add_argument("--out", required=True)
    stage.set_defaults(handler=command_stage_ffmpeg)
    synth = commands.add_parser("synthesize")
    synth.add_argument("--out", required=True)
    synth.add_argument("--ffmpeg", required=True)
    synth.add_argument("--font", action="append", required=True)
    synth.add_argument("--control", required=True)
    synth.set_defaults(handler=command_synthesize)
    extracting = commands.add_parser("extract")
    for option in ("--vsift", "--ffmpeg", "--ffprobe", "--work", "--video", "--duration-us", "--out"):
        extracting.add_argument(option, required=True)
    extracting.add_argument("--at-us", action="append", default=[])
    extracting.set_defaults(handler=command_extract)
    recognising = commands.add_parser("recognise")
    recognising.add_argument("--engine", action="append", required=True)
    recognising.add_argument("--frames", action="append", required=True)
    recognising.add_argument("--language", action="append", default=[])
    recognising.add_argument("--tesseract")
    recognising.add_argument("--tessdata")
    recognising.add_argument("--ocrs")
    recognising.add_argument("--ffmpeg")
    recognising.add_argument("--scratch", required=True)
    recognising.add_argument("--describe", required=True)
    recognising.set_defaults(handler=command_recognise)
    scoring = commands.add_parser("score")
    for option in ("--work", "--synthetic", "--describe", "--report", "--summary"):
        scoring.add_argument(option, required=True)
    scoring.set_defaults(handler=command_score)
    arguments = parser.parse_args(argv)
    try:
        if arguments.command == "recognise":
            arguments.language = parse_languages(arguments.language)
        return arguments.handler(arguments)
    except EvaluationError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
