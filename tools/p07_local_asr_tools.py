"""Stage the pinned local-ASR toolchain for the opt-in P07 local-ASR workflow.

Only a disposable GitHub Actions runner may run this. It downloads, over HTTPS
with bounded sizes, the reviewed FFmpeg/FFprobe build and the reviewed
whisper.cpp v1.9.2 CPU build for the runner's platform (the P06 candidate
pins in p06_ubuntu_candidate_smoke.py and p06_windows_candidate_smoke.py)
and both pinned whisper.cpp models. Every download is checked against its
pinned size and SHA-256 before use, and fails closed. Only the reviewed files
are extracted, each checked again. The selected paths are then written to
GITHUB_PATH and GITHUB_ENV for the opt-in tests. Nothing is installed on the
machine and no repository credential is used.
"""

from __future__ import annotations

import os
from pathlib import Path
import platform
import sys
import tarfile
import zipfile

import p06_ubuntu_candidate_smoke as ubuntu
import p06_windows_candidate_smoke as windows
from p06_windows_candidate_smoke import CandidateRejected, verified_download


# The two reviewed models, identical to reviewed_whisper_models() in
# crates/vsift-infrastructure/src/managed_catalogue.rs. The q5_1 revision is
# the repository's newest, where ggml-base.bin is byte-identical to the base
# pin at revision 80da2d8 (which predates the quantized files).
MODELS = {
    "base": (windows.MODEL_URL, windows.MODEL_SIZE, windows.MODEL_HASH, "ggml-base.bin"),
    "base_q5_1": (
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/"
        "5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base-q5_1.bin",
        59_707_625,
        "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
        "ggml-base-q5_1.bin",
    ),
}


def stage_linux(work: Path) -> tuple[Path, Path]:
    """Return the FFmpeg bin directory and whisper-cli from the Ubuntu pins."""
    archives = {}
    for component, (url, size, digest) in ubuntu.ARCHIVES.items():
        archives[component] = work / f"{component}.archive"
        verified_download(url, archives[component], size, digest)
        print(f"{component}: archive size and SHA-256 verified")
    ffmpeg_dir = work / "ffmpeg"
    whisper_dir = work / "whisper"
    ubuntu.extract_candidate(archives["ffmpeg"], ffmpeg_dir, ubuntu.FFMPEG_SELECTED, {})
    ubuntu.extract_candidate(archives["whisper"], whisper_dir, ubuntu.WHISPER_SELECTED,
                             ubuntu.ALIASES)
    for archive in archives.values():
        archive.unlink()
    return ffmpeg_dir / "bin", whisper_dir / "whisper-cli"


def stage_windows(work: Path) -> tuple[Path, Path]:
    """Return the FFmpeg bin directory and whisper-cli.exe from the Windows pins."""
    archives = {}
    for component, (url, size, digest) in windows.ARCHIVES.items():
        archives[component] = work / f"{component}.zip"
        verified_download(url, archives[component], size, digest)
        print(f"{component}: archive size and SHA-256 verified")
    ffmpeg_dir = work / "ffmpeg"
    whisper_dir = work / "whisper"
    windows.verified_extract(archives["ffmpeg"], ffmpeg_dir, "single-root", windows.FFMPEG_FILES)
    windows.verified_extract(archives["whisper"], whisper_dir, "none", windows.WHISPER_FILES)
    for archive in archives.values():
        archive.unlink()
    return ffmpeg_dir / "bin", whisper_dir / "whisper-cli.exe"


def stage_models(work: Path) -> dict[str, Path]:
    """Download both pinned models, each verified by size and SHA-256."""
    models = work / "models"
    models.mkdir(mode=0o700)
    staged = {}
    for profile, (url, size, digest, name) in MODELS.items():
        staged[profile] = models / name
        verified_download(url, staged[profile], size, digest)
        print(f"model {profile}: size and SHA-256 verified")
    return staged


def export(ffmpeg_bin: Path, whisper: Path, models: dict[str, Path]) -> None:
    """Publish the staged paths to later workflow steps."""
    for required in [ffmpeg_bin, whisper, *models.values()]:
        if not required.exists():
            raise CandidateRejected("a staged file is missing")
    with open(os.environ["GITHUB_PATH"], "a", encoding="utf-8") as path_file:
        path_file.write(f"{ffmpeg_bin}\n")
    with open(os.environ["GITHUB_ENV"], "a", encoding="utf-8") as env_file:
        env_file.write(f"VSIFT_TEST_WHISPER_CLI={whisper}\n")
        env_file.write(f"VSIFT_TEST_WHISPER_MODEL={models['base']}\n")
        env_file.write(f"VSIFT_TEST_WHISPER_MODEL_Q5_1={models['base_q5_1']}\n")


def main(arguments: list[str]) -> None:
    if not os.environ.get("GITHUB_ACTIONS") or platform.machine().lower() not in {"x86_64", "amd64"}:
        raise CandidateRejected("pinned binaries may be staged only on an x64 Actions runner")
    if len(arguments) != 1:
        raise CandidateRejected("usage: p07_local_asr_tools.py <empty staging directory>")
    work = Path(arguments[0])
    work.mkdir(mode=0o700, parents=True, exist_ok=False)
    if sys.platform == "linux":
        ffmpeg_bin, whisper = stage_linux(work)
    elif os.name == "nt":
        ffmpeg_bin, whisper = stage_windows(work)
    else:
        raise CandidateRejected("no reviewed whisper.cpp build is pinned for this platform")
    export(ffmpeg_bin, whisper, stage_models(work))
    print("pinned local-ASR toolchain staged")


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except (CandidateRejected, OSError, ValueError, KeyError, tarfile.TarError,
            zipfile.BadZipFile) as error:
        print(f"P07 local-ASR staging failed: {error}", file=sys.stderr)
        sys.exit(1)
