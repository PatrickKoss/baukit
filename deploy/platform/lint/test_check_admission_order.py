from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import check_admission_order as cao
from test_check_monitor_order import flux

WORKLOADS = {"kured": "kind: DaemonSet\n", "tenant": "kind: HelmRelease\n"}


E2E_RECONCILIATION = cao.PLATFORM / "e2e" / "overlay" / "reconciliation.yaml"


def everything_but_baseline(path: Path) -> str:
    return "kind: Namespace\n" if path.name == "cluster-baseline" else "kind: Deployment\n"


def render(path: Path) -> str:
    return WORKLOADS.get(path.name, "kind: ConfigMap\n")


class AdmissionOrderTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.platform = Path(self.tmp.name)
        for base in ("kyverno", "kyverno-policies", "kured", "cluster-baseline", "tenant"):
            directory = self.platform / base
            directory.mkdir()
            (directory / "kustomization.yaml").write_text("resources: []\n")

    def check(self, *documents: str) -> list[str]:
        reconciliation = self.platform / "reconciliation.yaml"
        reconciliation.write_text("---\n".join(documents))
        return cao.violations(reconciliation, render, self.platform)

    def test_workload_before_the_policies_fails(self):
        found = self.check(
            flux("e2e-kyverno", "kyverno"),
            flux("e2e-kyverno-policies", "kyverno-policies", "e2e-kyverno"),
            flux("e2e-kured", "kured", "e2e-kyverno"),
        )
        self.assertEqual(len(found), 1)
        self.assertIn(
            "e2e-kured creates workloads but does not depend on e2e-kyverno-policies", found[0]
        )

    def test_transitive_dependency_on_the_policies_passes(self):
        found = self.check(
            flux("e2e-kyverno", "kyverno"),
            flux("e2e-kyverno-policies", "kyverno-policies", "e2e-kyverno"),
            flux("e2e-baseline", "cluster-baseline", "e2e-kyverno-policies"),
            flux("e2e-kured", "kured", "e2e-baseline"),
            flux("e2e-tenant", "tenant", "e2e-kured"),
        )
        self.assertEqual(found, [])

    def test_kustomization_without_workloads_needs_no_order(self):
        found = self.check(
            flux("e2e-kyverno-policies", "kyverno-policies"),
            flux("e2e-baseline", "cluster-baseline"),
        )
        self.assertEqual(found, [])

    def test_exempt_kustomizations_pass_and_state_a_reason(self):
        self.assertIsNotNone(cao.exemption("e2e-kyverno"))
        self.assertIsNotNone(cao.exemption("local-foundation"))
        self.assertIsNone(cao.exemption("e2e-kyverno-policies"))
        self.assertIsNone(cao.exemption("e2e-kured"))

    def test_overlay_without_policies_passes(self):
        self.assertEqual(self.check(flux("local-kured", "kured")), [])

    def test_e2e_overlay_orders_every_workload_after_the_policies(self):
        self.assertEqual(cao.violations(E2E_RECONCILIATION, everything_but_baseline), [])

    def test_e2e_overlay_with_observability_before_the_policies_fails(self):
        broken = E2E_RECONCILIATION.read_text().replace(
            "    - name: e2e-foundation\n    - name: e2e-kyverno-policies\n",
            "    - name: e2e-foundation\n",
        )
        reconciliation = self.platform / "reconciliation.yaml"
        reconciliation.write_text(broken)
        found = cao.violations(reconciliation, everything_but_baseline, cao.PLATFORM)
        self.assertIn("e2e-observability", "\n".join(found))
        self.assertIn("e2e-kured", "\n".join(found))


if __name__ == "__main__":
    unittest.main()
