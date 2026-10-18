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
    "b5454/whisper-bin-ubuntu-x64.tar.gz"
)
ARCHIVES = {
    "ffmpeg": (FFMPEG_URL, 113_372_924,
               "204fc02692b11249c3e688ad18538ce2939129a1fc6abc32a6b2638a024496cf"),
    "whisper": (WHISPER_URL, 10_364_195,
                "a72becf15d7917f990f6313867a52638b82b7f9ef237fb0c980dac56a135781c"),
}
FFMPEG_SELECTED = {
    "LICENSE.txt": (7_651, "da7eabb7bafdf7d3ae5e9f223aa5bdc1eece45ac569dc21b3b037520b4464768"),
    "bin/ffmpeg": (116_038_416, "ed57193f048a65bfb0aa3c360639d7f7109ca014405201e3ea478c9ca4ea20fc"),
    "bin/ffprobe": (115_829_520, "0e3357bef1737ec02ae600e7f6e4e409966d8d0647521ca523c622be574137b7"),
}
WHISPER_SELECTED = {
    "LICENSE": (1_078, "94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d"),
    "whisper-cli": (976_312, "110a858e128d1fbc5a75899b42a1de8a7a4c10856bfe45dad1adc278f6459d14"),
    "libggml.so.0.26.0": (55_184, "25406a47ddc7a6b45adb4995ad5efbc6a7ef6efbc43e64a506d978d11dd6fec8"),
    "libggml-base.so.0.26.0": (931_232, "8cd57970d5ba71529442758c0b42cb820ed319e03be0691aead74ef298d3f51f"),
    "libggml-cpu-x64.so": (942_168, "c19a80e868a34bdb197e2901c8f785a77640d6e56055134514b5fa2807171af5"),
    "libggml-cpu-alderlake.so": (1_280_320, "d33aa9c10d716534dff77655da446f09516ee4583b695e8cc1aa447e57848dac"),
    "libggml-cpu-cannonlake.so": (1_362_352, "0cb6e59a34c9798a512398e820942ee18ce99c1e0b308ec593e78655759eaafe"),
    "libggml-cpu-cascadelake.so": (1_374_344, "a0286798b10092eabaf5f81ce9ae664aa78d3c609ed2138fa8a7ca22da73ef7d"),
    "libggml-cpu-cooperlake.so": (1_374_344, "4169f4b9c975e46ddc386b695f64fb3a38a3dacd40bbb15465a2a5dbcabb9a1f"),
    "libggml-cpu-haswell.so": (1_284_416, "a63377026e388671d7925e729fced52d9dd85d1bca780dce97bc2c74817053d4"),
    "libggml-cpu-icelake.so": (1_374_344, "3f28f5a44dd3640cf179d4f678ea9bdacc536ad69b6b5fb5efd155b6b91c49c8"),
    "libggml-cpu-ivybridge.so": (1_213_104, "8b22efecdde26d41d3d864f2d5f6e29c31e0951d6453465d1f4fbda50b9eaa4b"),
    "libggml-cpu-piledriver.so": (1_209_008, "e04b72d641d4ae129c05a2ca167dae4843eb0ecae5d40a31a5d98ae480641b11"),
    "libggml-cpu-sandybridge.so": (1_217_200, "806218b034614ad31dbe4fbae48bcc0282c966bbe9f0085380d1c49b6b0f7d0b"),
    "libggml-cpu-sapphirerapids.so": (1_636_576, "d190abfe2ad900b24da89c883f633dc621f69ba12b2f5664659826374c1f4cfb"),
    "libggml-cpu-skylakex.so": (1_362_352, "66d8e2a22ef39225eb2d80e3a1dd9c8a7dd2be41966760c27f22e79623611dd5"),
    "libggml-cpu-sse42.so": (950_184, "58e6f62fa6bc2732c5a8b6323377ae62ba6cc1ebad9dfd298b6548819bd94375"),
    "libggml-cpu-zen4.so": (1_374_344, "42bb25dff93816eb12327a1b82c04ae436dd17eebf30da556143ec9acffd9582"),
    "libwhisper.so.1.9.5": (615_800, "1517f17d5188f7ef70cbb66a2b8ce258c510d20c25018d18e2801351de13cd55"),
}
ALIASES = {
    "libggml.so.0": "libggml.so.0.26.0",
    "libggml.so": "libggml.so.0",
    "libggml-base.so.0": "libggml-base.so.0.26.0",
    "libggml-base.so": "libggml-base.so.0",
    "libwhisper.so.1": "libwhisper.so.1.9.5",
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
                    "libparakeet.so.1", "libparakeet.so.1.9.5"
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
