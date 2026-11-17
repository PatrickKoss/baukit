import hashlib
import json
import re
import tempfile
import unittest
from pathlib import Path

import run_cluster_policies as rcl

POLICIES = rcl.PLATFORM / "kyverno-policies/policies"
SNAPSHOT_TREE = rcl.REPO_ROOT / "cli/tests/snapshots/platform.tree"

ALLOWLIST = """\
apiVersion: v1
kind: ConfigMap
metadata:
  name: baukit-registry-allowlist
  namespace: kyverno
data:
  registries: |
    ghcr.io/
"""

NAMESPACE = """\
apiVersion: v1
kind: Namespace
metadata:
  name: demo
  labels:
    pod-security.kubernetes.io/enforce: baseline
    pod-security.kubernetes.io/enforce-version: v1.34
    pod-security.kubernetes.io/audit: restricted
    pod-security.kubernetes.io/warn: restricted
"""

POD_WITH_LIMITS = """\
apiVersion: v1
kind: Pod
metadata:
  name: app
spec:
  containers:
    - name: app
      image: ghcr.io/example/app:1.0.0
      resources:
        requests: {cpu: 10m, memory: 16Mi}
        limits: {cpu: 100m, memory: 64Mi}
"""

POD_WITHOUT_LIMITS = """\
apiVersion: v1
kind: Pod
metadata:
  name: app
spec:
  securityContext: null
  containers:
    - name: app
      image: ghcr.io/example/app:1.0.0
"""

EXCEPTION = """\
apiVersion: policies.kyverno.io/v1
kind: PolicyException
metadata:
  name: demo-app-resources
  namespace: kyverno
  annotations:
    baukit.dev/reason: test fixture
spec:
  policyRefs:
    - name: require-container-resources
      kind: ValidatingPolicy
  matchConditions:
    - name: demo-app
      expression: object.metadata.name == 'app'
"""


def policy(name: str, actions: list[str] | None = None, kind: str = "ValidatingPolicy") -> dict:
    spec = {"validationActions": actions} if actions else {}
    return {"kind": kind, "metadata": {"name": name}, "spec": spec}


def report(*results: tuple[str, str]) -> dict:
    return {
        "results": [
            {
                "policy": name,
                "result": outcome,
                "message": "said so",
                "resources": [{"kind": "Pod", "namespace": "demo", "name": "app"}],
            }
            for name, outcome in results
        ]
    }


def policies_with(name: str, actions: str) -> str:
    """The rendered in-cluster policy set, with one policy's validationActions replaced."""
    documents = []
    for path in sorted(POLICIES.glob("*.yaml")):
        text = path.read_text()
        if re.search(rf"^  name: {name}$", text, re.MULTILINE):
            text = re.sub(
                r"validationActions:\n    - \w+", f"validationActions:\n    - {actions}", text
            )
        documents.append(text)
    return "\n---\n".join(documents)


class PolicyModeTest(unittest.TestCase):
    def test_deny_and_enforce_block_admission(self) -> None:
        self.assertEqual(rcl.policy_mode(policy("p", ["Deny"])), "Deny")
        self.assertEqual(rcl.policy_mode(policy("p", ["Audit", "Deny"])), "Deny")
        legacy = {"kind": "ClusterPolicy", "spec": {"validationFailureAction": "Enforce"}}
        self.assertEqual(rcl.policy_mode(legacy), "Deny")

    def test_audit_and_warn_only_report(self) -> None:
        self.assertEqual(rcl.policy_mode(policy("p", ["Audit"])), "Audit")
        self.assertEqual(rcl.policy_mode(policy("p", ["Audit", "Warn"])), "Audit")

    def test_policy_without_actions_is_named_by_kind(self) -> None:
        self.assertEqual(rcl.policy_mode(policy("p", kind="GeneratingPolicy")), "Generating")


class TallyTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tally = rcl.Tally({"audit": "Audit", "deny": "Deny", "ci": "CI"})

    def test_audit_fail_is_reported_but_not_enforced(self) -> None:
        self.tally.add("base-demo", report(("audit", "fail")))
        self.assertEqual(self.tally.enforced, [])
        self.assertEqual(self.tally.audited, ["base-demo: audit: Pod/demo/app: said so"])

    def test_deny_and_ci_fail_are_enforced(self) -> None:
        self.tally.add("base-demo", report(("deny", "fail"), ("ci", "fail")))
        self.assertEqual(len(self.tally.enforced), 2)

    def test_error_is_enforced_even_in_audit(self) -> None:
        self.tally.add("base-demo", report(("audit", "error")))
        self.assertEqual(len(self.tally.enforced), 1)

    def test_table_lists_modes_and_policies_without_results(self) -> None:
        self.tally.add("base-demo", report(("audit", "fail"), ("audit", "pass")))
        table = self.tally.table()
        self.assertRegex(table, r"audit +Audit +1 +1 +0 +0")
        self.assertRegex(table, r"deny +Deny +0 +0 +0 +0")


class NormalizeTest(unittest.TestCase):
    def test_namespaced_objects_get_the_release_namespace(self) -> None:
        objects = [
            {"kind": "Deployment", "metadata": {"name": "a"}},
            {"kind": "Service", "metadata": {"name": "b", "namespace": "other"}},
            {"kind": "ClusterRole", "metadata": {"name": "c"}},
        ]
        placed = rcl.with_namespace(objects, "release", {"ClusterRole"})
        self.assertEqual([rcl.namespace_of(obj) for obj in placed], ["release", "other", ""])

    def test_release_pins_tagged_images_by_digest(self) -> None:
        pinned = "redis:8@sha256:" + "1" * 64
        job = {
            "kind": "CronJob",
            "spec": {
                "jobTemplate": {
                    "spec": {
                        "template": {
                            "spec": {
                                "initContainers": [{"image": "app:1"}],
                                "containers": [{"image": pinned}],
                            }
                        }
                    }
                }
            },
        }
        pod_spec = rcl.pin_release_digests([job])[0]["spec"]["jobTemplate"]["spec"]["template"]
        self.assertEqual(
            pod_spec["spec"]["initContainers"][0]["image"], "app:1" + rcl.RELEASE_DIGEST
        )
        self.assertEqual(pod_spec["spec"]["containers"][0]["image"], pinned)

    def test_nulls_are_dropped(self) -> None:
        cleaned = rcl.without_nulls({"a": None, "b": [None, {"c": None, "d": 1}]})
        self.assertEqual(cleaned, {"b": [{"d": 1}]})

    def test_cluster_scoped_crd_kinds_are_found(self) -> None:
        crd = (
            "apiVersion: apiextensions.k8s.io/v1\nkind: CustomResourceDefinition\n"
            "metadata:\n  name: issuers.example.io\nspec:\n  names:\n    kind: ClusterIssuer\n"
            "  scope: Cluster\n"
        )
        documents, kinds = rcl.crd_documents([crd, crd])
        self.assertEqual(len(documents), 1)
        self.assertEqual(kinds, {"ClusterIssuer"})


class FakeKyverno:
    def __init__(self, output: str = "", code: int = 0) -> None:
        self.output = output
        self.code = code

    def __call__(self, command: list[str]) -> tuple[int, str]:
        return self.code, self.output


def inputs(objects: list[dict]) -> rcl.Inputs:
    return rcl.Inputs(
        [rcl.Universe("base-demo", objects)], [policy("deny", ["Deny"])], [], [], {}, []
    )


class ApplyGuardTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.work = Path(self.tmp.name)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_no_result_at_all_is_a_problem(self) -> None:
        _, problems = rcl.apply_policies(inputs([]), POLICIES, FakeKyverno(), self.work)
        self.assertEqual(problems, ["no rendered object produced a cluster policy result"])

    def test_crash_without_report_is_a_problem(self) -> None:
        pod = {"kind": "Pod", "metadata": {"name": "app"}}
        _, problems = rcl.apply_policies(
            inputs([pod]), POLICIES, FakeKyverno("panic", 2), self.work
        )
        self.assertEqual(len(problems), 1)
        self.assertIn("without a report", problems[0])

    def test_nonzero_exit_with_clean_report_is_a_problem(self) -> None:
        pod = {"kind": "Pod", "metadata": {"name": "app"}}
        output = json.dumps({"summary": {"pass": 1}, **report(("deny", "pass"))})
        _, problems = rcl.apply_policies(inputs([pod]), POLICIES, FakeKyverno(output, 1), self.work)
        self.assertEqual(len(problems), 1)


class ParseReportTest(unittest.TestCase):
    def test_report_after_generated_yaml_is_found(self) -> None:
        output = "policy generate applied:\nspec:\n  podSelector: {}\n" + json.dumps(
            {"summary": {"pass": 1}, "results": []}
        )
        self.assertEqual(rcl.parse_report(output), {"summary": {"pass": 1}, "results": []})

    def test_output_without_report_is_none(self) -> None:
        self.assertIsNone(rcl.parse_report("podSelector: {}"))


class ApplierTest(unittest.TestCase):
    def test_platform_flux_applies_overlays_and_bases(self) -> None:
        for universe in ("base-kyverno", "overlay-local"):
            self.assertEqual(
                rcl.applier(universe), "system:serviceaccount:flux-system:kustomize-controller"
            )

    def test_tenant_flux_applies_the_app_chart(self) -> None:
        self.assertEqual(
            rcl.applier("app-all-options"),
            f"system:serviceaccount:{rcl.APP_NAMESPACE}:flux-reconciler",
        )

    def test_apply_passes_the_userinfo_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            userinfo = rcl.write_userinfo(rcl.applier("overlay-local") or "", work)
            paths = {key: work / key for key in ("policies", "crds", "values", "context")}
            command = rcl.apply_command(paths, work, work / "r.yaml", userinfo)
            body = json.loads(userinfo.read_text())
        self.assertEqual(command[-2:], ["--userinfo", str(userinfo)])
        self.assertIn("system:serviceaccounts:flux-system", body["userInfo"]["groups"])


class KyvernoStageTest(unittest.TestCase):
    """Runs the pinned kyverno and helm from .tools/bin, as validate.sh puts them on PATH."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.rendered = root / "rendered"
        self.work = root / "work"
        self.rendered.mkdir()
        self.work.mkdir()

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def stage(self, pod: str, actions: str, *extra: str) -> rcl.Tally:
        policies = policies_with("require-container-resources", actions)
        (self.rendered / "base-kyverno-policies.yaml").write_text(
            "\n---\n".join([policies, ALLOWLIST, *extra])
        )
        (self.rendered / "base-demo.yaml").write_text(NAMESPACE + "---\n" + pod)
        stage_inputs = rcl.collect_inputs(self.rendered, {}, [], rcl.run_ci_policies.run_command)
        stage_inputs.universes = [
            rcl.Universe(
                universe.name, rcl.with_namespace(universe.objects, "demo", rcl.CLUSTER_SCOPED)
            )
            for universe in stage_inputs.universes
            if universe.name == "base-demo"
        ]
        tally, problems = rcl.apply_policies(
            stage_inputs, rcl.HERE / "policies", rcl.run_ci_policies.run_command, self.work
        )
        self.assertEqual(problems, [])
        return tally

    def resource_lines(self, lines: list[str]) -> list[str]:
        return [line for line in lines if "require-container-resources: Pod/demo/app" in line]

    def test_pod_with_limits_passes(self) -> None:
        tally = self.stage(POD_WITH_LIMITS, "Deny")
        self.assertEqual(tally.counts["require-container-resources"]["pass"], 1)
        self.assertEqual(self.resource_lines(tally.enforced), [])

    def test_pod_without_limits_fails_the_stage_under_deny(self) -> None:
        tally = self.stage(POD_WITHOUT_LIMITS, "Deny")
        self.assertEqual(len(self.resource_lines(tally.enforced)), 1)

    def test_pod_without_limits_is_reported_under_audit(self) -> None:
        tally = self.stage(POD_WITHOUT_LIMITS, "Audit")
        self.assertEqual(self.resource_lines(tally.enforced), [])
        self.assertEqual(len(self.resource_lines(tally.audited)), 1)

    def test_policy_exception_from_the_rendered_base_skips_the_fail(self) -> None:
        tally = self.stage(POD_WITHOUT_LIMITS, "Deny", EXCEPTION)
        self.assertEqual(self.resource_lines(tally.enforced), [])
        self.assertEqual(tally.counts["require-container-resources"]["skip"], 1)


class AppProfileTest(unittest.TestCase):
    def test_generated_values_match_the_blessed_platform_snapshot(self) -> None:
        values = rcl.APP_PROFILES["generated-platform"]
        digest = hashlib.sha256(values.read_bytes()).hexdigest()
        match = re.search(r"^(\w+)  deploy/values.yaml$", SNAPSHOT_TREE.read_text(), re.MULTILINE)
        self.assertIsNotNone(match)
        self.assertEqual(
            digest,
            match.group(1) if match else "",
            "templates/backend/deploy/values.yaml changed: copy the platform flavor's generated "
            f"deploy/values.yaml to {values}",
        )

    def test_every_profile_exists(self) -> None:
        for values in rcl.APP_PROFILES.values():
            self.assertTrue(values.is_file(), values)

    def test_tenant_component_labels_the_namespace(self) -> None:
        labels = rcl.tenant_labels(rcl.TENANT_COMPONENT)
        self.assertEqual(labels["baukit.dev/tenant"], "true")
        self.assertEqual(labels["pod-security.kubernetes.io/enforce"], "restricted")


class SuitesTest(unittest.TestCase):
    def test_every_in_cluster_policy_has_a_pass_and_fail_suite(self) -> None:
        names = rcl.run_ci_policies.policy_names(POLICIES)
        self.assertGreaterEqual(len(names), 20)
        for name in names:
            suite = rcl.PLATFORM / "kyverno-policies/tests" / name / "kyverno-test.yaml"
            self.assertTrue(suite.is_file(), name)
            text = suite.read_text()
            if name != "generate-tenant-defaults":
                self.assertIn("result: pass", text, name)
                self.assertIn("result: fail", text, name)


if __name__ == "__main__":
    unittest.main()
