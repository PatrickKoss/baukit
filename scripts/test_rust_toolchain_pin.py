from __future__ import annotations

import re
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github/workflows"
TOOLCHAIN_INPUT = re.compile(r'^\s*toolchain:\s*"?([^"\s#]+)"?', re.MULTILINE)
UNPINNED_CHANNELS = {"nightly"}


def mise_rust_version(root: Path) -> str:
    tools = tomllib.loads((root / "mise.toml").read_text())["tools"]
    rust = tools["rust"]
    return rust["version"] if isinstance(rust, dict) else rust


def msrv(root: Path) -> str:
    return tomllib.loads((root / "rust/Cargo.toml").read_text())["workspace"]["package"][
        "rust-version"
    ]


def workflow_toolchains(workflows: Path) -> dict[str, list[str]]:
    return {
        path.name: TOOLCHAIN_INPUT.findall(path.read_text())
        for path in sorted(workflows.glob("*.yml"))
    }


class RustToolchainPinTest(unittest.TestCase):
    def test_ci_uses_the_local_gate_toolchain(self) -> None:
        allowed = {mise_rust_version(ROOT), msrv(ROOT), *UNPINNED_CHANNELS}
        for workflow, toolchains in workflow_toolchains(WORKFLOWS).items():
            for toolchain in toolchains:
                with self.subTest(workflow=workflow, toolchain=toolchain):
                    self.assertIn(toolchain, allowed)


if __name__ == "__main__":
    unittest.main()
