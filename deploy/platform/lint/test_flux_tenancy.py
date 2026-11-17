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
FLUX_VERSION = "v2.10.0"
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
                "apiVersion: kustomize.config.k8s.io/v1beta1\nkind: Kustomization\n"
                "namespace: acme\nresources:\n"
                + "".join(f"  - {resource}\n" for resource in resources)
                + "components:\n  - tenant\n"
            )
            return kustomize(root)

    def test_tenant_gets_a_namespace_scoped_reconciler(self) -> None:
        result = self.render(with_parameters=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        docs = objects(result.stdout)
        accounts = {a["metadata"]["name"]: a for a in by_kind(docs, "ServiceAccount")}
        self.assertEqual(set(accounts), {"flux-reconciler", "default"})
        self.assertEqual(accounts["default"]["metadata"]["namespace"], "acme")
        self.assertFalse(accounts["default"]["automountServiceAccountToken"])
        account = accounts["flux-reconciler"]
        self.assertEqual(
            account["metadata"],
            {**account["metadata"], "name": "flux-reconciler", "namespace": "acme"},
        )
        self.assertFalse(account["automountServiceAccountToken"])
        [binding] = by_kind(docs, "RoleBinding")
        self.assertEqual(binding["metadata"]["namespace"], "acme")
        self.assertEqual(
            binding["roleRef"],
            {"apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": "admin"},
        )
        self.assertEqual(
            binding["subjects"],
            [{"kind": "ServiceAccount", "name": "flux-reconciler", "namespace": "acme"}],
        )
        self.assertEqual(by_kind(docs, "ClusterRoleBinding"), [])
        self.assertEqual(by_kind(docs, "ClusterRole"), [])
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
        self.assertEqual(
            namespace["metadata"]["annotations"]["baukit.dev/sops-age-recipient"], "age1acme"
        )
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
                "  name: flux-system\n  namespace: flux-system\nspec:\n  interval: 10m\n"
                "  path: ./\n"
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
        lockdown = {
            "--no-cross-namespace-refs=true",
            "--no-remote-bases=true",
            "--default-service-account=default",
        }
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


def kustomization_roots() -> list[Path]:
    roots = [
        path.parent
        for path in sorted((PLATFORM / "overlays").rglob("kustomization.yaml"))
        if "kind: Kustomization" in path.read_text()
    ]
    e2e_overlay = PLATFORM / "e2e/overlay"
    if (e2e_overlay / "kustomization.yaml").is_file():
        roots.append(e2e_overlay)
    return roots


def render_roots() -> list[Path]:
    bases = sorted(
        path.parent
        for path in PLATFORM.glob("*/kustomization.yaml")
        if path.parent.name != "overlays"
    )
    return bases + kustomization_roots()


def account_findings(docs: list[dict]) -> list[str]:
    return [
        f"{doc['metadata'].get('namespace', '')}/{doc['metadata']['name']}"
        for doc in docs
        if doc.get("kind") == "Kustomization"
        and is_flux_object(doc)
        and (
            doc["metadata"].get("namespace") != "flux-system"
            or doc["spec"].get("serviceAccountName") != "kustomize-controller"
        )
    ]


def source_findings(docs: list[dict]) -> list[str]:
    return [
        f"{doc['kind']}/{doc['metadata'].get('namespace', '')}/{doc['metadata']['name']}"
        for doc in docs
        if is_flux_object(doc)
        and set(source_namespaces(doc)) - {doc["metadata"].get("namespace", "")}
    ]


def rendered(path: Path) -> list[dict]:
    result = kustomize(path)
    if result.returncode != 0:
        raise AssertionError(f"{path}: {result.stderr}")
    return objects(result.stdout)


PLATFORM_KUSTOMIZATION = {
    "apiVersion": "kustomize.toolkit.fluxcd.io/v1",
    "kind": "Kustomization",
    "metadata": {"name": "local-baseline", "namespace": "flux-system"},
    "spec": {
        "serviceAccountName": "kustomize-controller",
        "sourceRef": {"kind": "GitRepository", "name": "baukit"},
    },
}


CLUSTER_SCOPED = {
    "APIService",
    "ClusterIssuer",
    "ClusterPolicy",
    "ClusterRole",
    "ClusterRoleBinding",
    "ClusterCleanupPolicy",
    "CustomResourceDefinition",
    "DeletingPolicy",
    "GeneratingPolicy",
    "ImageValidatingPolicy",
    "GatewayClass",
    "IngressClass",
    "MutatingAdmissionPolicy",
    "MutatingAdmissionPolicyBinding",
    "MutatingPolicy",
    "MutatingWebhookConfiguration",
    "Namespace",
    "PersistentVolume",
    "PriorityClass",
    "RuntimeClass",
    "StorageClass",
    "ValidatingAdmissionPolicy",
    "ValidatingAdmissionPolicyBinding",
    "ValidatingPolicy",
    "ValidatingWebhookConfiguration",
}


def namespace_findings(kustomization: dict, docs: list[dict]) -> list[str]:
    if kustomization["spec"].get("targetNamespace"):
        return []
    return [
        f"{doc['kind']}/{doc['metadata']['name']}"
        for doc in docs
        if doc.get("kind") not in CLUSTER_SCOPED and not doc["metadata"].get("namespace")
    ]


def platform_kustomizations() -> list[tuple[str, dict]]:
    found = []
    for root in kustomization_roots():
        for doc in rendered(root):
            if is_flux_object(doc) and doc["kind"] == "Kustomization":
                found.append((root.relative_to(PLATFORM).as_posix(), doc))
    return found


class PlatformUnderLockdownTest(unittest.TestCase):
    def test_every_overlay_is_covered(self) -> None:
        roots = {path.relative_to(PLATFORM).as_posix() for path in kustomization_roots()}
        self.assertIn("overlays/local", roots)
        self.assertIn("overlays/everything", roots)

    def test_platform_kustomizations_name_the_controller_account(self) -> None:
        checked = 0
        for root in kustomization_roots():
            docs = rendered(root)
            checked += sum(
                1 for doc in docs if is_flux_object(doc) and doc["kind"] == "Kustomization"
            )
            with self.subTest(root=root.relative_to(PLATFORM).as_posix()):
                self.assertEqual(account_findings(docs), [])
        self.assertGreater(checked, 0)

    def test_platform_flux_objects_reference_sources_in_their_own_namespace(self) -> None:
        checked = 0
        for root in render_roots():
            docs = rendered(root)
            checked += sum(1 for doc in docs if is_flux_object(doc))
            with self.subTest(root=root.relative_to(PLATFORM).as_posix()):
                self.assertEqual(source_findings(docs), [])
        self.assertGreater(checked, 0)

    def test_kustomization_without_the_controller_account_is_found(self) -> None:
        self.assertEqual(account_findings([PLATFORM_KUSTOMIZATION]), [])
        missing = {
            **PLATFORM_KUSTOMIZATION,
            "spec": {"sourceRef": PLATFORM_KUSTOMIZATION["spec"]["sourceRef"]},
        }
        self.assertEqual(account_findings([missing]), ["flux-system/local-baseline"])
        elsewhere = {
            **PLATFORM_KUSTOMIZATION,
            "metadata": {"name": "x", "namespace": "observability"},
        }
        self.assertEqual(account_findings([elsewhere]), ["observability/x"])

    def test_cross_namespace_source_is_found(self) -> None:
        doc = {
            "kind": "HelmRelease",
            "apiVersion": "helm.toolkit.fluxcd.io/v2",
            "metadata": {"name": "x", "namespace": "observability"},
            "spec": {
                "dependsOn": [{"name": "kube-prometheus-stack", "namespace": "observability"}],
                "chart": {
                    "spec": {
                        "sourceRef": {
                            "kind": "GitRepository",
                            "name": "baukit",
                            "namespace": "flux-system",
                        }
                    }
                },
            },
        }
        self.assertEqual(source_findings([doc]), ["HelmRelease/observability/x"])
        own = {**doc, "metadata": {"name": "x", "namespace": "flux-system"}}
        self.assertEqual(source_findings([own]), [])

    def test_every_namespaced_object_of_a_platform_kustomization_has_a_namespace(self) -> None:
        checked = 0
        for root, kustomization in platform_kustomizations():
            path = kustomization["spec"].get("path", "")
            if not path.startswith("./deploy/platform/"):
                continue
            checked += 1
            docs = rendered(PLATFORM.parent.parent / path)
            with self.subTest(root=root, kustomization=kustomization["metadata"]["name"]):
                self.assertEqual(namespace_findings(kustomization, docs), [])
        self.assertGreater(checked, 0)

    def test_object_without_a_namespace_is_found(self) -> None:
        binding = {"kind": "RoleBinding", "metadata": {"name": "flux-reconciler"}}
        role = {"kind": "ClusterRole", "metadata": {"name": "reader"}}
        self.assertEqual(
            namespace_findings(PLATFORM_KUSTOMIZATION, [binding, role]),
            ["RoleBinding/flux-reconciler"],
        )
        placed = {**binding, "metadata": {"name": "flux-reconciler", "namespace": "acme"}}
        self.assertEqual(namespace_findings(PLATFORM_KUSTOMIZATION, [placed, role]), [])
        targeted = {
            **PLATFORM_KUSTOMIZATION,
            "spec": {**PLATFORM_KUSTOMIZATION["spec"], "targetNamespace": "acme"},
        }
        self.assertEqual(namespace_findings(targeted, [binding]), [])


EDIT_VERBS = ["create", "delete", "deletecollection", "patch", "update"]
VIEW_VERBS = ["get", "list", "watch"]
TENANT_GRANTS = {
    "flagger": ("flagger.app", ["canaries", "metrictemplates"]),
    "traefik": ("traefik.io", ["ingressroutes", "middlewares", "traefikservices"]),
    "observability-stack": ("monitoring.coreos.com", ["servicemonitors"]),
}
AGGREGATE = "rbac.authorization.k8s.io/aggregate-to-"


def aggregated_roles(docs: list[dict]) -> dict[str, dict]:
    return {
        doc["metadata"]["name"]: doc
        for doc in by_kind(docs, "ClusterRole")
        if any(key.startswith(AGGREGATE) for key in doc["metadata"].get("labels", {}))
    }


class TenantAggregatedRolesTest(unittest.TestCase):
    def test_each_crd_base_grants_tenant_admins_exactly_the_chart_kinds(self) -> None:
        for base, (group, resources) in TENANT_GRANTS.items():
            with self.subTest(base=base):
                roles = list(aggregated_roles(rendered(PLATFORM / base)).values())
                rules = [rule for role in roles for rule in role["rules"]]
                self.assertEqual(sorted(rule["verbs"] for rule in rules), [EDIT_VERBS, VIEW_VERBS])
                for rule in rules:
                    self.assertEqual(rule["apiGroups"], [group])
                    self.assertEqual(rule["resources"], resources)
                for role in roles:
                    labels = role["metadata"]["labels"]
                    self.assertEqual(labels[f"{AGGREGATE}admin"], "true")
                    writes = role["rules"][0]["verbs"] == EDIT_VERBS
                    self.assertEqual(f"{AGGREGATE}view" in labels, not writes)

    def test_traefik_refuses_cross_namespace_and_external_name_refs(self) -> None:
        [release] = by_kind(rendered(PLATFORM / "traefik"), "HelmRelease")
        providers = release["spec"]["values"]["providers"]
        self.assertIs(providers["kubernetesCRD"]["allowCrossNamespace"], False)
        self.assertIs(providers["kubernetesCRD"]["allowExternalNameServices"], False)
        self.assertIs(providers["kubernetesIngress"]["allowExternalNameServices"], False)

    def test_only_kyverno_reads_ingressroutetcps(self) -> None:
        roles = {
            doc["metadata"]["name"]: doc
            for doc in by_kind(rendered(PLATFORM / "traefik"), "ClusterRole")
        }
        role = roles["baukit-kyverno-traefik-tcp-view"]
        labels = role["metadata"]["labels"]
        self.assertFalse(any(key.startswith(AGGREGATE) for key in labels))
        self.assertEqual(labels["rbac.kyverno.io/aggregate-to-admission-controller"], "true")
        self.assertEqual(
            role["rules"],
            [{"apiGroups": ["traefik.io"], "resources": ["ingressroutetcps"], "verbs": VIEW_VERBS}],
        )

    def test_no_other_base_aggregates_into_admin(self) -> None:
        for path in sorted(PLATFORM.glob("*/kustomization.yaml")):
            base = path.parent.name
            if base == "overlays" or base in TENANT_GRANTS:
                continue
            with self.subTest(base=base):
                self.assertEqual(aggregated_roles(rendered(path.parent)), {})


if __name__ == "__main__":
    unittest.main()
