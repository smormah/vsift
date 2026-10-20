"""Fetch the noisy-speech set's clean utterances (issue #150) from the test-media release.

The twelve utterance WAVs are not in the repository (maintainer decision, 2026-10-20):
they live in the VSift test-media release, and `noisy-speech-provenance.json` records,
for each one, its byte count and SHA-256 (`synthesis.utterances`) and its release asset
(`utterance_release`). Only the rebuild of the clips and the verifier's text and level
checks need them; the ASR measurement reads the committed clips.

The download is HTTPS only, from the repository's own release URL, through GitHub's
release-asset hosts only, bounded to the recorded size and checked against the recorded
SHA-256 before the file takes its name. Run it on a disposable runner, or on a machine
whose owner has agreed to the download.

    python tools/fetch_p07_noisy_utterances.py --dest <empty folder>
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
from typing import Callable, Protocol
import urllib.error
import urllib.parse
import urllib.request


ROOT = Path(__file__).resolve().parents[1]
PROVENANCE = ROOT / "fixtures" / "corpus" / "noisy-speech" / "generated" / "noisy-speech-provenance.json"
RELEASE_PREFIX = "https://github.com/smormah/vsift/releases/download/test-media-"
ALLOWED_HOSTS = frozenset({"github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com"})
ASSET_NAME = re.compile(r"^noisy-speech-[A-Z]{2}[0-9]{2}-utterance\.wav$")
PASSAGE_ID = re.compile(r"^[A-Z]{2}[0-9]{2}$")
MAX_REDIRECTS = 5
CHUNK = 1024 * 1024
USER_AGENT = "vsift-p07-noisy-speech/1"


class FetchError(Exception):
    """An utterance could not be fetched exactly as the record pins it."""


class Response(Protocol):
    def read(self, size: int) -> bytes: ...

    def geturl(self) -> str: ...

    def __enter__(self) -> "Response": ...

    def __exit__(self, *exc: object) -> None: ...


def allowed_url(url: str) -> bool:
    parsed = urllib.parse.urlparse(url)
    return (parsed.scheme == "https" and parsed.hostname in ALLOWED_HOSTS
            and not parsed.username and not parsed.password)


class ReleaseRedirects(urllib.request.HTTPRedirectHandler):
    """Follow a bounded number of redirects, only to GitHub's release-asset hosts."""

    max_redirections = MAX_REDIRECTS

    def redirect_request(self, request, fp, code, msg, headers, newurl):  # noqa: ANN001 - urllib's signature
        if not allowed_url(newurl):
            raise FetchError("the release download redirected outside GitHub's release-asset hosts")
        return super().redirect_request(request, fp, code, msg, headers, newurl)


def default_open(url: str) -> Response:
    opener = urllib.request.build_opener(ReleaseRedirects())
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    return opener.open(request, timeout=60)


def pins(record: dict) -> list[tuple[str, str, int, str]]:
    """(passage, release URL, bytes, SHA-256) of every utterance, checked for shape."""
    release = record.get("utterance_release")
    if not isinstance(release, dict):
        raise FetchError("the record names no release for the utterances")
    base = release["base_url"]
    if not base.startswith(RELEASE_PREFIX) or not base.endswith("/") or base.count("/") != 8:
        raise FetchError("the record's release URL is not one of this repository's test-media releases")
    assets = release["assets"]
    utterances = record["synthesis"]["utterances"]
    if sorted(assets) != sorted(entry["passage"] for entry in utterances):
        raise FetchError("the release assets do not cover exactly the recorded utterances")
    result = []
    for entry in utterances:
        passage, asset = entry["passage"], assets[entry["passage"]]
        if not PASSAGE_ID.match(passage) or not ASSET_NAME.match(asset) or passage not in asset:
            raise FetchError(f"{passage} has an unexpected release asset name")
        size, digest = entry["bytes"], entry["sha256"]
        if not (isinstance(size, int) and 44 < size <= 4 * 1024 * 1024 and re.fullmatch(r"[0-9a-f]{64}", digest)):
            raise FetchError(f"{passage} has no usable size and digest")
        result.append((passage, base + asset, size, digest))
    return result


def fetch_one(url: str, destination: Path, size: int, digest: str,
              opener: Callable[[str], Response] = default_open) -> None:
    """Download at most `size` bytes to a temporary name; keep it only if the digest matches."""
    if not allowed_url(url):
        raise FetchError("refusing a URL outside GitHub's release hosts")
    partial = destination.with_name(destination.name + ".partial")
    sha256 = hashlib.sha256()
    received = 0
    try:
        with opener(url) as response, partial.open("xb") as sink:
            if not allowed_url(response.geturl()):
                raise FetchError("the download ended outside GitHub's release-asset hosts")
            while chunk := response.read(CHUNK):
                received += len(chunk)
                if received > size:
                    raise FetchError(f"{destination.name} is larger than its recorded {size} bytes")
                sha256.update(chunk)
                sink.write(chunk)
        if received != size or sha256.hexdigest() != digest:
            raise FetchError(f"{destination.name} differs from its recorded size or SHA-256")
        os.replace(partial, destination)
    except urllib.error.URLError as error:
        raise FetchError(f"the HTTPS transfer of {destination.name} failed") from error
    finally:
        if partial.exists():
            partial.unlink()


def fetch(destination: Path, provenance: Path = PROVENANCE,
          opener: Callable[[str], Response] = default_open) -> list[str]:
    """Fetch every utterance into `<destination>/speech/`, which must not exist yet."""
    record = json.loads(provenance.read_text(encoding="utf-8"))
    pinned = pins(record)
    speech = destination / "speech"
    speech.mkdir(parents=True, exist_ok=False)
    written = []
    for passage, url, size, digest in pinned:
        target = speech / f"{passage}-utterance.wav"
        fetch_one(url, target, size, digest, opener)
        written.append(target.name)
    return written


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--dest", required=True, type=Path, help="folder that receives speech/<passage>-utterance.wav")
    parser.add_argument("--provenance", type=Path, default=PROVENANCE)
    args = parser.parse_args()
    written = fetch(args.dest.resolve(), args.provenance.resolve(strict=True))
    print(f"Fetched {len(written)} utterances, each checked against its recorded SHA-256")


if __name__ == "__main__":
    main()
