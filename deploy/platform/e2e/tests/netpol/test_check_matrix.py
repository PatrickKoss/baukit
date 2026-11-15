import shutil
import tempfile
import unittest
from pathlib import Path

import check_matrix

KUSTOMIZE = shutil.which("kustomize") or "kustomize"
RAW = """\
apiVersion: v1
kind: Namespace
metadata:
  name: obs
---
apiVersion: helm.toolkit.fluxcd.io/v2
kind: HelmRelease
metadata:
  name: gateway
  namespace: obs
spec:
  chart:
    spec:
      chart: gateway
---
apiVersion: v1
kind: Service
metadata:
  name: sink
  namespace: obs
spec:
  ports:
    - port: 8080
---
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: prometheus
  namespace: obs
spec:
  podSelector:
    matchLabels:
      app: prometheus
  policyTypes: [Ingress, Egress]
  ingress:
    - from:
        - namespaceSelector:
            matchLabels:
              client: "true"
          podSelector:
            matchLabels:
              role: client
      ports:
        - port: 9090
---
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata:
  name: pg
  namespace: obs
"""
CHART = """\
apiVersion: v1
kind: Service
metadata:
  name: gateway
spec:
  ports:
    - port: 80
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: gateway
spec:
  template:
    metadata:
      labels:
        app: gateway
    spec:
      containers:
        - name: gateway
          ports:
            - containerPort: 8080
"""
SUITE = """\
apiVersion: chainsaw.kyverno.io/v1alpha1
kind: Test
metadata:
  name: netpol
spec:
  steps:
    - try:
        - apply:
            resource:
              apiVersion: v1
              kind: Namespace
              metadata:
                name: netpol-t
                labels:
                  baukit.dev/tenant: "true"
                  client: "true"
"""
GOOD = """\
- from: {namespace: netpol-t, labels: {role: client}}
  to: {pod: {namespace: obs, labels: {app: prometheus}}, port: 9090}
  expect: allow
- from: {namespace: obs, labels: {app: gateway}}
  to: {service: gateway.obs, port: 80}
  expect: allow
- from: {namespace: netpol-t, labels: {}}
  to: {service: sink.obs, port: 8080}
  expect: deny
- from: {namespace: obs, labels: {app: prometheus}}
  to: {service: pg-rw.obs, port: 5432}
  expect: allow
- from: {namespace: flux-system, labels: {}}
  to: {pod: {namespace: kube-system, labels: {k8s-app: kube-dns}}, port: 53}
  expect: allow
- from: {namespace: obs, labels: {app: prometheus}}
  to: {pod: {namespace: obs, labels: {app: gateway}}, port: 8080}
  expect: allow
- from: {namespace: obs, labels: {app: gateway}}
  to: {host: node, port: 10250}
  expect: deny
"""


class CheckMatrixTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(self.enterContext(tempfile.TemporaryDirectory()))
        overlay = self.tmp / "overlay"
        overlay.mkdir()
        (overlay / "objects.yaml").write_text(RAW)
        (overlay / "kustomization.yaml").write_text("resources:\n  - objects.yaml\n")
        self.paths = [overlay]
        rendered = self.tmp / "rendered"
        rendered.mkdir()
        (rendered / "base-obs.yaml").write_text(RAW)
        (rendered / "chart-obs-gateway.yaml").write_text(CHART)
        (rendered / "install-flux-lockdown.yaml").write_text(
            "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: flux-system\n"
        )
        self.rendered = rendered
        self.suite = self.tmp / "chainsaw-test.yaml"
        self.suite.write_text(SUITE)

    def problems(self, matrix):
        path = self.tmp / "matrix.yaml"
        path.write_text(matrix)
        return check_matrix.problems(self.rendered, KUSTOMIZE, path, self.paths, self.suite)

    def assert_one_problem(self, matrix, text):
        found = self.problems(matrix)
        self.assertEqual(len(found), 1, found)
        self.assertIn(text, found[0])

    def test_rows_that_resolve_pass(self):
        self.assertEqual(self.problems(GOOD), [])

    def test_a_renamed_service_fails(self):
        self.assert_one_problem(
            GOOD.replace("gateway.obs, port: 80", "gw.obs, port: 80"),
            "Service obs/gw is not in the e2e cluster",
        )

    def test_a_service_port_that_does_not_exist_fails(self):
        self.assert_one_problem(
            GOOD.replace("sink.obs, port: 8080", "sink.obs, port: 8081"),
            "Service obs/sink has no port 8081",
        )

    def test_an_unknown_namespace_fails(self):
        self.assert_one_problem(
            GOOD.replace("from: {namespace: flux-system", "from: {namespace: flux"),
            "namespace flux is not in the e2e cluster",
        )

    def test_source_labels_no_pod_carries_fail(self):
        self.assert_one_problem(
            GOOD.replace(
                "from: {namespace: obs, labels: {app: gateway}}\n  to: {service",
                "from: {namespace: obs, labels: {app: gw}}\n  to: {service",
            ),
            "no pod in obs carries the source labels",
        )

    def test_destination_pod_port_that_is_not_exposed_fails(self):
        self.assert_one_problem(
            GOOD.replace(
                "labels: {app: gateway}}, port: 8080", "labels: {app: gateway}}, port: 8443"
            ),
            "the destination pods expose no port 8443",
        )

    def test_suite_namespace_labels_need_a_selecting_peer(self):
        self.assert_one_problem(
            GOOD.replace("labels: {role: client}", "labels: {role: other}"),
            "no pod in netpol-t carries the source labels",
        )

    def test_suite_namespace_without_tenant_label_fails(self):
        self.suite.write_text(
            SUITE.replace('baukit.dev/tenant: "true"', 'baukit.dev/tenant: "false"')
        )
        self.assert_one_problem(GOOD, "suite namespace netpol-t lacks baukit.dev/tenant=true")

    def test_the_checked_in_suite_namespaces_are_tenants(self):
        self.assertEqual(check_matrix.suite_problems(check_matrix.CHAINSAW_TEST), [])
        names = {
            item["metadata"]["name"]
            for item in check_matrix.suite_namespaces(check_matrix.CHAINSAW_TEST)
        }
        self.assertEqual(names, {"netpol-tenant", "netpol-tenant-open", "netpol-tenant-pg"})


if __name__ == "__main__":
    unittest.main()
