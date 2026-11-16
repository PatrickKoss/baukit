from __future__ import annotations

import argparse
import tempfile
import unittest
from pathlib import Path

import check_admission_order as cao
import check_monitor_order as cmo
import flux_sources as fs

CONSUMER_KUSTOMIZATION = """\
apiVersion: kustomize.toolkit.fluxcd.io/v1
kind: Kustomization
metadata:
  name: {name}
spec:
  path: {path}
  sourceRef:
    kind: GitRepository
    name: {source}
{depends}"""


def kustomization(name: str, path: str, source: str, *depends_on: str) -> str:
    depends = "".join(f"    - name: {dependency}\n" for dependency in depends_on)
    return CONSUMER_KUSTOMIZATION.format(
        name=name,
        path=path,
        source=source,
        depends=f"  dependsOn:\n{depends}" if depends else "",
    )


def render(path: Path) -> str:
    return "kind: Deployment\n" if path.name == "exposure" else "kind: ConfigMap\n"


class FluxSourcesTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.platform = root / "baukit"
        self.consumer = root / "consumer"
        for directory in (
            self.platform / "kyverno-policies",
            self.platform / "observability-stack",
            self.consumer / "platform/local/exposure",
        ):
            directory.mkdir(parents=True)
            (directory / "kustomization.yaml").write_text("resources: []\n")
        self.roots = {"flux-system": self.consumer}
        self.reconciliation = root / "reconciliation.yaml"

    def write(self, *documents: str) -> Path:
        self.reconciliation.write_text("---\n".join(documents))
        return self.reconciliation

    def test_consumer_path_resolves_under_its_source_root(self):
        spec = {"path": "./platform/local/exposure", "sourceRef": {"name": "flux-system"}}
        directory = fs.kustomization_dir("exposure", spec, self.roots, self.platform)
        self.assertEqual(directory, self.consumer / "platform/local/exposure")

    def test_baukit_path_resolves_in_the_checker_checkout(self):
        spec = {"path": "./deploy/platform/kyverno-policies", "sourceRef": {"name": "baukit"}}
        directory = fs.kustomization_dir("policies", spec, {}, self.platform)
        self.assertEqual(directory, self.platform / "kyverno-policies")

    def test_unknown_source_fails_and_names_the_flag(self):
        spec = {"path": "./platform/local/exposure", "sourceRef": {"name": "flux-system"}}
        with self.assertRaisesRegex(fs.SourceError, "--source-root flux-system=DIR"):
            fs.kustomization_dir("exposure", spec, {}, self.platform)

    def test_missing_source_ref_fails(self):
        with self.assertRaisesRegex(fs.SourceError, "sourceRef must name a GitRepository"):
            fs.kustomization_dir("exposure", {"path": "./"}, self.roots, self.platform)

    def test_missing_path_fails(self):
        spec = {"path": "./platform/local/gone", "sourceRef": {"name": "flux-system"}}
        with self.assertRaisesRegex(fs.SourceError, "is not at"):
            fs.kustomization_dir("gone", spec, self.roots, self.platform)

    def test_baukit_path_outside_the_platform_fails(self):
        spec = {"path": "./rust", "sourceRef": {"name": "baukit"}}
        with self.assertRaisesRegex(fs.SourceError, "outside ./deploy/platform/"):
            fs.kustomization_dir("rust", spec, {}, self.platform)

    def test_source_root_argument(self):
        self.assertEqual(
            fs.source_root(f"flux-system={self.consumer}"), ("flux-system", self.consumer.resolve())
        )
        for value in ("flux-system", "=dir", f"baukit={self.consumer}", "x=/nonexistent/dir"):
            with self.subTest(value=value), self.assertRaises(argparse.ArgumentTypeError):
                fs.source_root(value)

    def test_admission_order_checks_consumer_kustomizations(self):
        reconciliation = self.write(
            kustomization("policies", "./deploy/platform/kyverno-policies", "baukit"),
            kustomization("exposure", "./platform/local/exposure", "flux-system"),
        )
        found = cao.violations(reconciliation, render, self.platform, self.roots)
        self.assertEqual(len(found), 1)
        self.assertIn("exposure creates workloads but does not depend on policies", found[0])

    def test_admission_order_passes_an_ordered_consumer_kustomization(self):
        reconciliation = self.write(
            kustomization("policies", "./deploy/platform/kyverno-policies", "baukit"),
            kustomization("exposure", "./platform/local/exposure", "flux-system", "policies"),
        )
        self.assertEqual(cao.violations(reconciliation, render, self.platform, self.roots), [])

    def test_unresolved_consumer_source_fails_both_checkers(self):
        reconciliation = self.write(
            kustomization("policies", "./deploy/platform/kyverno-policies", "baukit"),
            kustomization("exposure", "./platform/local/exposure", "flux-system", "policies"),
        )
        for found in (
            cao.violations(reconciliation, render, self.platform),
            cmo.violations(reconciliation, set(), self.platform),
        ):
            self.assertEqual(len(found), 1)
            self.assertIn("no checkout for GitRepository/flux-system", found[0])

    def test_monitor_order_reads_consumer_kustomizations(self):
        (self.platform / "kured").mkdir()
        (self.platform / "kured/kustomization.yaml").write_text("resources: []\n")
        (self.consumer / "platform/local/exposure/kustomization.yaml").write_text(
            "resources:\n  - ../../../../baukit/kured\n"
        )
        stack = kustomization("stack", "./deploy/platform/observability-stack", "baukit")
        monitors = {"kured", "observability-stack"}
        unordered = self.write(
            stack, kustomization("exposure", "./platform/local/exposure", "flux-system")
        )
        found = cmo.violations(unordered, monitors, self.platform, self.roots)
        self.assertEqual(len(found), 1)
        self.assertIn("exposure renders Prometheus Operator objects", found[0])
        ordered = self.write(
            stack, kustomization("exposure", "./platform/local/exposure", "flux-system", "stack")
        )
        self.assertEqual(cmo.violations(ordered, monitors, self.platform, self.roots), [])

    def test_main_accepts_source_roots(self):
        reconciliation = self.write(
            kustomization("exposure", "./platform/local/exposure", "flux-system"),
        )
        self.assertEqual(
            cao.main([f"--source-root=flux-system={self.consumer}", str(reconciliation)]), 0
        )
        self.assertEqual(cao.main([str(reconciliation)]), 1)


if __name__ == "__main__":
    unittest.main()
