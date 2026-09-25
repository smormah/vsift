"""Guardrails for staging the pinned local-ASR toolchain in the P07 workflow."""

from __future__ import annotations

from pathlib import Path
import os
import re
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


if __name__ == "__main__":
    unittest.main()
