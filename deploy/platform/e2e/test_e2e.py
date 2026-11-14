import json
import os
import re
import subprocess
import sys
import unittest
from pathlib import Path

E2E = Path(__file__).resolve().parent
PLATFORM = E2E.parent
REPO = PLATFORM.parent.parent
RECONCILIATION = E2E / "overlay/reconciliation.yaml"
NOT_BASES = {"components", "e2e", "lib", "lint", "overlays", "schemas"}
LEFT_OUT = {"hcloud", "posthog", "redis"}


def kustomization(name, depends_on=()):
    spec = {"dependsOn": [{"name": dependency} for dependency in depends_on]}
    return {
        "apiVersion": "kustomize.toolkit.fluxcd.io/v1",
        "kind": "Kustomization",
        "metadata": {"name": name},
        "spec": spec,
    }


def run_without(document, *names):
    return subprocess.run(  # noqa: S603 -- argv is this interpreter and the repo's without.py
        [sys.executable, str(E2E / "without.py"), *names],
        input=json.dumps(document),
        capture_output=True,
        text=True,
        check=False,
    )


def referenced_bases():
    paths = re.findall(r"^  path: \./deploy/platform/(\S+)$", RECONCILIATION.read_text(), re.M)
    pending = [PLATFORM / path for path in paths]
    seen = set()
    while pending:
        directory = pending.pop().resolve()
        if directory in seen:
            continue
        seen.add(directory)
        text = (directory / "kustomization.yaml").read_text()
        for reference in re.findall(r"^\s+- (\S+)$", text, re.M):
            target = directory / reference
            if (target / "kustomization.yaml").is_file():
                pending.append(target)
    return {
        directory.relative_to(PLATFORM).parts[0]
        for directory in seen
        if directory.parent == PLATFORM and directory.name not in NOT_BASES
    }


class WithoutTests(unittest.TestCase):
    def test_drops_the_kustomization_and_its_dependency_edges(self):
        document = {
            "kind": "List",
            "items": [kustomization("e2e-a"), kustomization("e2e-b", ["e2e-a", "e2e-c"])],
        }
        result = run_without(document, "e2e-a")
        self.assertEqual(result.returncode, 0, result.stderr)
        items = json.loads(result.stdout)["items"]
        self.assertEqual([item["metadata"]["name"] for item in items], ["e2e-b"])
        self.assertEqual(items[0]["spec"]["dependsOn"], [{"name": "e2e-c"}])

    def test_keeps_everything_without_names(self):
        document = {"kind": "List", "items": [kustomization("e2e-a")]}
        result = run_without(document)
        self.assertEqual(json.loads(result.stdout), document)

    def test_unknown_name_exits_2(self):
        result = run_without({"kind": "List", "items": [kustomization("e2e-a")]}, "e2e-x")
        self.assertEqual(result.returncode, 2)
        self.assertIn("e2e-x", result.stderr)


class OverlayTests(unittest.TestCase):
    def test_composes_every_base_except_the_left_out_ones(self):
        bases = {
            path.parent.name
            for path in PLATFORM.glob("*/kustomization.yaml")
            if path.parent.name not in NOT_BASES
        }
        self.assertEqual(referenced_bases(), bases - LEFT_OUT)

    def test_tenant_waits_for_the_policies(self):
        text = RECONCILIATION.read_text()
        tenant = next(doc for doc in text.split("\n---\n") if "name: e2e-tenant\n" in doc)
        self.assertIn("- name: e2e-kyverno-policies", tenant)


class RunnerArgumentTests(unittest.TestCase):
    def run_runner(self, *arguments):
        environment = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}
        return subprocess.run(  # noqa: S603 -- argv is the repo's run.sh, no shell
            [str(E2E / "run.sh"), *arguments],
            capture_output=True,
            text=True,
            check=False,
            env=environment,
            cwd=REPO,
        )

    def test_unknown_flag_exits_2(self):
        result = self.run_runner("--bogus")
        self.assertEqual(result.returncode, 2)
        self.assertIn("usage:", result.stderr)

    def test_bad_cluster_name_exits_2(self):
        result = self.run_runner("--cluster-name", "Bad_Name")
        self.assertEqual(result.returncode, 2)
        self.assertIn("--cluster-name", result.stderr)

    def test_unknown_without_exits_2(self):
        result = self.run_runner("--without", "no-such-kustomization")
        self.assertEqual(result.returncode, 2)
        self.assertIn("e2e-no-such-kustomization", result.stderr)

    def test_missing_suite_exits_2(self):
        result = self.run_runner("--suite", "no-such-suite")
        self.assertEqual(result.returncode, 2)
        self.assertIn("no chainsaw-test.yaml", result.stderr)


if __name__ == "__main__":
    unittest.main()
