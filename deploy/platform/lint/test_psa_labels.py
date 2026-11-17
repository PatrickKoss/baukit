from __future__ import annotations

import re
import unittest
from pathlib import Path

PLATFORM = Path(__file__).resolve().parent.parent
LIFECYCLE = PLATFORM / "platform-lifecycle.sh"
K3S_IMAGE_SOURCE = PLATFORM / "local-git-http.sh"
SKIPPED_DIRS = {".helm-cache", ".local-state", "tests", "lint"}
PSA_POLICY = PLATFORM / "kyverno-policies/policies/require-psa-labels.yaml"
ALLOWLIST_EXPRESSION = re.compile(r"name: privilegedAllowlist\n\s+expression: '\[(.*)\]'")
LEVELS = {"privileged", "baseline", "restricted"}
PSA = "pod-security.kubernetes.io/"
DOCUMENT_SEPARATOR = re.compile(r"^---[ \t]*$", re.MULTILINE)
K3S_IMAGE = re.compile(r"rancher/k3s:(v\d+\.\d+)\.\d+")
LIFECYCLE_PSA_VERSION = re.compile(r"^psa_version=(\S+)$", re.MULTILINE)
SCRIPT_ENFORCE_VERSION = re.compile(r"pod-security\.kubernetes\.io/enforce-version=(v[\d.]+)")
PLATFORM_UP = PLATFORM / "platform-up.sh"


def namespaces(text: str) -> list[tuple[str, dict[str, str]]]:
    found = []
    for document in DOCUMENT_SEPARATOR.split(text):
        lines = document.splitlines()
        if "kind: Namespace" not in lines:
            continue
        names = (line.split(":", 1)[1].strip() for line in lines if line.startswith("  name:"))
        name = next(names, "")
        labels = {}
        in_labels = False
        for line in lines:
            if line == "  labels:":
                in_labels = True
                continue
            if in_labels and line.startswith("    ") and ":" in line:
                key, value = line.strip().split(":", 1)
                labels[key] = value.strip().strip("\"'")
            elif in_labels:
                in_labels = False
        found.append((name, labels))
    return found


def violations(name: str, labels: dict[str, str], version: str) -> list[str]:
    problems = []
    enforce = labels.get(PSA + "enforce", "")
    if enforce not in LEVELS:
        problems.append(f"{name}: enforce must be one of {sorted(LEVELS)}")
    if enforce == "privileged" and name not in PRIVILEGED_ALLOWLIST:
        problems.append(f"{name}: privileged is reserved for {sorted(PRIVILEGED_ALLOWLIST)}")
    if labels.get(PSA + "enforce-version") != version:
        problems.append(f"{name}: enforce-version must be {version}")
    for mode in ("audit", "warn"):
        if labels.get(PSA + mode) != "restricted":
            problems.append(f"{name}: {mode} must be restricted")
    return problems


def privileged_allowlist() -> set[str]:
    match = ALLOWLIST_EXPRESSION.search(PSA_POLICY.read_text(encoding="utf-8"))
    if match is None:
        raise AssertionError(f"no privilegedAllowlist variable in {PSA_POLICY}")
    return {item.strip().strip('"') for item in match.group(1).split(",")}


PRIVILEGED_ALLOWLIST = privileged_allowlist()


def cluster_minor() -> str:
    match = K3S_IMAGE.search(K3S_IMAGE_SOURCE.read_text(encoding="utf-8"))
    if match is None:
        raise AssertionError(f"no rancher/k3s image in {K3S_IMAGE_SOURCE}")
    return match.group(1)


def namespace_manifests() -> list[Path]:
    return [
        path
        for path in sorted(PLATFORM.rglob("*.yaml"))
        if not SKIPPED_DIRS.intersection(path.relative_to(PLATFORM).parts)
        and "kind: Namespace" in path.read_text(encoding="utf-8")
    ]


GOOD = """apiVersion: v1
kind: Namespace
metadata:
  name: {name}
  labels:
    app.kubernetes.io/part-of: baukit-platform
    pod-security.kubernetes.io/enforce: {level}
    pod-security.kubernetes.io/enforce-version: v1.34
    pod-security.kubernetes.io/audit: restricted
    pod-security.kubernetes.io/warn: restricted
"""


class ViolationsTest(unittest.TestCase):
    def check(self, text: str) -> list[str]:
        found = namespaces(text)
        return [problem for name, labels in found for problem in violations(name, labels, "v1.34")]

    def test_complete_labels_pass(self) -> None:
        self.assertEqual(self.check(GOOD.format(name="observability", level="restricted")), [])

    def test_allowlisted_privileged_passes(self) -> None:
        self.assertEqual(self.check(GOOD.format(name="kured", level="privileged")), [])

    def test_privileged_outside_allowlist_fails(self) -> None:
        self.assertEqual(
            self.check(GOOD.format(name="traefik", level="privileged")),
            [
                "traefik: privileged is reserved for "
                "['falco', 'kube-system', 'kured', 'observability-agents']"
            ],
        )

    def test_missing_labels_fail(self) -> None:
        text = "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: bare\n"
        self.assertEqual(len(self.check(text)), 4)

    def test_stale_version_fails(self) -> None:
        text = GOOD.format(name="kyverno", level="restricted").replace("v1.34", "v1.33")
        self.assertEqual(self.check(text), ["kyverno: enforce-version must be v1.34"])


class PlatformNamespacesTest(unittest.TestCase):
    def test_every_base_namespace_carries_psa_labels(self) -> None:
        version = cluster_minor()
        manifests = namespace_manifests()
        self.assertTrue(manifests)
        problems = []
        for path in manifests:
            for name, labels in namespaces(path.read_text(encoding="utf-8")):
                relative = path.relative_to(PLATFORM)
                problems += [f"{relative}: {p}" for p in violations(name, labels, version)]
        self.assertEqual(problems, [])

    def test_lifecycle_labels_flux_system_at_the_cluster_minor(self) -> None:
        match = LIFECYCLE_PSA_VERSION.search(LIFECYCLE.read_text(encoding="utf-8"))
        self.assertIsNotNone(match)
        self.assertEqual(match.group(1) if match else "", cluster_minor())

    def test_platform_up_labels_flux_system_at_the_cluster_minor(self) -> None:
        versions = SCRIPT_ENFORCE_VERSION.findall(PLATFORM_UP.read_text(encoding="utf-8"))
        self.assertEqual(versions, [cluster_minor()])


if __name__ == "__main__":
    unittest.main()
