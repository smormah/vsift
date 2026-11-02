"""Information phase only: which candidate pages serve a real pair of separate https streams.

This is #409's platform qualification, step 1 (ADR 0028 section 4.7, "Phase 1"). For each
public page named in CANDIDATES (space-separated, at most eight) it runs the pinned yt-dlp with
phase 1's fixed arguments, the same format selection and the same live filter, and prints what
VSift would have chosen: one file, a pair of separate streams, or the reason there is none.
Nothing is downloaded and no media is written. No cookie, token, sign-in or client trick is
used; a bot wall or a login wall is reported as it comes and never engineered round.

It also reads one video's size before a real fetch: give the link in CANDIDATES and the
selected formats' `bytes` and `total_bytes` are the size the fetch would take (null when the
site does not say). Usage: search_formats.py <path to the pinned yt-dlp> [<path to a Deno>]
Works on Windows, macOS and Linux; the second argument is the runtime some sites need.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from urllib.parse import urlsplit

# ADR 0028 section 4.7, "The format and live filter".
FORMAT_SELECTION = (
    "bv*[protocol=https][ext=mp4]+ba[protocol=https][ext=m4a]"
    "/b[protocol=https][ext=mp4]"
    "/bv*[protocol=https][ext=webm]+ba[protocol=https][ext=webm]"
    "/b[protocol=https][ext=webm]"
)
MATCH_FILTER = "!is_live & !was_live & live_status!=is_upcoming & live_status!=post_live"

# ADR 0028 section 4.7, "The fixed arguments" (the ones that matter to an information read).
FIXED_ARGUMENTS = [
    "--ignore-config",
    "--no-plugin-dirs",
    "--no-update",
    "--no-remote-components",
    "--no-cookies",
    "--no-cookies-from-browser",
    "--no-playlist",
    "--no-batch-file",
    "--max-downloads",
    "1",
    "--abort-on-error",
    "--no-exec",
    "--restrict-filenames",
    "--no-mtime",
    "--no-part",
    "--no-continue",
    "--downloader",
    "native",
    "--no-check-formats",
    "--socket-timeout",
    "30",
]

MAX_CANDIDATES = 8
PHASE_TIMEOUT_SECONDS = 120


class CandidateRefused(ValueError):
    """A candidate that is not a plain public https page link."""


def check_candidate(link: str) -> str:
    """Return the link when it is a plain https page link with no credentials or tokens."""
    parts = urlsplit(link)
    if parts.scheme != "https" or not parts.hostname or "." not in parts.hostname:
        raise CandidateRefused("only https links to a named host are searched")
    if parts.username or parts.password or parts.port or parts.fragment:
        raise CandidateRefused("a link with credentials, a port or a fragment is not searched")
    if parts.hostname.replace(".", "").isdigit():
        raise CandidateRefused("a host written as an address is not searched")
    query = [pair.split("=", 1)[0] for pair in parts.query.split("&") if pair]
    if any(name != "v" for name in query):
        raise CandidateRefused("only a plain `v` query is allowed, so no token is searched")
    return link


def is_audio_only(entry: dict) -> bool:
    return entry.get("vcodec") == "none"


def is_video_only(entry: dict) -> bool:
    return entry.get("acodec") == "none"


def describe(entry: dict) -> dict:
    return {
        "format_id": entry.get("format_id"),
        "protocol": entry.get("protocol"),
        "ext": entry.get("ext"),
        "vcodec": entry.get("vcodec"),
        "acodec": entry.get("acodec"),
        "bytes": entry.get("filesize") or entry.get("filesize_approx"),
    }


def classify(info: dict) -> dict:
    """What phase 1 selected: a pair, one file, or nothing; plus the near misses."""
    downloads = info.get("requested_downloads") or []
    first = downloads[0] if downloads else info
    selected = first.get("requested_formats") or [first]
    chosen = [describe(entry) for entry in selected]
    pair = (
        len(selected) == 2
        and any(is_video_only(entry) for entry in selected)
        and any(is_audio_only(entry) for entry in selected)
    )
    sizes = [entry["bytes"] for entry in chosen]
    return {
        "verdict": "PAIR" if pair else "SINGLE_FILE",
        "total_bytes": None if None in sizes else sum(sizes),
        "extractor_key": info.get("extractor_key"),
        "type": info.get("_type"),
        "availability": info.get("availability"),
        "duration_s": info.get("duration"),
        "license": info.get("license"),
        "selected": chosen,
    }


def available_shapes(info: dict) -> dict:
    """Every offered format, grouped by protocol and kind, so a near miss is visible."""
    shapes: dict[str, int] = {}
    for entry in info.get("formats") or []:
        if is_audio_only(entry):
            kind = "audio-only"
        elif is_video_only(entry):
            kind = "video-only"
        else:
            kind = "combined"
        key = f"{kind} {entry.get('protocol')} {entry.get('ext')}"
        shapes[key] = shapes.get(key, 0) + 1
    return shapes


def runtime_arguments(runtime: str | None) -> list[str]:
    """No runtime found on PATH is ever used: none, or exactly the one given."""
    return ["--no-js-runtimes", *(["--js-runtimes", f"deno:{runtime}"] if runtime else [])]


def run_yt_dlp(
    executable: str, arguments: list[str], home: str, runtime: str | None
) -> subprocess.CompletedProcess:
    environment = {
        "HOME": home,
        "TMPDIR": home,
        "TEMP": home,
        "TMP": home,
        "YTDLP_NO_PLUGINS": "1",
    }
    if os.name == "nt":
        environment["SystemRoot"] = os.environ.get("SystemRoot", "")
    else:
        environment["PATH"] = "/usr/bin:/bin"
    return subprocess.run(
        [executable, *FIXED_ARGUMENTS, *runtime_arguments(runtime), *arguments],
        env=environment,
        capture_output=True,
        text=True,
        timeout=PHASE_TIMEOUT_SECONDS,
        check=False,
    )


def one_line(text: str, limit: int = 300) -> str:
    return " ".join(text.split())[:limit]


def search_one(executable: str, link: str, home: str, runtime: str | None) -> dict:
    result: dict = {"link": link}
    try:
        phase_one = run_yt_dlp(
            executable,
            [
                "-f",
                FORMAT_SELECTION,
                "--match-filters",
                MATCH_FILTER,
                "--simulate",
                "--dump-single-json",
                "--",
                link,
            ],
            home,
            runtime,
        )
    except subprocess.TimeoutExpired:
        result.update(verdict="TIMEOUT")
        return result
    result["exit_code"] = phase_one.returncode
    if phase_one.returncode == 0:
        try:
            info = json.loads(phase_one.stdout)
        except json.JSONDecodeError:
            result.update(verdict="NOT_JSON")
            return result
        result.update(classify(info))
        result["offered"] = available_shapes(info)
        return result
    result["verdict"] = "REFUSED"
    result["error"] = one_line(phase_one.stderr)
    # The selection found nothing: read the page again without it, to see what is offered.
    if "format is not available" in phase_one.stderr:
        try:
            plain = run_yt_dlp(
                executable, ["--simulate", "--dump-single-json", "--", link], home, runtime
            )
        except subprocess.TimeoutExpired:
            return result
        if plain.returncode == 0:
            try:
                result["offered"] = available_shapes(json.loads(plain.stdout))
            except json.JSONDecodeError:
                pass
    return result


def main(argv: list[str]) -> int:
    if len(argv) not in (2, 3):
        print(__doc__)
        return 2
    executable = argv[1]
    runtime = argv[2] if len(argv) == 3 else None
    links = os.environ.get("CANDIDATES", "").split()
    if not links or len(links) > MAX_CANDIDATES:
        print(f"give between 1 and {MAX_CANDIDATES} candidates")
        return 2
    try:
        links = [check_candidate(link) for link in links]
    except CandidateRefused as refusal:
        print(f"refused: {refusal}")
        return 2
    results = []
    with tempfile.TemporaryDirectory(prefix="vsift-search-") as home:
        for link in links:
            results.append(search_one(executable, link, home, runtime))
    report = json.dumps(results, indent=2, sort_keys=True)
    print(report)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write("```json\n" + report + "\n```\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
