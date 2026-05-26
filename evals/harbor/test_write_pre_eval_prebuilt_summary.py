#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import stat
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "evals/harbor/write_pre_eval_summary.py"


def load_module():
    spec = importlib.util.spec_from_file_location("write_pre_eval_summary", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def linux_x86_64_elf_header() -> bytes:
    header = bytearray(64)
    header[:4] = b"\x7fELF"
    header[4] = 2
    header[5] = 1
    header[7] = 3
    header[18:20] = (0x3E).to_bytes(2, "little")
    return bytes(header)


class PreEvalPrebuiltSummaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def executable_file(self, root: Path, contents: bytes) -> Path:
        path = root / "roder-linux-amd64"
        path.write_bytes(contents)
        path.chmod(path.stat().st_mode | stat.S_IXUSR)
        return path

    def test_required_prebuilt_blocks_non_linux_x86_64_elf(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            binary = self.executable_file(Path(temp), b"#!/bin/sh\nexit 0\n")

            summary = self.module.prebuilt_summary(binary, required=True)
            blocked = self.module.prebuilt_is_blocked(summary)

            self.assertFalse(summary["linuxX8664Elf"])
            self.assertTrue(blocked)

    def test_required_prebuilt_accepts_linux_x86_64_elf(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            binary = self.executable_file(Path(temp), linux_x86_64_elf_header())

            summary = self.module.prebuilt_summary(binary, required=True)
            blocked = self.module.prebuilt_is_blocked(summary)

            self.assertTrue(summary["linuxX8664Elf"])
            self.assertFalse(blocked)


if __name__ == "__main__":
    unittest.main()
