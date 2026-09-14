"""Opt-in Ubuntu candidate smoke, separate from the production installer.

Only a disposable GitHub Actions runner may execute the reviewed bytes. Archive
links are inspected but never extracted as links; fixed library aliases become
regular-file copies after the archive digest has been verified.
"""

from __future__ import annotations

import hashlib
import os
from pathlib import Path
import platform
import shutil
import stat
import sys
import tarfile
import tempfile
import time

from p06_windows_candidate_smoke import (
    CandidateRejected,
    MODEL_HASH,
    MODEL_SIZE,
    MODEL_URL,
    run_bounded,
    validate_archive_path,
    verified_download,
)


FFMPEG_URL = (
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/"
    "autobuild-2026-08-31-13-27/"
    "ffmpeg-n9.0.1-11-ge47273f4d9-linux64-lgpl-9.0.tar.xz"
)
WHISPER_URL = (
    "https://github.com/ggml-org/whisper.cpp/releases/download/"
    "v1.9.2/whisper-bin-ubuntu-x64.tar.gz"
)
ARCHIVES = {
    "ffmpeg": (FFMPEG_URL, 113_372_924,
               "204fc02692b11249c3e688ad18538ce2939129a1fc6abc32a6b2638a024496cf"),
    "whisper": (WHISPER_URL, 9_497_583,
                "46811a3ecf584307480a220b9ef5ff81b7b22dc41577cbc274ce3afc61f753b1"),
}
FFMPEG_SELECTED = {
    "LICENSE.txt": (7_651, "da7eabb7bafdf7d3ae5e9f223aa5bdc1eece45ac569dc21b3b037520b4464768"),
    "bin/ffmpeg": (116_038_416, "ed57193f048a65bfb0aa3c360639d7f7109ca014405201e3ea478c9ca4ea20fc"),
    "bin/ffprobe": (115_829_520, "0e3357bef1737ec02ae600e7f6e4e409966d8d0647521ca523c622be574137b7"),
}
WHISPER_SELECTED = {
    "LICENSE": (1_078, "94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d"),
    "whisper-cli": (976_312, "61fa94d25ba9a4695118883011f35e8521c158145ec73bcd8805a7c11760e6d7"),
    "libggml.so.0.18.1": (54_936, "1985fa3dc169a16715a0998da0a075b29be8f68ea2501e3c043be53be7f11857"),
    "libggml-base.so.0.18.1": (910_680, "bc41368cecccc3db8b4f52ad168b51413ee6c005a772b1d3e4f4b3bb47777553"),
    "libggml-cpu-x64.so": (878_024, "b7c084e19dc63a83acf9d6dac8d2cba089026996bf805659e10d650d5a51c216"),
    "libwhisper.so.1.9.2": (611_280, "afd9560fa2dd20a7c0f9aa682f9c4f339b2d223f2ad6fa200fc229bc3b1606d6"),
}
ALIASES = {
    "libggml.so.0": "libggml.so.0.18.1",
    "libggml.so": "libggml.so.0",
    "libggml-base.so.0": "libggml-base.so.0.18.1",
    "libggml-base.so": "libggml-base.so.0",
    "libwhisper.so.1": "libwhisper.so.1.9.2",
    "libwhisper.so": "libwhisper.so.1",
}


def safe_observed_line(value: str, limit: int) -> str:
    """Keep candidate diagnostics printable and bounded in hosted job logs."""
    return "".join(character if 32 <= ord(character) <= 126 else "?"
                   for character in value[:limit])


def extract_candidate(archive_path: Path, destination: Path,
                      expected: dict[str, tuple[int, str]],
                      aliases: dict[str, str]) -> None:
    """Validate a whole tar index and copy only reviewed regular files."""
    destination.mkdir(mode=0o700)
    with tarfile.open(archive_path, "r:*") as archive:
        members = archive.getmembers()
        if len(members) > 96 or sum(member.size for member in members) > 500_000_000:
            raise CandidateRejected("tar entry or expanded-size limit exceeded")
        seen: set[str] = set()
        selected: dict[str, tarfile.TarInfo] = {}
        found_aliases: dict[str, str] = {}
        for member in members:
            path = validate_archive_path(member.name)
            if len(path.parts) < 2:
                if member.isdir() and len(path.parts) == 1:
                    continue
                raise CandidateRejected("unexpected tar root entry")
            folded = str(path).casefold()
            if folded in seen:
                raise CandidateRejected("duplicate tar entry")
            seen.add(folded)
            relative = "/".join(path.parts[1:])
            if member.issym():
                if relative in aliases and member.linkname == aliases[relative]:
                    found_aliases[relative] = member.linkname
                elif relative.startswith("libparakeet.so") and member.linkname in {
                    "libparakeet.so.1", "libparakeet.so.1.9.2"
                }:
                    continue  # Not selected; do not create any archive link.
                else:
                    raise CandidateRejected("unreviewed archive link")
            elif member.isfile():
                if relative in expected:
                    if member.size != expected[relative][0]:
                        raise CandidateRejected("selected tar file size mismatch")
                    selected[relative] = member
            elif not member.isdir():
                raise CandidateRejected("archive contains a special file")
        if selected.keys() != expected.keys() or found_aliases != aliases:
            raise CandidateRejected("candidate inventory differs from reviewed files")
        for relative, member in selected.items():
            source = archive.extractfile(member)
            if source is None:
                raise CandidateRejected("selected tar file is unreadable")
            output = destination / relative
            output.parent.mkdir(parents=True, exist_ok=True)
            digest = hashlib.sha256()
            copied = 0
            with source, output.open("xb") as sink:
                while chunk := source.read(1024 * 1024):
                    copied += len(chunk)
                    if copied > expected[relative][0]:
                        raise CandidateRejected("selected tar file exceeded reviewed size")
                    digest.update(chunk)
                    sink.write(chunk)
            if copied != expected[relative][0] or digest.hexdigest() != expected[relative][1]:
                raise CandidateRejected("selected tar file failed integrity")
            if relative in {"bin/ffmpeg", "bin/ffprobe", "whisper-cli"}:
                output.chmod(stat.S_IRUSR | stat.S_IWUSR | stat.S_IXUSR)
    for alias, target in aliases.items():
        shutil.copyfile(destination / target, destination / alias)


def main() -> None:
    if sys.platform != "linux" or platform.machine() != "x86_64" or not os.environ.get("GITHUB_ACTIONS"):
        raise CandidateRejected("candidate binaries may run only on an x64 Linux Actions runner")
    import resource

    fixture = Path(__file__).resolve().parents[1] / "fixtures/corpus/generated/F01.mp4"
    if not fixture.is_file():
        raise CandidateRejected("synthetic F01 fixture missing")
    print(f"qualification host: {platform.platform()} {platform.machine()}")
    with tempfile.TemporaryDirectory(prefix="vsift-p06-ubuntu-smoke-") as temporary:
        work = Path(temporary)
        paths = {}
        for component, (url, size, digest) in ARCHIVES.items():
            path = work / component
            verified_download(url, path, size, digest)
            paths[component] = path
            print(f"{component}: archive hash verified")
        model = work / "ggml-base.bin"
        verified_download(MODEL_URL, model, MODEL_SIZE, MODEL_HASH)
        print("model: pinned LFS hash verified")
        ffmpeg_dir = work / "ffmpeg-runtime"
        whisper_dir = work / "whisper-runtime"
        extract_candidate(paths["ffmpeg"], ffmpeg_dir, FFMPEG_SELECTED, {})
        extract_candidate(paths["whisper"], whisper_dir, WHISPER_SELECTED, ALIASES)
        os.environ["LD_LIBRARY_PATH"] = str(whisper_dir)
        ffmpeg = ffmpeg_dir / "bin/ffmpeg"
        ffprobe = ffmpeg_dir / "bin/ffprobe"
        whisper = whisper_dir / "whisper-cli"
        version_output = run_bounded([str(ffmpeg), "-version"], work, 15,
                                     work / "ffmpeg-version.log")
        print("ffmpeg:", safe_observed_line(version_output.splitlines()[0], 250))
        configuration = next((line for line in version_output.splitlines()
                              if line.startswith("configuration:")), None)
        if configuration is None or len(configuration) > 3000:
            raise CandidateRejected("pinned FFmpeg build configuration was not captured")
        print("ffmpeg build configuration:", safe_observed_line(configuration, 3000))
        licence_output = run_bounded([str(ffmpeg), "-L"], work, 15,
                                     work / "ffmpeg-license.log")
        licence_statement = next((line for line in licence_output.splitlines()
                                  if "Lesser General Public License" in line), None)
        if licence_statement is None:
            raise CandidateRejected("pinned FFmpeg build did not report the expected LGPL statement")
        print("ffmpeg runtime licence statement:", safe_observed_line(licence_statement, 500))
        print("ffprobe:", run_bounded([str(ffprobe), "-version"], work, 15,
                                      work / "ffprobe-version.log").splitlines()[0])
        run_bounded([str(ffprobe), "-v", "error", "-show_format", str(fixture)],
                    work, 30, work / "ffprobe.log")
        wav = work / "fixture.wav"
        run_bounded([str(ffmpeg), "-nostdin", "-v", "error", "-i", str(fixture),
                     "-vn", "-ac", "1", "-ar", "16000", "-f", "wav", str(wav)],
                    work, 60, work / "ffmpeg.log")
        if not wav.is_file() or wav.stat().st_size < 44:
            raise CandidateRejected("FFmpeg did not produce PCM input")
        output = work / "candidate-transcript"
        inference_started = time.monotonic()
        run_bounded([str(whisper), "-m", str(model), "-f", str(wav),
                     "-otxt", "-of", str(output)], work, 180, work / "whisper.log")
        inference_seconds = time.monotonic() - inference_started
        largest_child_rss_kib = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
        if not output.with_suffix(".txt").is_file():
            raise CandidateRejected("whisper.cpp did not write a transcript artifact")
        print("F01 media and model-backed inference passed; speech accuracy not assessed")
        print(f"hosted F01 inference elapsed: {inference_seconds:.2f}s; "
              f"largest child peak RSS across smoke: {largest_child_rss_kib} KiB")


if __name__ == "__main__":
    try:
        main()
    except (CandidateRejected, OSError, ValueError, tarfile.TarError) as error:
        print(f"P06 Ubuntu candidate smoke failed: {error}", file=sys.stderr)
        sys.exit(1)
