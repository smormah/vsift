"""Opt-in, candidate-only Windows binary smoke; never an installer trust anchor.

The selected hashes are repeated from the reviewed candidate record deliberately:
the script fails closed on changed network bytes and extracts only known files.
The workflow passes no repository secrets or checkout credentials to the process.
"""

from __future__ import annotations

import hashlib
import os
from pathlib import Path, PurePosixPath
import platform
import stat
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.error
import urllib.request
import zipfile


FFMPEG_URL = (
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/"
    "autobuild-2026-08-31-13-27/"
    "ffmpeg-n9.0.1-11-ge47273f4d9-win64-lgpl-9.0.zip"
)
WHISPER_URL = (
    "https://github.com/ggml-org/whisper.cpp/releases/download/"
    "b5454/whisper-bin-x64.zip"
)
MODEL_URL = (
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/"
    "80da2d8bfee42b0e836fc3a9890373e5defc00a6/ggml-base.bin"
)

ARCHIVES = {
    "ffmpeg": (
        FFMPEG_URL,
        147_007_942,
        "2484854ad6988d34560f4e6ea7a6ecb9dde0af7c229d2591815d056b04ec4f56",
    ),
    "whisper": (
        WHISPER_URL,
        8_928_640,
        "6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb",
    ),
}
MODEL_SIZE = 147_951_465
MODEL_HASH = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"
FFMPEG_FILES = {
    "LICENSE.txt": (7_651, "da7eabb7bafdf7d3ae5e9f223aa5bdc1eece45ac569dc21b3b037520b4464768"),
    "bin/ffmpeg.exe": (114_400_768, "63a0b3c76a245bc0d986853612d9ec43a2a2d1f1c7a3fa40ee459c248075b3a6"),
    "bin/ffprobe.exe": (114_198_528, "1ce64d9fdbfce857de2dd1f157c37eaa61c7501a356273dbcbe8b1674aef5879"),
}
WHISPER_FILES = {
    "whisper-cli.exe": (479_744, "331dba46d6427105d2b802cdbc7eae916ea1c5abf9b0d3b5cfe460d8db8e4366"),
    "whisper.dll": (1_370_112, "034396d75bb5720c1851674ff64a96192b58f6927b68d334ebd254ce6cad9674"),
    "ggml.dll": (62_464, "31fe18616d872af9b6134b47a6dff8072ae4ecbe5df0273f4f8dc36766c9db8e"),
    "ggml-base.dll": (693_248, "bede0d6b2d387236ecd2b5dd676d51c8984b5b6e0ae461a12174e9c01345c5f0"),
    "ggml-cpu-alderlake.dll": (1_009_664, "d8f5c56ea490394eeae8b271650f2647aecd039a542d6df9e0140726e62a4603"),
    "ggml-cpu-cannonlake.dll": (1_049_600, "74127b40abd4f595d9db6af0fa313a02e18ff42fc228bfa05281e7c788dd4507"),
    "ggml-cpu-cascadelake.dll": (1_061_376, "4b11c427b844eeac87529af9a8d94b35aad2024666beb2a35e59fe3077e4b81e"),
    "ggml-cpu-haswell.dll": (1_010_688, "1306b1014c8118d27234d8ddc0e66ced1f8c8476bf85071595fd182ea9b7263b"),
    "ggml-cpu-icelake.dll": (1_061_376, "2b4dc4c66e7d8fe2807283816e9cf3b7f7d0556f59df34069871e37eab8aed40"),
    "ggml-cpu-sandybridge.dll": (1_026_560, "04edbdebc01a1d421062a960881a4f34d520d99693999518243f332406444c6e"),
    "ggml-cpu-skylakex.dll": (1_049_600, "1774714cf93621406ff35344b54715619a83242ae3e331e44d98a3b35ce801be"),
    "ggml-cpu-sse42.dll": (862_720, "f060318772c4c2dbac312bbfff6ea700b85dbc502bb38585a7186d06d549301a"),
    "ggml-cpu-x64.dll": (866_816, "4b4b05caba40d4309b5d710ef74c59ba190516b8489f27bdb1ba11ae33c6e9d3"),
}


class CandidateRejected(Exception):
    """A candidate failed its bounded qualification preconditions."""


class HttpsRedirectsOnly(urllib.request.HTTPRedirectHandler):
    """Permit redirects only to HTTPS URLs without embedded credentials."""

    def redirect_request(self, request, fp, code, msg, headers, newurl):
        parsed = urllib.parse.urlparse(newurl)
        if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
            raise CandidateRejected("non-HTTPS or credential-bearing redirect")
        return super().redirect_request(request, fp, code, msg, headers, newurl)


def verified_download(url: str, destination: Path, expected_size: int, expected_hash: str) -> None:
    """Stream no more than the reviewed byte count, then verify SHA-256."""
    opener = urllib.request.build_opener(HttpsRedirectsOnly())
    request = urllib.request.Request(url, headers={"User-Agent": "vsift-p06-candidate-smoke/1"})
    digest = hashlib.sha256()
    received = 0
    try:
        with opener.open(request, timeout=30) as response, destination.open("xb") as output:
            final = urllib.parse.urlparse(response.geturl())
            if final.scheme != "https" or not final.hostname:
                raise CandidateRejected("download ended outside HTTPS")
            length = response.headers.get("Content-Length")
            if length is not None and int(length) != expected_size:
                raise CandidateRejected("unexpected Content-Length")
            while chunk := response.read(1024 * 1024):
                received += len(chunk)
                if received > expected_size:
                    raise CandidateRejected("download exceeds reviewed size")
                digest.update(chunk)
                output.write(chunk)
    except urllib.error.URLError as error:
        raise CandidateRejected("HTTPS transfer failed") from error
    if received != expected_size or digest.hexdigest() != expected_hash:
        raise CandidateRejected("download size or SHA-256 mismatch")


def validate_archive_path(name: str) -> PurePosixPath:
    """Reject absolute, ambiguous and Windows-special archive paths."""
    if "\\" in name or ":" in name or "\x00" in name:
        raise CandidateRejected("ambiguous archive path")
    path = PurePosixPath(name)
    if path.is_absolute() or any(part in {".", ".."} for part in name.split("/")):
        raise CandidateRejected("archive path escapes staging")
    if not path.parts:
        raise CandidateRejected("empty archive path")
    return path


def selected_members(archive: zipfile.ZipFile, prefix: str, expected: dict[str, tuple[int, str]]):
    """Validate the whole ZIP directory before returning the exact allowed members."""
    infos = archive.infolist()
    if len(infos) > 64 or sum(info.file_size for info in infos) > 1_000_000_000:
        raise CandidateRejected("archive entry or expanded-size limit exceeded")
    seen: set[str] = set()
    selected: dict[str, zipfile.ZipInfo] = {}
    for info in infos:
        path = validate_archive_path(info.filename)
        folded = str(path).rstrip("/").casefold()
        if folded in seen:
            raise CandidateRejected("duplicate archive entry")
        seen.add(folded)
        mode = (info.external_attr >> 16) & 0o170000
        if mode not in {0, stat.S_IFREG, stat.S_IFDIR}:
            raise CandidateRejected("archive contains a link or special file")
        if info.is_dir():
            continue
        relative = "/".join(path.parts[1:]) if prefix == "single-root" else str(path)
        if prefix == "single-root" and len(path.parts) < 2:
            raise CandidateRejected("unexpected archive root file")
        if relative.startswith("Release/"):
            relative = relative.removeprefix("Release/")
        if relative in expected:
            if relative in selected or info.file_size != expected[relative][0]:
                raise CandidateRejected("selected file duplicate or size mismatch")
            selected[relative] = info
    if selected.keys() != expected.keys():
        raise CandidateRejected("archive is missing a required file")
    return selected


def verified_extract(archive_path: Path, destination: Path, prefix: str,
                     expected: dict[str, tuple[int, str]]) -> None:
    """Copy only selected regular entries to a fresh private directory."""
    destination.mkdir(mode=0o700)
    with zipfile.ZipFile(archive_path) as archive:
        selected = selected_members(archive, prefix, expected)
        for relative, info in selected.items():
            output_path = destination / relative
            output_path.parent.mkdir(parents=True, exist_ok=True)
            digest = hashlib.sha256()
            copied = 0
            with archive.open(info) as source, output_path.open("xb") as output:
                while chunk := source.read(1024 * 1024):
                    copied += len(chunk)
                    if copied > expected[relative][0]:
                        raise CandidateRejected("expanded file exceeds reviewed size")
                    digest.update(chunk)
                    output.write(chunk)
            if copied != expected[relative][0] or digest.hexdigest() != expected[relative][1]:
                raise CandidateRejected("expanded file integrity mismatch")


def run_bounded(argv: list[str], cwd: Path, timeout: int, log_path: Path) -> str:
    """Run one smoke command without a shell and retain only bounded diagnostics."""
    with log_path.open("xb") as log:
        process = subprocess.Popen(argv, cwd=cwd, stdin=subprocess.DEVNULL,
                                   stdout=log, stderr=subprocess.STDOUT, shell=False)
        deadline = time.monotonic() + timeout
        while process.poll() is None:
            if time.monotonic() >= deadline or log_path.stat().st_size > 10_000_000:
                process.kill()
                process.wait(timeout=5)
                raise CandidateRejected("candidate operation exceeded time or output limit")
            time.sleep(0.1)
    excerpt = log_path.read_bytes()[:4096].decode("utf-8", errors="replace")
    if process.returncode != 0 or log_path.stat().st_size > 10_000_000:
        raise CandidateRejected(f"candidate operation failed or overproduced output: {excerpt}")
    return excerpt


def main() -> None:
    if os.name != "nt" or not os.environ.get("GITHUB_ACTIONS"):
        raise CandidateRejected("candidate binaries may run only on a disposable Windows Actions runner")
    root = Path(__file__).resolve().parents[1]
    fixture = root / "fixtures/corpus/generated/F01.mp4"
    if not fixture.is_file():
        raise CandidateRejected("synthetic F01 fixture missing")
    print(f"qualification host: {platform.platform()} {platform.machine()}")
    with tempfile.TemporaryDirectory(prefix="vsift-p06-smoke-") as temporary:
        work = Path(temporary)
        paths = {}
        for component, (url, size, digest) in ARCHIVES.items():
            path = work / f"{component}.zip"
            verified_download(url, path, size, digest)
            paths[component] = path
            print(f"{component}: archive hash verified")
        model = work / "ggml-base.bin"
        verified_download(MODEL_URL, model, MODEL_SIZE, MODEL_HASH)
        print("model: pinned LFS hash verified")
        ffmpeg_dir = work / "ffmpeg"
        whisper_dir = work / "whisper"
        verified_extract(paths["ffmpeg"], ffmpeg_dir, "single-root", FFMPEG_FILES)
        verified_extract(paths["whisper"], whisper_dir, "none", WHISPER_FILES)
        ffmpeg = ffmpeg_dir / "bin/ffmpeg.exe"
        ffprobe = ffmpeg_dir / "bin/ffprobe.exe"
        whisper = whisper_dir / "whisper-cli.exe"
        print("ffmpeg:", run_bounded([str(ffmpeg), "-version"], work, 15,
                                     work / "ffmpeg-version.log").splitlines()[0])
        print("ffprobe:", run_bounded([str(ffprobe), "-version"], work, 15,
                                      work / "ffprobe-version.log").splitlines()[0])
        print("ffprobe:", run_bounded([str(ffprobe), "-v", "error", "-show_format", str(fixture)],
                                      work, 30, work / "ffprobe.log")[:250])
        wav = work / "fixture.wav"
        run_bounded([str(ffmpeg), "-nostdin", "-v", "error", "-i", str(fixture),
                     "-vn", "-ac", "1", "-ar", "16000", "-f", "wav", str(wav)],
                    work, 60, work / "ffmpeg.log")
        if not wav.is_file() or wav.stat().st_size < 44:
            raise CandidateRejected("FFmpeg did not produce PCM input")
        print("ffmpeg: F01 audio extraction passed")
        output = work / "candidate-transcript"
        excerpt = run_bounded([str(whisper), "-m", str(model), "-f", str(wav),
                               "-otxt", "-of", str(output)],
                              work, 180, work / "whisper.log")
        if not output.with_suffix(".txt").is_file():
            raise CandidateRejected("whisper.cpp did not write a transcript artifact")
        print("whisper.cpp: model-loaded F01 inference passed; transcript accuracy not assessed")
        print("whisper.cpp diagnostics:", excerpt[:250].replace("\n", " "))


if __name__ == "__main__":
    try:
        main()
    except (CandidateRejected, OSError, ValueError, zipfile.BadZipFile) as error:
        print(f"P06 candidate smoke failed: {error}", file=sys.stderr)
        sys.exit(1)
