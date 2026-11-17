from __future__ import annotations

import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PACKAGES_DIR = ROOT / "typescript" / "packages"
PACKAGE_LIST = ROOT / "scripts" / "fixture-ts-packages.txt"
BUILD_SCRIPT = ROOT / "scripts" / "build-fixture-ts-packages.sh"
TEMPLATE_IMPORT = re.compile(r"""from ['"](@baukit/[a-z0-9-]+)""")
CLI_LITERAL = re.compile(r'"(@baukit/[a-z0-9-]+)"')
CLI_DEPENDENCY_SOURCES = ("cli/src/lib.rs", "cli/src/generate/dependencies.rs")
PREBUILD_CALLERS = (
    "Makefile",
    "scripts/generated-fixture-gate.sh",
    "scripts/mcp-remote-fixture.sh",
    "CLAUDE.md",
)


def workspace_packages() -> dict[str, dict]:
    manifests = (json.loads(path.read_text()) for path in PACKAGES_DIR.glob("*/package.json"))
    return {manifest["name"]: manifest for manifest in manifests}


def workspace_dependencies(manifest: dict, workspace: dict[str, dict]) -> set[str]:
    sections = ("dependencies", "peerDependencies")
    return {name for section in sections for name in manifest.get(section, {}) if name in workspace}


def closure(roots: set[str], workspace: dict[str, dict]) -> set[str]:
    seen: set[str] = set()
    pending = list(roots)
    while pending:
        name = pending.pop()
        if name in seen or name not in workspace:
            continue
        seen.add(name)
        pending.extend(workspace_dependencies(workspace[name], workspace))
    return seen


def fixture_packages() -> set[str]:
    found: set[str] = set()
    for path in (ROOT / "templates").rglob("*"):
        if path.is_file() and path.suffix != ".md":
            found.update(TEMPLATE_IMPORT.findall(path.read_text(errors="ignore")))
    for source in CLI_DEPENDENCY_SOURCES:
        found.update(CLI_LITERAL.findall((ROOT / source).read_text()))
    return found


def listed_packages() -> set[str]:
    lines = (line.strip() for line in PACKAGE_LIST.read_text().splitlines())
    return {line for line in lines if line and not line.startswith("#")}


def needs_build(manifest: dict) -> bool:
    return "build" in manifest.get("scripts", {})


class FixtureTypescriptPackagesTest(unittest.TestCase):
    def test_prebuild_selection_covers_fixture_packages_and_their_closure(self) -> None:
        workspace = workspace_packages()
        imported = closure(fixture_packages(), workspace)
        required = {name for name in imported if needs_build(workspace[name])}
        selected = closure(listed_packages(), workspace)
        self.assertEqual(set(), required - selected)

    def test_build_script_selects_workspace_dependencies(self) -> None:
        self.assertIn('--filter "$package..."', BUILD_SCRIPT.read_text())

    def test_fixture_prebuilds_use_the_shared_script(self) -> None:
        for caller in PREBUILD_CALLERS:
            with self.subTest(caller=caller):
                text = (ROOT / caller).read_text()
                self.assertIn("build-fixture-ts-packages.sh", text)
                self.assertNotRegex(text, r"--filter @baukit/type-guards --filter")


if __name__ == "__main__":
    unittest.main()
