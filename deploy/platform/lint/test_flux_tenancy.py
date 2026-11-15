from __future__ import annotations

import os
import shutil
import tempfile
import unittest
from pathlib import Path

import helm_yaml

PLATFORM = Path(__file__).resolve().parent.parent
TOOLS = PLATFORM.parent.parent / ".tools/bin"
TENANT = PLATFORM / "components/tenant"
LOCKDOWN = PLATFORM / "components/flux-lockdown"
FLUX_VERSION = "v2.9.4"
FLUX_KINDS = {"Kustomization", "HelmRelease"}
PARAMETERS = """\
apiVersion: v1
kind: ConfigMap
metadata:
  name: baukit-tenant
  annotations:
    config.kubernetes.io/local-config: "true"
data:
  sopsSecretName: sops-age-acme
  sopsAgeRecipient: age1acme
"""
TENANT_OBJECTS = """\
apiVersion: v1
kind: Namespace
metadata:
  name: acme
---
apiVersion: kustomize.toolkit.fluxcd.io/v1
kind: Kustomization
metadata:
  name: apps
spec:
  interval: 10m
  path: ./deploy
  prune: true
  sourceRef:
    kind: GitRepository
    name: acme
---
apiVersion: helm.toolkit.fluxcd.io/v2
kind: HelmRelease
metadata:
  name: app
spec:
  interval: 10m
  chartRef:
    kind: OCIRepository
    name: app
"""


def run(*args: str):
    return helm_yaml.process_runner.run_process(
        list(args),
        capture_output=True,
        text=True,
        env={**os.environ, "PATH": f"{TOOLS}:{os.environ['PATH']}"},
    )


def kustomize(path: Path):
    return run(str(TOOLS / "kustomize"), "build", str(path))


def objects(text: str) -> list[dict]:
    return [doc for doc in helm_yaml.parse_documents(helm_yaml.split_documents(text)) if doc]


def by_kind(docs: list[dict], kind: str) -> list[dict]:
    return [doc for doc in docs if doc.get("kind") == kind]


def is_flux_object(doc: dict) -> bool:
    return doc.get("kind") in FLUX_KINDS and doc.get("apiVersion", "").split(".")[0] in {
        "kustomize",
        "helm",
    }


def source_namespaces(doc: dict) -> list[str]:
    spec = doc.get("spec", {})
    refs = [spec.get("sourceRef"), spec.get("chart", {}).get("spec", {}).get("sourceRef")]
    refs.append(spec.get("chartRef"))
    namespace = doc["metadata"].get("namespace", "")
    return [ref.get("namespace", namespace) for ref in refs if ref]


class TenantComponentTest(unittest.TestCase):
    def render(self, *, with_parameters: bool):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shutil.copytree(TENANT, root / "tenant")
            resources = ["tenant.yaml"] + (["parameters.yaml"] if with_parameters else [])
            (root / "tenant.yaml").write_text(TENANT_OBJECTS)
            (root / "parameters.yaml").write_text(PARAMETERS)
            (root / "kustomization.yaml").write_text(
                "apiVersion: kustomize.config.k8s.io/v1beta1\nkind: Kustomization\nnamespace: acme\n"
                "resources:\n"
                + "".join(f"  - {resource}\n" for resource in resources)
                + "components:\n  - tenant\n"
            )
            return kustomize(root)

    def test_tenant_gets_a_namespace_scoped_reconciler(self) -> None:
        result = self.render(with_parameters=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        docs = objects(result.stdout)
        [account] = by_kind(docs, "ServiceAccount")
        self.assertEqual(account["metadata"], {**account["metadata"], "name": "flux-reconciler", "namespace": "acme"})
        self.assertFalse(account["automountServiceAccountToken"])
        [binding] = by_kind(docs, "RoleBinding")
        self.assertEqual(binding["metadata"]["namespace"], "acme")
        self.assertEqual(binding["roleRef"], {"apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": "admin"})
        self.assertEqual(binding["subjects"], [{"kind": "ServiceAccount", "name": "flux-reconciler", "namespace": "acme"}])
        self.assertEqual(by_kind(docs, "ClusterRoleBinding"), [])
        self.assertEqual(by_kind(docs, "ConfigMap"), [])

    def test_flux_objects_reconcile_as_the_tenant_with_its_own_sops_key(self) -> None:
        result = self.render(with_parameters=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        docs = objects(result.stdout)
        for doc in docs:
            if is_flux_object(doc):
                self.assertEqual(doc["spec"]["serviceAccountName"], "flux-reconciler")
        [kustomization] = by_kind(docs, "Kustomization")
        self.assertEqual(
            kustomization["spec"]["decryption"],
            {"provider": "sops", "secretRef": {"name": "sops-age-acme"}},
        )
        self.assertEqual(kustomization["spec"]["targetNamespace"], "acme")
        [namespace] = by_kind(docs, "Namespace")
        self.assertEqual(namespace["metadata"]["annotations"]["baukit.dev/sops-age-recipient"], "age1acme")
        self.assertEqual(namespace["metadata"]["labels"]["baukit.dev/tenant"], "true")

    def test_tenant_without_parameters_fails(self) -> None:
        result = self.render(with_parameters=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("baukit-tenant", result.stderr)


class FluxLockdownComponentTest(unittest.TestCase):
    def test_controllers_get_the_lockdown_flags(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shutil.copytree(LOCKDOWN, root / "flux-lockdown")
            export = run(
                str(TOOLS / "flux"),
                "install",
                f"--version={FLUX_VERSION}",
                "--components=source-controller,kustomize-controller,helm-controller,notification-controller",
                "--network-policy=false",
                "--export",
            )
            self.assertEqual(export.returncode, 0, export.stderr)
            (root / "gotk-components.yaml").write_text(export.stdout)
            (root / "gotk-sync.yaml").write_text(
                "apiVersion: kustomize.toolkit.fluxcd.io/v1\nkind: Kustomization\nmetadata:\n"
                "  name: flux-system\n  namespace: flux-system\nspec:\n  interval: 10m\n  path: ./\n"
                "  prune: true\n  sourceRef:\n    kind: GitRepository\n    name: flux-system\n"
            )
            (root / "kustomization.yaml").write_text(
                "apiVersion: kustomize.config.k8s.io/v1beta1\nkind: Kustomization\nresources:\n"
                "  - gotk-components.yaml\n  - gotk-sync.yaml\ncomponents:\n  - flux-lockdown\n"
            )
            result = kustomize(root)
        self.assertEqual(result.returncode, 0, result.stderr)
        docs = objects(result.stdout)
        args = {
            doc["metadata"]["name"]: doc["spec"]["template"]["spec"]["containers"][0]["args"]
            for doc in by_kind(docs, "Deployment")
        }
        lockdown = {"--no-cross-namespace-refs=true", "--no-remote-bases=true", "--default-service-account=default"}
        expected = {
            "source-controller": set(),
            "kustomize-controller": lockdown,
            "helm-controller": {"--no-cross-namespace-refs=true"},
            "notification-controller": set(),
        }
        self.assertEqual(set(args), set(expected))
        for name, flags in expected.items():
            with self.subTest(controller=name):
                self.assertEqual(lockdown.intersection(args[name]), flags)
        [sync] = by_kind(docs, "Kustomization")
        self.assertEqual(sync["spec"]["serviceAccountName"], "kustomize-controller")


class PlatformUnderLockdownTest(unittest.TestCase):
    def test_platform_kustomizations_name_the_controller_account(self) -> None:
        result = kustomize(PLATFORM / "overlays/local")
        self.assertEqual(result.returncode, 0, result.stderr)
        kustomizations = [doc for doc in objects(result.stdout) if is_flux_object(doc)]
        self.assertTrue(kustomizations)
        for doc in kustomizations:
            with self.subTest(name=doc["metadata"]["name"]):
                self.assertEqual(doc["metadata"]["namespace"], "flux-system")
                self.assertEqual(doc["spec"].get("serviceAccountName"), "kustomize-controller")

    def test_platform_flux_objects_reference_sources_in_their_own_namespace(self) -> None:
        bases = sorted(
            path.parent
            for path in PLATFORM.glob("*/kustomization.yaml")
            if path.parent.name != "overlays"
        )
        checked = 0
        for base in bases:
            result = kustomize(base)
            self.assertEqual(result.returncode, 0, result.stderr)
            for doc in objects(result.stdout):
                if not is_flux_object(doc):
                    continue
                checked += 1
                with self.subTest(base=base.name, name=doc["metadata"]["name"]):
                    self.assertEqual(set(source_namespaces(doc)), {doc["metadata"]["namespace"]})
        self.assertGreater(checked, 0)

    def test_cross_namespace_source_is_detected(self) -> None:
        doc = {
            "kind": "HelmRelease",
            "apiVersion": "helm.toolkit.fluxcd.io/v2",
            "metadata": {"name": "x", "namespace": "observability"},
            "spec": {"chart": {"spec": {"sourceRef": {"kind": "GitRepository", "name": "baukit", "namespace": "flux-system"}}}},
        }
        self.assertEqual(source_namespaces(doc), ["flux-system"])


if __name__ == "__main__":
    unittest.main()
