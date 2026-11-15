from __future__ import annotations

import contextlib
import io
import os
import tempfile
import unittest
from pathlib import Path

import check_retention_size as crs

BASE = crs.PLATFORM / "observability-stack"


def release(pvc: str | None, size: str | None) -> dict:
    spec: dict = {}
    if pvc is not None:
        spec["storageSpec"] = {
            "volumeClaimTemplate": {"spec": {"resources": {"requests": {"storage": pvc}}}}
        }
    if size is not None:
        spec["retentionSize"] = size
    return {"spec": {"values": {"prometheus": {"prometheusSpec": spec}}}}


def write_overlay(directory: Path, prometheus_spec: str) -> Path:
    overlay = directory / "overlay"
    overlay.mkdir()
    (overlay / "kustomization.yaml").write_text(
        "apiVersion: kustomize.config.k8s.io/v1beta1\nkind: Kustomization\nresources:\n"
        f"  - {os.path.relpath(BASE, overlay)}\npatches:\n  - path: patch.yaml\n"
    )
    (overlay / "patch.yaml").write_text(
        "apiVersion: helm.toolkit.fluxcd.io/v2\nkind: HelmRelease\nmetadata:\n"
        "  name: kube-prometheus-stack\n  namespace: observability\nspec:\n  values:\n"
        "    prometheus:\n      prometheusSpec:\n" + prometheus_spec
    )
    return overlay


SMALL_PVC = (
    "        storageSpec:\n          volumeClaimTemplate:\n            spec:\n"
    "              resources:\n                requests:\n                  storage: 2Gi\n"
)


class UnitTest(unittest.TestCase):
    def test_sizes_use_base_two_units(self) -> None:
        self.assertEqual(crs.prometheus_bytes("25GB"), 25 * 2**30)
        self.assertEqual(crs.prometheus_bytes("1740MiB"), 1740 * 2**20)
        self.assertEqual(crs.kube_bytes("30Gi"), 30 * 2**30)
        self.assertEqual(crs.kube_bytes("2G"), 2 * 10**9)

    def test_size_at_85_percent_passes(self) -> None:
        self.assertEqual(crs.release_problems("x", release("2Gi", "1740MB")), [])

    def test_size_above_90_percent_fails(self) -> None:
        [problem] = crs.release_problems("x", release("2Gi", "1900MB"))
        self.assertIn("above 90%", problem)

    def test_missing_size_fails(self) -> None:
        [problem] = crs.release_problems("x", release("2Gi", None))
        self.assertIn("no retentionSize", problem)

    def test_missing_pvc_fails(self) -> None:
        [problem] = crs.release_problems("x", release(None, "1GB"))
        self.assertIn("no Prometheus PVC size", problem)

    def test_unparseable_size_fails(self) -> None:
        [problem] = crs.release_problems("x", release("2Gi", "2Gi"))
        self.assertIn("cannot parse retentionSize", problem)


class RenderTest(unittest.TestCase):
    def run_check(self, *directories: Path) -> tuple[int, str]:
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(io.StringIO()):
            code = crs.main([str(directory) for directory in directories])
        return code, stderr.getvalue()

    def test_committed_bases_and_overlays_pass(self) -> None:
        self.assertEqual(self.run_check(), (0, ""))

    def test_overlay_that_shrinks_the_pvc_alone_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            overlay = write_overlay(Path(directory), SMALL_PVC)
            code, stderr = self.run_check(overlay)
        self.assertEqual(code, 1)
        self.assertIn("retentionSize 25GB is 1250% of the 2Gi PVC", stderr)

    def test_overlay_that_shrinks_both_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            overlay = write_overlay(Path(directory), "        retentionSize: 1740MB\n" + SMALL_PVC)
            self.assertEqual(self.run_check(overlay), (0, ""))


if __name__ == "__main__":
    unittest.main()
