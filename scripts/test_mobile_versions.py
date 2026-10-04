from __future__ import annotations

import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class MobileReactVersionsTest(unittest.TestCase):
    def test_templates_match_published_expo_bundled_versions(self):
        published = json.loads((ROOT / "scripts/fixtures/expo-react-versions.json").read_text())
        for directory in ("mobile", "__auth__/mobile"):
            with self.subTest(template=directory):
                base = ROOT / "templates/mobile" / directory
                source = (base / "package.json").read_text()
                package = json.loads(source.replace("{{ context.app_name }}", "fixture").replace("{{ context.baukit_mobile_typescript_dependencies }}", '"@baukit/fixture": "0.7.0"'))
                self.assertEqual(package["dependencies"]["expo"], published["expo"])
                for name in ("react", "react-native"):
                    self.assertEqual(package["dependencies"][name], published["bundledNativeModules"][name])
                self.assertNotIn("react", package["expo"]["install"]["exclude"])
                override = re.search(r"^  react-dom: (.+)$", (base / "pnpm-workspace.yaml").read_text(), re.MULTILINE)
                self.assertIsNotNone(override)
                self.assertEqual(override[1], published["bundledNativeModules"]["react-dom"])
        matrix = (ROOT / "docs/platform/compatibility-matrix.md").read_text()
        self.assertIn(f"{published['expo']} (RN {published['bundledNativeModules']['react-native']}, React {published['bundledNativeModules']['react']})", matrix)
