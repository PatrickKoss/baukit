#!/usr/bin/env python3

import re
import sys
import tomllib
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
TOOLCHAIN_INPUT = re.compile(r'^\s*toolchain:\s*"?(.+?)"?\s*(?:#.*)?$', re.MULTILINE)
MSRV_OUTPUT = "$" + "{" + "{ steps.msrv.outputs.version }" + "}"
UNPINNED_CHANNELS = {"nightly"}


def mise_rust_version() -> str:
    rust = tomllib.loads((ROOT / "mise.toml").read_text())["tools"]["rust"]
    return rust["version"] if isinstance(rust, dict) else rust


class RustToolchainPinTest(unittest.TestCase):
    def test_workflows_install_the_pinned_toolchain(self) -> None:
        allowed = {mise_rust_version(), MSRV_OUTPUT, *UNPINNED_CHANNELS}
        for path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            for toolchain in TOOLCHAIN_INPUT.findall(path.read_text()):
                with self.subTest(workflow=path.name, toolchain=toolchain):
                    self.assertIn(toolchain, allowed)


if __name__ == "__main__":
    unittest.main()
