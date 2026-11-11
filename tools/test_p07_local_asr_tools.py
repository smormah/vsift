"""Guardrails for staging the pinned local-ASR toolchain in the P07 workflow."""

from __future__ import annotations

import hashlib
import io
import json
from pathlib import Path
import os
import re
import tarfile
import tempfile
import unittest
from unittest import mock

import p07_local_asr_tools as tools
from p06_windows_candidate_smoke import CandidateRejected


CATALOGUE = Path(__file__).resolve().parents[1] / (
    "crates/vsift-infrastructure/src/managed_catalogue.rs"
)


def rust_constant(name: str) -> str:
    """Read one reviewed model literal from the Rust catalogue source."""
    source = CATALOGUE.read_text(encoding="utf-8")
    match = re.search(rf"const {name}: [^=]+= \"?([^\";]+)\"?;", source)
    if match is None:
        raise AssertionError(f"{name} missing from the catalogue")
    return match.group(1)


class PinTests(unittest.TestCase):
    def test_model_pins_equal_the_rust_catalogue(self) -> None:
        for profile, prefix in [("base", "MODEL"), ("base_q5_1", "MODEL_Q5_1")]:
            url, size, digest, name = tools.MODELS[profile]
            self.assertEqual(url, rust_constant(f"{prefix}_URL"))
            self.assertEqual(size, int(rust_constant(f"{prefix}_BYTES").replace("_", "")))
            self.assertEqual(digest, rust_constant(f"{prefix}_SHA256"))
            self.assertTrue(url.startswith("https://huggingface.co/ggerganov/whisper.cpp/resolve/"))
            self.assertTrue(url.endswith("/" + name))

    def test_models_are_distinct_and_pinned_by_revision(self) -> None:
        digests = {digest for _, _, digest, _ in tools.MODELS.values()}
        self.assertEqual(len(digests), 2)
        for url, _, _, _ in tools.MODELS.values():
            revision = url.split("/resolve/")[1].split("/")[0]
            self.assertRegex(revision, r"^[0-9a-f]{40}$")


class RunnerGuardTests(unittest.TestCase):
    def test_refuses_to_stage_outside_actions(self) -> None:
        with mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaises(CandidateRejected):
                tools.main(["unused"])

    def test_refuses_an_existing_staging_directory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary, \
                mock.patch.dict(os.environ, {"GITHUB_ACTIONS": "true"}), \
                mock.patch("platform.machine", return_value="x86_64"):
            with self.assertRaises(FileExistsError):
                tools.main([temporary])

    def test_export_requires_every_staged_file(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(CandidateRejected):
                tools.export(root / "bin", root / "whisper-cli", {"base": root / "m.bin"})


TAG = "whisper-cli-windows-v1.9.5-r1"
STEM = "whisper-cli-windows-1.9.5-r1-windows-x64"


def make_engine(directory: Path, members: list[tuple[str, bytes, bytes]]) -> tuple[Path, dict]:
    """Write an engine archive of `(name, type, content)` members and its inventory."""
    archive_path = directory / f"{STEM}.tar.gz"
    files = []
    with tarfile.open(archive_path, mode="w:gz") as archive:
        folder = tarfile.TarInfo(STEM)
        folder.type = tarfile.DIRTYPE
        archive.addfile(folder)
        for name, kind, content in members:
            info = tarfile.TarInfo(name)
            if kind == b"link":
                info.type = tarfile.SYMTYPE
                info.linkname = "elsewhere"
                archive.addfile(info)
                continue
            info.size = len(content)
            archive.addfile(info, io.BytesIO(content))
            files.append({"path": name, "bytes": len(content),
                          "sha256": hashlib.sha256(content).hexdigest()})
    data = archive_path.read_bytes()
    inventory = {"archive": archive_path.name, "bytes": len(data),
                 "sha256": hashlib.sha256(data).hexdigest(), "format": "tar_gz",
                 "root": STEM, "files": files}
    return archive_path, inventory


class PublishedWindowsWhisperTests(unittest.TestCase):
    def listed(self, inventory: dict) -> dict:
        return tools.inventory_files(inventory, STEM)

    def test_engine_tag_names_the_stem_and_refuses_anything_else(self) -> None:
        self.assertEqual(tools.engine_stem(TAG), STEM)
        for refused in ["v1.9.5", "whisper-cli-macos-v1.9.5-r1", "whisper-cli-windows-v1.9.5",
                        "whisper-cli-windows-v1.9.5-r1/../x", "whisper-cli-windows-v1.9.5-r1\n"]:
            with self.assertRaises(CandidateRejected, msg=refused):
                tools.engine_stem(refused)

    def test_extracts_exactly_the_listed_files_and_checks_each_digest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive, inventory = make_engine(root, [
                (f"{STEM}/whisper-cli.exe", b"file", b"exe-bytes"),
                (f"{STEM}/LICENSE", b"file", b"licence"),
            ])
            tools.extract_engine(archive, root / "out", STEM, self.listed(inventory))
            self.assertEqual((root / "out" / "whisper-cli.exe").read_bytes(), b"exe-bytes")

    def test_refuses_a_file_the_inventory_does_not_list(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive, inventory = make_engine(root, [
                (f"{STEM}/whisper-cli.exe", b"file", b"exe-bytes"),
                (f"{STEM}/extra.dll", b"file", b"extra"),
            ])
            listed = self.listed(inventory)
            del listed["extra.dll"]
            with self.assertRaises(CandidateRejected):
                tools.extract_engine(archive, root / "out", STEM, listed)

    def test_refuses_a_changed_file_a_link_a_traversal_and_a_missing_file(self) -> None:
        cases = {
            "changed": [(f"{STEM}/whisper-cli.exe", b"file", b"exe-bytes")],
            "link": [(f"{STEM}/whisper-cli.exe", b"file", b"exe-bytes"), (f"{STEM}/l", b"link", b"")],
            "traversal": [(f"{STEM}/../whisper-cli.exe", b"file", b"exe-bytes")],
            "missing": [(f"{STEM}/whisper-cli.exe", b"file", b"exe-bytes")],
        }
        for name, members in cases.items():
            with self.subTest(name), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                archive, inventory = make_engine(root, members)
                listed = self.listed(inventory) if name != "traversal" else {
                    "whisper-cli.exe": (9, hashlib.sha256(b"exe-bytes").hexdigest())}
                if name == "changed":
                    listed["whisper-cli.exe"] = (9, "0" * 64)
                if name == "missing":
                    listed["LICENSE"] = (7, "1" * 64)
                with self.assertRaises(CandidateRejected):
                    tools.extract_engine(archive, root / "out", STEM, listed)

    def test_inventory_must_agree_with_the_given_digest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            _, inventory = make_engine(root, [(f"{STEM}/whisper-cli.exe", b"file", b"exe-bytes")])
            path = root / "inventory.json"
            path.write_text(json.dumps(inventory), encoding="utf-8")
            self.assertEqual(tools.read_inventory(path, STEM, inventory["sha256"]), inventory)
            with self.assertRaises(CandidateRejected):
                tools.read_inventory(path, STEM, "0" * 64)
            inventory["archive"] = "another.tar.gz"
            path.write_text(json.dumps(inventory), encoding="utf-8")
            with self.assertRaises(CandidateRejected):
                tools.read_inventory(path, STEM, hashlib.sha256(b"x").hexdigest())

    def test_an_inventory_without_the_executable_is_refused(self) -> None:
        inventory = {"files": [{"path": f"{STEM}/LICENSE", "bytes": 1, "sha256": "2" * 64}]}
        with self.assertRaises(CandidateRejected):
            tools.inventory_files(inventory, STEM)

    def test_a_malformed_digest_is_refused_before_any_download(self) -> None:
        with tempfile.TemporaryDirectory() as temporary, \
                mock.patch("p07_local_asr_tools.fetch_bounded") as fetch:
            with self.assertRaises(CandidateRejected):
                tools.stage_published_windows_whisper(Path(temporary), TAG, "ABC")
            fetch.assert_not_called()


if __name__ == "__main__":
    unittest.main()
