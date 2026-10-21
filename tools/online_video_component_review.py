"""Review the pinned optional online-video components on a disposable runner.

ADR 0028 section 4 pins two optional managed components for Ubuntu 24.04
x86-64: yt-dlp's one-file executable and the Deno runtime it runs a page's
player JavaScript with. Their pins are checked at review time here, never on a
user's machine, so that VSift needs no OpenPGP code and nothing is downloaded
to a developer machine:

- yt-dlp: the release's `SHA2-256SUMS` is fetched by its pinned size and
  SHA-256, its detached signature is verified with `gpg` against the signing
  key in yt-dlp's repository at the release tag (the key file is pinned by its
  Git blob identity and its fingerprint), and each pinned executable digest
  must appear in the signed sums under its file name.
- Deno: the release zip is fetched by its pinned size and SHA-256, the
  publisher's `.sha256sum` files are checked against the pins, and the one
  entry of the zip is inventoried (size, SHA-256, mode), which is what the
  managed catalogue's inner-file pin is copied from.
- Each executable is then run once with `--version`, in an empty environment
  whose temporary folder is a private directory of this run, and its first
  line is compared with the reviewed banner.

The result is one JSON document on standard output and a table in the job
summary.
"""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import zipfile

from p06_windows_candidate_smoke import (
    CandidateRejected,
    HttpsRedirectsOnly,
    validate_archive_path,
    verified_download,
)


YT_DLP_TAG = "2026.08.19"
YT_DLP_RELEASE = f"https://github.com/yt-dlp/yt-dlp/releases/download/{YT_DLP_TAG}"
YT_DLP_SUMS = (f"{YT_DLP_RELEASE}/SHA2-256SUMS", 1_595,
               "a63701f30755cb4d9317950d69703e9d751d490cb1b7059e1bb501a353fe7dcb")
YT_DLP_SUMS_SIGNATURE = (f"{YT_DLP_RELEASE}/SHA2-256SUMS.sig", 566,
                         "8d6f9a7c84c57222e3650c61699185878a3468308de702993ce4f20b5f9cf199")
# The signing key as committed in yt-dlp's repository at the release tag,
# pinned by its Git blob identity (what GitHub's contents API reports for
# `public.key` at the tag) so a changed key file fails the review.
YT_DLP_KEY_URL = f"https://raw.githubusercontent.com/yt-dlp/yt-dlp/{YT_DLP_TAG}/public.key"
YT_DLP_KEY_BYTES = 1_676
YT_DLP_KEY_GIT_BLOB = "b3af31e9aa2476149045e56ebe82c4a9c41cf09c"
# Every executable yt-dlp publishes for the platforms VSift names. Only the
# Linux one is managed; the others are what a user may supply on Windows and
# macOS, and their published digests are recorded for the guide.
YT_DLP_EXECUTABLES = {
    "yt-dlp_linux": (40_446_224,
                     "58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a"),
    "yt-dlp.exe": (17_840_399,
                   "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a"),
    "yt-dlp_macos": (37_146_048,
                     "0f192b7ec147ab6288885d6351d9ab67367640029b4377576ef46dd79cf7b202"),
}
YT_DLP_MANAGED = "yt-dlp_linux"
YT_DLP_BANNER = "2026.08.19"

DENO_TAG = "v2.9.7"
DENO_RELEASE = f"https://github.com/denoland/deno/releases/download/{DENO_TAG}"
DENO_ZIP = (f"{DENO_RELEASE}/deno-x86_64-unknown-linux-gnu.zip", 41_596_794,
            "c6527f24f4b16031d3ae4fa9f658d5f11534c8d84ce7dc8502420280919c3490")
DENO_ZIP_SUM = (f"{DENO_RELEASE}/deno-x86_64-unknown-linux-gnu.zip.sha256sum", 100,
                "bf6ee050dc421189520a9ef6dbf9f868cbbdb1e5eda1a28c37dad34e7962a520")
DENO_INNER_SUM = (f"{DENO_RELEASE}/deno-x86_64-unknown-linux-gnu.sha256sum", 71,
                  "d2318dc4a0f945d0955c2e820bb4e925366e043a2e1b4f14d6bb482c5f043ef3")
DENO_BANNER_PREFIX = "deno 2.9.7 "
DENO_MAX_ENTRIES = 4
DENO_MAX_EXPANDED_BYTES = 400_000_000

BANNER_DEADLINE_SECONDS = 60
MAX_BANNER_BYTES = 64 * 1024


def bounded_fetch(url: str, destination: Path, exact_size: int) -> None:
    """Fetch a file whose size is pinned but whose digest is checked otherwise."""
    opener = urllib.request.build_opener(HttpsRedirectsOnly())
    request = urllib.request.Request(url, headers={"User-Agent": "vsift-online-video-review/1"})
    received = 0
    try:
        with opener.open(request, timeout=30) as response, destination.open("xb") as output:
            if urllib.parse.urlparse(response.geturl()).scheme != "https":
                raise CandidateRejected("download ended outside HTTPS")
            while chunk := response.read(64 * 1024):
                received += len(chunk)
                if received > exact_size:
                    raise CandidateRejected("download exceeds its pinned size")
                output.write(chunk)
    except urllib.error.URLError as error:
        raise CandidateRejected("HTTPS transfer failed") from error
    if received != exact_size:
        raise CandidateRejected("download size differs from its pin")


def fetch_pinned(pin: tuple[str, int, str], destination: Path) -> None:
    """Fetch one pinned `(url, bytes, sha256)` file, checking its size and digest."""
    url, size, digest = pin
    verified_download(url, destination, size, digest)


def git_blob_identity(content: bytes) -> str:
    """The SHA-1 Git gives a file's content as a blob."""
    return hashlib.sha1(b"blob %d\0" % len(content) + content, usedforsecurity=False).hexdigest()


def parse_sums(text: str) -> dict[str, str]:
    """`<sha256>  <name>` lines of a sums file; duplicate or malformed lines are refused."""
    sums: dict[str, str] = {}
    for line in text.splitlines():
        if not line.strip():
            continue
        digest, separator, name = line.partition("  ")
        name = name.lstrip("*")
        if (not separator or len(digest) != 64
                or any(character not in "0123456789abcdef" for character in digest)
                or not name or "/" in name or name in sums):
            raise CandidateRejected("malformed sums file")
        sums[name] = digest
    return sums


def gpg_verify(key: Path, signature: Path, signed: Path, home: Path) -> str:
    """Verify `signature` over `signed` with only `key`; return the signer's fingerprint."""
    home.mkdir(mode=0o700)
    environment = {"GNUPGHOME": str(home), "PATH": os.environ.get("PATH", "/usr/bin:/bin")}
    subprocess.run(["gpg", "--batch", "--quiet", "--import", str(key)], check=True,
                   env=environment, capture_output=True, timeout=60)
    result = subprocess.run(
        ["gpg", "--batch", "--status-fd", "1", "--verify", str(signature), str(signed)],
        env=environment, capture_output=True, text=True, timeout=60, check=False)
    valid = [line.split() for line in result.stdout.splitlines()
             if line.startswith("[GNUPG:] VALIDSIG ")]
    if result.returncode != 0 or len(valid) != 1:
        raise CandidateRejected("the sums signature does not verify with yt-dlp's key")
    return valid[0][2]


def review_yt_dlp(work: Path) -> dict:
    sums_path = work / "SHA2-256SUMS"
    signature_path = work / "SHA2-256SUMS.sig"
    key_path = work / "public.key"
    fetch_pinned(YT_DLP_SUMS, sums_path)
    fetch_pinned(YT_DLP_SUMS_SIGNATURE, signature_path)
    bounded_fetch(YT_DLP_KEY_URL, key_path, YT_DLP_KEY_BYTES)
    if git_blob_identity(key_path.read_bytes()) != YT_DLP_KEY_GIT_BLOB:
        raise CandidateRejected("yt-dlp's signing key differs from the one at the release tag")
    fingerprint = gpg_verify(key_path, signature_path, sums_path, work / "gnupg")
    sums = parse_sums(sums_path.read_text(encoding="ascii"))
    for name, (_, digest) in YT_DLP_EXECUTABLES.items():
        if sums.get(name) != digest:
            raise CandidateRejected(f"the signed sums do not list the pinned {name}")
    size, digest = YT_DLP_EXECUTABLES[YT_DLP_MANAGED]
    executable = work / YT_DLP_MANAGED
    verified_download(f"{YT_DLP_RELEASE}/{YT_DLP_MANAGED}", executable, size, digest)
    executable.chmod(0o700)
    banner = run_banner(executable, work / "yt-dlp-temp")
    if banner != YT_DLP_BANNER:
        raise CandidateRejected("yt-dlp's banner differs from the reviewed version")
    return {
        "tag": YT_DLP_TAG,
        "signing_key_fingerprint": fingerprint,
        "signing_key_git_blob": YT_DLP_KEY_GIT_BLOB,
        "signed_sums": {name: sums[name] for name in YT_DLP_EXECUTABLES},
        "managed": {"file": YT_DLP_MANAGED, "bytes": size, "sha256": digest},
        "banner": banner,
    }


def inventory_one_entry_zip(archive_path: Path) -> dict:
    """The entries of a small zip with each regular file's size, SHA-256 and mode."""
    entries = []
    with zipfile.ZipFile(archive_path) as archive:
        infos = archive.infolist()
        if (len(infos) > DENO_MAX_ENTRIES
                or sum(info.file_size for info in infos) > DENO_MAX_EXPANDED_BYTES):
            raise CandidateRejected("archive entry or expanded-size limit exceeded")
        for info in infos:
            path = str(validate_archive_path(info.filename))
            mode = (info.external_attr >> 16) & 0o177777
            if info.is_dir() or (mode & 0o170000) not in {0, stat.S_IFREG}:
                entries.append({"path": path, "kind": "other", "mode": oct(mode)})
                continue
            digest = hashlib.sha256()
            read = 0
            with archive.open(info) as source:
                while chunk := source.read(1024 * 1024):
                    read += len(chunk)
                    if read > info.file_size:
                        raise CandidateRejected("archive entry is longer than its index says")
                    digest.update(chunk)
            if read != info.file_size:
                raise CandidateRejected("archive entry is shorter than its index says")
            entries.append({
                "path": path,
                "kind": "file",
                "bytes": info.file_size,
                "compressed_bytes": info.compress_size,
                "method": info.compress_type,
                "mode": oct(mode),
                "crc32": f"{info.CRC:08x}",
                "sha256": digest.hexdigest(),
            })
    return {"entries": len(entries), "members": entries}


def review_deno(work: Path) -> dict:
    archive = work / "deno.zip"
    zip_sum = work / "deno.zip.sha256sum"
    inner_sum = work / "deno.sha256sum"
    fetch_pinned(DENO_ZIP, archive)
    fetch_pinned(DENO_ZIP_SUM, zip_sum)
    fetch_pinned(DENO_INNER_SUM, inner_sum)
    published_zip = parse_sums(zip_sum.read_text(encoding="ascii"))
    if published_zip != {"deno-x86_64-unknown-linux-gnu.zip": DENO_ZIP[2]}:
        raise CandidateRejected("Deno's published zip digest differs from the pin")
    published_inner = parse_sums(inner_sum.read_text(encoding="ascii"))
    inventory = inventory_one_entry_zip(archive)
    files = [entry for entry in inventory["members"] if entry["kind"] == "file"]
    if inventory["entries"] != 1 or len(files) != 1 or files[0]["path"] != "deno":
        raise CandidateRejected("the Deno zip is not exactly one file named deno")
    if published_inner != {"deno": files[0]["sha256"]}:
        raise CandidateRejected("Deno's published inner digest differs from the zip's entry")
    executable = work / "deno"
    with zipfile.ZipFile(archive) as opened, opened.open("deno") as source, \
            executable.open("xb") as output:
        while chunk := source.read(1024 * 1024):
            output.write(chunk)
    executable.chmod(0o700)
    banner = run_banner(executable, work / "deno-temp")
    if not banner.startswith(DENO_BANNER_PREFIX):
        raise CandidateRejected("Deno's banner differs from the reviewed version")
    return {"tag": DENO_TAG, "zip": {"bytes": DENO_ZIP[1], "sha256": DENO_ZIP[2]},
            **inventory, "banner": banner}


def run_banner(executable: Path, temporary: Path) -> str:
    """First line of `<executable> --version` in an empty environment with a private TMPDIR."""
    temporary.mkdir(mode=0o700)
    environment = {"TMPDIR": str(temporary), "HOME": str(temporary), "YTDLP_NO_PLUGINS": "1",
                   "NO_COLOR": "1"}
    result = subprocess.run([str(executable), "--version"], env=environment, cwd=temporary,
                            capture_output=True, timeout=BANNER_DEADLINE_SECONDS, check=False)
    if result.returncode != 0 or len(result.stdout) > MAX_BANNER_BYTES:
        raise CandidateRejected("the executable's version banner failed")
    first = result.stdout.decode("ascii", errors="replace").splitlines()
    return first[0].strip() if first else ""


def summary_table(document: dict) -> str:
    yt_dlp = document["yt_dlp"]
    deno = document["deno"]
    lines = [
        "### Optional online-video components",
        "",
        f"yt-dlp `{yt_dlp['tag']}`: signed sums verified with key "
        f"`{yt_dlp['signing_key_fingerprint']}`; banner `{yt_dlp['banner']}`.",
        "",
        "| File | Signed SHA-256 |",
        "| --- | --- |",
    ]
    lines += [f"| `{name}` | `{digest}` |" for name, digest in yt_dlp["signed_sums"].items()]
    lines += ["", f"Deno `{deno['tag']}`: banner `{deno['banner']}`.", "",
              "| Entry | Bytes | Mode | SHA-256 |", "| --- | ---: | --- | --- |"]
    lines += [f"| `{entry['path']}` | {entry.get('bytes', '')} | {entry['mode']} | "
              f"`{entry.get('sha256', '')}` |" for entry in deno["members"]]
    return "\n".join(lines) + "\n"


def main(arguments: list[str]) -> None:
    if not os.environ.get("GITHUB_ACTIONS"):
        raise CandidateRejected("pinned components may be fetched only on an Actions runner")
    if arguments:
        raise CandidateRejected("usage: online_video_component_review.py")
    with tempfile.TemporaryDirectory(prefix="vsift-online-video-review-") as temporary:
        work = Path(temporary)
        (work / "yt-dlp").mkdir(mode=0o700)
        (work / "deno").mkdir(mode=0o700)
        document = {"yt_dlp": review_yt_dlp(work / "yt-dlp"), "deno": review_deno(work / "deno")}
    print(json.dumps(document, indent=2, sort_keys=True))
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as summary_file:
            summary_file.write(summary_table(document))


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except (CandidateRejected, OSError, ValueError, zipfile.BadZipFile,
            subprocess.SubprocessError) as error:
        print(f"Online-video component review failed: {error}", file=sys.stderr)
        sys.exit(1)
