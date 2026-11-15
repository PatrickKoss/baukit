from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import check_monitor_order as cmo

KUSTOMIZATION = """\
apiVersion: kustomize.toolkit.fluxcd.io/v1
kind: Kustomization
metadata:
  name: {name}
spec:
  path: ./deploy/platform/{path}
{depends}"""


def flux(name: str, path: str, *depends_on: str) -> str:
    depends = "".join(f"    - name: {dependency}\n" for dependency in depends_on)
    return KUSTOMIZATION.format(
        name=name, path=path, depends=f"  dependsOn:\n{depends}" if depends else ""
    )


class MonitorOrderTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.platform = Path(self.tmp.name)
        for base in ("observability-stack", "kured", "cluster-baseline"):
            directory = self.platform / base
            directory.mkdir()
            (directory / "kustomization.yaml").write_text("resources: []\n")
        self.monitors = {"kured", "observability-stack"}

    def check(self, *documents: str) -> list[str]:
        reconciliation = self.platform / "reconciliation.yaml"
        reconciliation.write_text("---\n".join(documents))
        return cmo.violations(reconciliation, self.monitors, self.platform)

    def test_monitor_base_without_the_crd_dependency_fails(self):
        found = self.check(
            flux("baseline", "cluster-baseline"),
            flux("observability", "observability-stack", "baseline"),
            flux("kured", "kured", "baseline"),
        )
        self.assertEqual(len(found), 1)
        self.assertIn("kured renders Prometheus Operator objects", found[0])

    def test_transitive_dependency_on_the_crd_provider_passes(self):
        found = self.check(
            flux("observability", "observability-stack"),
            flux("baseline", "cluster-baseline", "observability"),
            flux("kured", "kured", "baseline"),
        )
        self.assertEqual(found, [])

    def test_rendered_file_names_map_to_their_base(self):
        bases = {"kyverno", "kyverno-policies", "observability-stack"}
        self.assertEqual(cmo.rendered_base("chart-kyverno-policy-reporter", bases), "kyverno")
        self.assertEqual(
            cmo.rendered_base("chart-kyverno-policies-kyverno-policies", bases), "kyverno-policies"
        )
        self.assertEqual(
            cmo.rendered_base("base-observability-stack", bases), "observability-stack"
        )
        self.assertIsNone(cmo.rendered_base("overlay-local", bases))


if __name__ == "__main__":
    unittest.main()
