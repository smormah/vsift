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
    "v1.9.2/whisper-bin-x64.zip"
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
        8_194_445,
        "49dcc16de826f20bd53d44f947a1ae49dfa81f86cad67a64d80820cb192d674a",
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
    "whisper-cli.exe": (479_232, "95e3c0b0e778ad9499eb0125f97c1dcf437dd9eb4ea77050b043574f93c2631d"),
    "whisper.dll": (1_368_064, "792fc523c7ad16e6b9c348e30ad5e5f591165cbcf6a80ca8d0db02a38ce3eea2"),
    "ggml.dll": (67_584, "894c6237ee7849843213906a2b6a0b371aaa6234048d465f206d910ae846fafb"),
    "ggml-base.dll": (666_624, "1482359d921b4c1b183d49db1d770f9b5e90d86a618b8b648d4845c2471ad6b0"),
    "ggml-cpu-alderlake.dll": (809_984, "d1c5411561361f7ce71ff8455ecf01f666f581b0608fa91a1dfe7d3fd6a25bd1"),
    "ggml-cpu-cannonlake.dll": (853_504, "2ef36f05fa252ff4fdcb8d42ebce1ceba4f3d3de12b93bed15bdee6237dccd63"),
    "ggml-cpu-cascadelake.dll": (850_944, "505899aaf3f99c5d714361640f561458ea97f8a09eb0614568a66bead2115cb0"),
    "ggml-cpu-haswell.dll": (811_008, "f8cf2f35a06498d783d77fde42004dd54d2f8236b0d42ac323b94bba65a603c4"),
    "ggml-cpu-icelake.dll": (850_944, "78ad143ee2e674d037b4840ef33b5748a0659762a26e0ae2b621c4f9451cbde8"),
    "ggml-cpu-sandybridge.dll": (803_328, "ee47db7dc40fb30eca73e62a05306059c2c3c42aecddf2e8d6ad7e530069b815"),
    "ggml-cpu-skylakex.dll": (852_992, "164e2793897944a43ee071ce6c0b09018088bdf4dd8b14ac0755c58849cf8c50"),
    "ggml-cpu-sse42.dll": (790_016, "7318a9a3b95a85b2453c437b274412bbbae89e5ecdf5babb19b99edc06ded063"),
    "ggml-cpu-x64.dll": (794_112, "af0f1c2f28ff9e3f472481dd969907bda85fa39d4fde17617d4bb0b389301b60"),
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
