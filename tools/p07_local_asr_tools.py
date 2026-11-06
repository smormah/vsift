"""Stage the pinned local-ASR toolchain for the opt-in P07 local-ASR workflow.

Only a disposable GitHub Actions runner may run this. It downloads, over HTTPS
with bounded sizes, the reviewed FFmpeg/FFprobe build and the reviewed
whisper.cpp v1.9.5 CPU build for the runner's platform (the P06 candidate
pins in p06_ubuntu_candidate_smoke.py and p06_windows_candidate_smoke.py)
and both pinned whisper.cpp models. Every download is checked against its
pinned size and SHA-256 before use, and fails closed. Only the reviewed files
are extracted, each checked again. The selected paths are then written to
GITHUB_PATH and GITHUB_ENV for the opt-in tests. Nothing is installed on the
machine and no repository credential is used.

On Windows a run may instead name the tag and archive SHA-256 of a published
VSift-built whisper.cpp (VSIFT_WHISPER_WINDOWS_TAG and
VSIFT_WHISPER_WINDOWS_ARCHIVE_SHA256, set from the workflow dispatch inputs):
the archive and its .inventory.json are then fetched from the release URL,
checked against that digest and against each other, and only the files the
inventory lists are extracted, each checked by size and SHA-256.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import platform
import re
import sys
import tarfile
import urllib.error
import urllib.parse
import urllib.request
import zipfile

import p06_ubuntu_candidate_smoke as ubuntu
import p06_windows_candidate_smoke as windows
from p06_windows_candidate_smoke import CandidateRejected, verified_download


# The two reviewed models, identical to reviewed_whisper_models() in
# crates/vsift-infrastructure/src/managed_catalogue.rs. The q5_1 revision is
# the repository's newest, where ggml-base.bin is byte-identical to the base
# pin at revision 80da2d8 (which predates the quantized files); the maintainer
# accepted that pin revision on 2026-09-25.
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


# The engine VSift builds itself for Windows (ADR 0032 decision D2). When a run
# names its release tag and archive SHA-256, the Windows leg tests those
# published bytes in place of upstream's b5454 whisper.cpp: the archive and its
# inventory are fetched from the release URL, and nothing is trusted that the
# inventory and the given digest do not agree on. Unset, the leg is unchanged.
VSIFT_RELEASES = "https://github.com/smormah/vsift/releases/download"
ENGINE_TAG = re.compile(r"^whisper-cli-windows-v([0-9]+\.[0-9]+\.[0-9]+-r[0-9]+)$")
SHA256_HEX = re.compile(r"^[0-9a-f]{64}$")
MAX_INVENTORY_BYTES = 1024 * 1024
MAX_ENGINE_ARCHIVE_BYTES = 64 * 1024 * 1024
MAX_ENGINE_MEMBERS = 64
ENGINE_EXECUTABLE = "whisper-cli.exe"


def engine_stem(tag: str) -> str:
    """The archive stem `<engine>-<version>-<target>` a Windows engine tag names."""
    match = ENGINE_TAG.fullmatch(tag)
    if match is None:
        raise CandidateRejected("not a whisper-cli-windows engine tag")
    return f"whisper-cli-windows-{match.group(1)}-windows-x64"


def read_inventory(path: Path, stem: str, archive_sha256: str) -> dict:
    """Parse the published inventory and require it to agree with the given digest."""
    try:
        inventory = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise CandidateRejected("the inventory is not readable JSON") from error
    if not isinstance(inventory, dict) or inventory.get("archive") != f"{stem}.tar.gz":
        raise CandidateRejected("the inventory names another archive")
    if (inventory.get("sha256") != archive_sha256 or inventory.get("format") != "tar_gz"
            or inventory.get("root") != stem):
        raise CandidateRejected("the inventory disagrees with the given archive digest")
    size = inventory.get("bytes")
    if not isinstance(size, int) or not 0 < size <= MAX_ENGINE_ARCHIVE_BYTES:
        raise CandidateRejected("the inventory size is outside its bound")
    return inventory


def inventory_files(inventory: dict, stem: str) -> dict[str, tuple[int, str]]:
    """The inventory's files as `{path under the root: (bytes, SHA-256)}`."""
    files = {}
    prefix = f"{stem}/"
    for entry in inventory.get("files", []):
        path, size, digest = entry.get("path"), entry.get("bytes"), entry.get("sha256")
        if not (isinstance(path, str) and path.startswith(prefix) and isinstance(size, int)
                and isinstance(digest, str) and SHA256_HEX.fullmatch(digest)):
            raise CandidateRejected("an inventory entry is malformed")
        files[path.removeprefix(prefix)] = (size, digest)
    if ENGINE_EXECUTABLE not in files:
        raise CandidateRejected("the inventory lists no whisper-cli.exe")
    return files


def extract_engine(archive_path: Path, destination: Path, stem: str,
                   expected: dict[str, tuple[int, str]]) -> None:
    """Copy exactly the inventory's regular files, each checked against its digest."""
    destination.mkdir(mode=0o700)
    seen: set[str] = set()
    copied: set[str] = set()
    with tarfile.open(archive_path, mode="r:gz") as archive:
        for member in archive:
            path = str(windows.validate_archive_path(member.name)).rstrip("/")
            if len(seen) >= MAX_ENGINE_MEMBERS:
                raise CandidateRejected("archive entry limit exceeded")
            if path in seen:
                raise CandidateRejected("duplicate archive entry")
            seen.add(path)
            if member.isdir():
                continue
            if not member.isreg():
                raise CandidateRejected("archive contains a link or special file")
            relative = path.removeprefix(f"{stem}/")
            if relative == path or relative not in expected:
                raise CandidateRejected("archive holds a file the inventory does not list")
            size, digest = expected[relative]
            if member.size != size:
                raise CandidateRejected("archive file size differs from the inventory")
            output = destination / relative
            output.parent.mkdir(parents=True, exist_ok=True)
            hasher = hashlib.sha256()
            source = archive.extractfile(member)
            if source is None:
                raise CandidateRejected("archive file cannot be read")
            with source, output.open("xb") as target:
                while chunk := source.read(1024 * 1024):
                    hasher.update(chunk)
                    target.write(chunk)
            if hasher.hexdigest() != digest:
                raise CandidateRejected("archive file digest differs from the inventory")
            copied.add(relative)
    if copied != set(expected):
        raise CandidateRejected("archive is missing a file the inventory lists")


def fetch_bounded(url: str, destination: Path, limit: int) -> None:
    """Download a small file over HTTPS, refusing anything larger than `limit`."""
    opener = urllib.request.build_opener(windows.HttpsRedirectsOnly())
    request = urllib.request.Request(url, headers={"User-Agent": "vsift-p07-local-asr/1"})
    received = 0
    try:
        with opener.open(request, timeout=30) as response, destination.open("xb") as output:
            final = urllib.parse.urlparse(response.geturl())
            if final.scheme != "https" or not final.hostname:
                raise CandidateRejected("download ended outside HTTPS")
            while chunk := response.read(64 * 1024):
                received += len(chunk)
                if received > limit:
                    raise CandidateRejected("download exceeds its bound")
                output.write(chunk)
    except urllib.error.URLError as error:
        raise CandidateRejected("HTTPS transfer failed") from error


def stage_published_windows_whisper(work: Path, tag: str, archive_sha256: str) -> Path:
    """Fetch and verify VSift's published Windows whisper.cpp; return `whisper-cli.exe`."""
    if not SHA256_HEX.fullmatch(archive_sha256):
        raise CandidateRejected("the archive digest is not 64 lowercase hex digits")
    stem = engine_stem(tag)
    base = f"{VSIFT_RELEASES}/{tag}/{stem}"
    inventory_path = work / "engine.inventory.json"
    # The inventory's size is not known in advance, so it is read under a bound;
    # it is trusted only as far as it agrees with the digest the run was given.
    fetch_bounded(f"{base}.inventory.json", inventory_path, MAX_INVENTORY_BYTES)
    inventory = read_inventory(inventory_path, stem, archive_sha256)
    archive_path = work / "engine.tar.gz"
    verified_download(f"{base}.tar.gz", archive_path, inventory["bytes"], archive_sha256)
    print("whisper-cli-windows: archive size and SHA-256 verified against the inventory")
    files = inventory_files(inventory, stem)
    folder = work / "whisper"
    extract_engine(archive_path, folder, stem, files)
    archive_path.unlink()
    size, digest = files[ENGINE_EXECUTABLE]
    print(f"whisper-cli-windows: {ENGINE_EXECUTABLE} {size} bytes, SHA-256 {digest}")
    return folder / ENGINE_EXECUTABLE


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
    tag = os.environ.get("VSIFT_WHISPER_WINDOWS_TAG", "")
    for component, (url, size, digest) in windows.ARCHIVES.items():
        if component == "whisper" and tag:
            continue
        archives[component] = work / f"{component}.zip"
        verified_download(url, archives[component], size, digest)
        print(f"{component}: archive size and SHA-256 verified")
    ffmpeg_dir = work / "ffmpeg"
    whisper_dir = work / "whisper"
    windows.verified_extract(archives["ffmpeg"], ffmpeg_dir, "single-root", windows.FFMPEG_FILES)
    if tag:
        whisper = stage_published_windows_whisper(
            work, tag, os.environ.get("VSIFT_WHISPER_WINDOWS_ARCHIVE_SHA256", ""))
    else:
        windows.verified_extract(archives["whisper"], whisper_dir, "none", windows.WHISPER_FILES)
        whisper = whisper_dir / "whisper-cli.exe"
    for archive in archives.values():
        archive.unlink(missing_ok=True)
    return ffmpeg_dir / "bin", whisper


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
