from __future__ import annotations

import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class MobileReactVersionsTest(unittest.TestCase):
    def test_android_java_pins_match_the_mise_build(self):
        mise = (ROOT / "mise.toml").read_text()
        self.assertIn('java = "temurin-25.0.4+101.0.LTS"', mise)
        workflows = [
            ROOT / ".github/workflows/ci.yml",
            ROOT / "templates/common/.github/workflows/ci.yml",
            ROOT / "templates/common/.github/workflows/native.yml",
        ]
        for path, expected_count in zip(workflows, (3, 1, 1), strict=True):
            with self.subTest(workflow=path):
                pins = re.findall(r'java-version: "([^"\n]+)"', path.read_text())
                self.assertEqual(pins, ["25.0.4+101.0.LTS"] * expected_count)
        matrix = (ROOT / "docs/platform/compatibility-matrix.md").read_text()
        self.assertNotIn("Java 21", matrix)
        self.assertNotIn("Temurin 21", matrix)
        self.assertIn("Temurin 25.0.4.1+1", matrix)
        for relative in (
            "README.md",
            "docs/platform/native-quality-gates.md",
            "examples/expo-sqlite-conformance/README.md",
            "examples/expo-notifications-conformance/README.md",
        ):
            with self.subTest(document=relative):
                source = (ROOT / relative).read_text()
                self.assertNotIn("Java 21", source)
                self.assertNotIn("Temurin 21", source)
                self.assertIn("Temurin 25.0.4.1+1", source)

    def test_templates_match_published_expo_bundled_versions(self):
        published = json.loads((ROOT / "scripts/fixtures/expo-react-versions.json").read_text())
        for directory in ("mobile", "__auth__/mobile"):
            with self.subTest(template=directory):
                base = ROOT / "templates/mobile" / directory
                source = (base / "package.json").read_text()
                source = re.sub(r"\{%.*?%\}", "", source)
                package = json.loads(
                    source.replace("{{ context.app_name }}", "fixture")
                    .replace(
                        "{{ context.baukit_mobile_typescript_dependencies }}",
                        '"@baukit/fixture": "0.7.0"',
                    )
                    .replace("{{ context.baukit_mobile_typescript_dev_dependencies }}", "")
                )
                self.assertEqual(package["dependencies"]["expo"], published["expo"])
                for name in ("react", "react-native"):
                    self.assertEqual(
                        package["dependencies"][name], published["bundledNativeModules"][name]
                    )
                self.assertEqual(package["expo"]["install"]["exclude"], [])
                jest_major = package["devDependencies"]["jest"].split(".")[0]
                self.assertEqual(
                    package["devDependencies"]["@types/jest"].split(".")[0], jest_major
                )
                self.assertEqual(
                    package["devDependencies"]["@jest/globals"].split(".")[0], jest_major
                )
                for name, expected in published["bundledNativeModules"].items():
                    actual = package["dependencies"].get(name, package["devDependencies"].get(name))
                    if actual == "catalog:":
                        entry = re.search(
                            rf"^  {re.escape(name)}: (.+)$",
                            (base / "pnpm-workspace.yaml").read_text(),
                            re.MULTILINE,
                        )
                        self.assertIsNotNone(entry, name)
                        actual = entry[1]
                    self.assertIsNotNone(actual, name)
                    expected_version = published.get("resolvedWebModules", {}).get(
                        name, expected.removeprefix("~")
                    )
                    self.assertEqual(actual, expected_version, name)
                    if name in published.get("resolvedWebModules", {}):
                        bundled = tuple(map(int, expected.removeprefix("~").split(".")))
                        resolved = tuple(map(int, expected_version.split(".")))
                        self.assertEqual(resolved[:2], bundled[:2], name)
                        self.assertGreaterEqual(resolved, bundled, name)
                if directory == "__auth__/mobile":
                    for name, expected in published["authNativeModules"].items():
                        self.assertEqual(
                            package["dependencies"][name], expected.removeprefix("~"), name
                        )
                override = re.search(
                    r"^  react-dom: (.+)$", (base / "pnpm-workspace.yaml").read_text(), re.MULTILINE
                )
                self.assertIsNotNone(override)
                self.assertEqual(override[1], published["bundledNativeModules"]["react-dom"])
        matrix = (ROOT / "docs/platform/compatibility-matrix.md").read_text()
        self.assertIn(
            f"{published['expo']} (RN {published['bundledNativeModules']['react-native']}, "
            f"React {published['bundledNativeModules']['react']})",
            matrix,
        )


class ExpoOfflineChecksTest(unittest.TestCase):
    COMMANDS = (
        ("templates/common/__strict__/scripts/quality-gate.sh", "dlx expo-doctor@"),
        ("templates/common/.github/workflows/ci.yml", "expo install --check"),
        ("Makefile", "expo install --check"),
        ("scripts/generated-fixture-gate.sh", "expo install --check"),
        ("scripts/mcp-remote-fixture.sh", "expo install --check"),
    )

    def test_expo_validation_runs_offline(self):
        for path, command in self.COMMANDS:
            with self.subTest(path=path):
                lines = [line for line in (ROOT / path).read_text().splitlines() if command in line]
                self.assertTrue(lines, command)
                for line in lines:
                    self.assertIn("EXPO_OFFLINE=1 ", line)

    def test_expo_doctor_is_pinned_to_an_exact_version(self):
        source = (ROOT / "templates/common/__strict__/scripts/quality-gate.sh").read_text()
        self.assertRegex(source, r"dlx expo-doctor@\d+\.\d+\.\d+\)")
