from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import flux_applied as fa
import flux_sources as fs


def flux_kustomization(name: str, path: str, source: str = "baukit") -> dict:
    return {
        "apiVersion": fa.FLUX_KUSTOMIZATION_API,
        "kind": "Kustomization",
        "metadata": {"name": name},
        "spec": {"path": path, "sourceRef": {"kind": "GitRepository", "name": source}},
    }


class AppliedDirsTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.platform = Path(self.tmp.name)
        (self.platform / "redis").mkdir()

    def test_paths_are_resolved_and_deduplicated(self) -> None:
        objects = [
            flux_kustomization("a", "./deploy/platform/redis"),
            flux_kustomization("b", "./deploy/platform/redis"),
            {"apiVersion": "kustomize.config.k8s.io/v1beta1", "kind": "Kustomization"},
        ]
        self.assertEqual(fa.applied_dirs(objects, self.platform), [self.platform / "redis"])

    def test_missing_path_is_an_error(self) -> None:
        with self.assertRaises(fs.SourceError):
            fa.applied_dirs([flux_kustomization("a", "./deploy/platform/gone")], self.platform)


if __name__ == "__main__":
    unittest.main()
