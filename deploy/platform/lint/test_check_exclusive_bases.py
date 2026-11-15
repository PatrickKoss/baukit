from __future__ import annotations

import tempfile
import textwrap
import unittest
from pathlib import Path

import check_exclusive_bases as ceb


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(textwrap.dedent(text))


class ExclusiveBasesTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.platform = Path(self.tmp.name)
        for base in ("redis", "redis-ha", "kured"):
            write(self.platform / base / "kustomization.yaml", "resources:\n  - namespace.yaml\n")

    def test_overlay_with_both_bases_fails(self):
        overlay = self.platform / "overlays/both"
        write(overlay / "kustomization.yaml", "resources:\n  - ../../redis\n  - flux.yaml\n")
        write(overlay / "flux.yaml", "spec:\n  path: ./deploy/platform/redis-ha\n")
        found = ceb.violations(overlay, self.platform)
        self.assertEqual(len(found), 1)
        self.assertIn("redis and redis-ha", found[0])

    def test_overlay_with_one_base_passes(self):
        overlay = self.platform / "overlays/one"
        write(overlay / "kustomization.yaml", "resources:\n  - ../../redis-ha\n  - ../../kured\n")
        self.assertEqual(ceb.violations(overlay, self.platform), [])
        self.assertEqual(ceb.composed_bases(overlay, self.platform), {"redis-ha", "kured"})

    def test_platform_overlays_pass(self):
        overlays = [*ceb.PLATFORM.glob("overlays/*/"), ceb.PLATFORM / "e2e/overlay"]
        self.assertEqual(ceb.main([str(overlay) for overlay in overlays]), 0)
        self.assertIn("redis-ha", ceb.composed_bases(ceb.PLATFORM / "e2e/overlay"))


if __name__ == "__main__":
    unittest.main()
