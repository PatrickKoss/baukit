from __future__ import annotations

import unittest
from pathlib import Path, PurePosixPath

import yaml
from process_runner import run_process

CHART = Path(__file__).resolve().parents[1] / "deploy/chart/baukit-app"
ALL_OPTIONS = CHART / "tests/all-options-values.yaml"
RELEASE = "security-test"
WORKLOAD_KINDS = {"Deployment", "StatefulSet", "Job"}


def render(*args: str) -> list[dict]:
    result = run_process(
        ["helm", "template", RELEASE, str(CHART), "-f", str(ALL_OPTIONS), *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    return [doc for doc in yaml.safe_load_all(result.stdout) if doc]


def workloads(docs: list[dict]) -> dict[str, dict]:
    return {
        doc["metadata"]["labels"]["baukit.dev/process"]: doc
        for doc in docs
        if doc["kind"] in WORKLOAD_KINDS
    }


def pod_spec(workload: dict) -> dict:
    return workload["spec"]["template"]["spec"]


def containers(workload: dict) -> list[dict]:
    spec = pod_spec(workload)
    return [*spec.get("initContainers", []), *spec["containers"]]


def restricted_violations(spec: dict, container: dict) -> list[str]:
    pod = spec.get("securityContext", {})
    own = container.get("securityContext", {})
    problems = []
    if not own.get("runAsNonRoot", pod.get("runAsNonRoot")):
        problems.append("runAsNonRoot")
    seccomp = own.get("seccompProfile", pod.get("seccompProfile", {}))
    if seccomp.get("type") not in {"RuntimeDefault", "Localhost"}:
        problems.append("seccompProfile")
    if own.get("allowPrivilegeEscalation") is not False:
        problems.append("allowPrivilegeEscalation")
    if own.get("capabilities", {}).get("drop") != ["ALL"]:
        problems.append("capabilities.drop")
    if own.get("readOnlyRootFilesystem") is not True:
        problems.append("readOnlyRootFilesystem")
    if own.get("privileged"):
        problems.append("privileged")
    return problems


class SecurityContextTest(unittest.TestCase):
    def assert_restricted(self, docs: list[dict]) -> None:
        for process, workload in workloads(docs).items():
            for container in containers(workload):
                with self.subTest(process=process, container=container["name"]):
                    self.assertEqual(restricted_violations(pod_spec(workload), container), [])

    def test_every_container_meets_restricted(self) -> None:
        docs = render("--set", "redis.enabled=true")
        self.assertEqual(set(workloads(docs)), {"api", "worker", "migrate", "seed", "web", "redis"})
        self.assert_restricted(docs)

    def test_sentinel_containers_meet_restricted(self) -> None:
        self.assert_restricted(render("--set", "redis.enabled=true", "--set", "redis.replicas=3"))

    def test_the_check_bites_on_a_weakened_context(self) -> None:
        docs = render("--set", "containerSecurityContext.allowPrivilegeEscalation=true")
        api = workloads(docs)["api"]
        self.assertEqual(
            restricted_violations(pod_spec(api), containers(api)[0]), ["allowPrivilegeEscalation"]
        )

    def test_every_pod_uses_the_release_service_account_without_a_token(self) -> None:
        docs = render("--set", "redis.enabled=true")
        accounts = [doc for doc in docs if doc["kind"] == "ServiceAccount"]
        self.assertEqual(len(accounts), 1)
        self.assertIs(accounts[0]["automountServiceAccountToken"], False)
        name = accounts[0]["metadata"]["name"]
        for process, workload in workloads(docs).items():
            with self.subTest(process=process):
                self.assertEqual(pod_spec(workload)["serviceAccountName"], name)
                self.assertIs(pod_spec(workload)["automountServiceAccountToken"], False)

    def test_an_existing_service_account_must_be_named(self) -> None:
        with self.assertRaisesRegex(AssertionError, "serviceAccount.name is required"):
            render("--set", "serviceAccount.create=false")
        docs = render("--set", "serviceAccount.create=false", "--set", "serviceAccount.name=shared")
        self.assertFalse([doc for doc in docs if doc["kind"] == "ServiceAccount"])
        self.assertEqual(pod_spec(workloads(docs)["api"])["serviceAccountName"], "shared")

    def test_application_processes_mount_a_writable_tmp(self) -> None:
        for process, workload in workloads(render()).items():
            with self.subTest(process=process):
                volumes = {v["name"]: v for v in pod_spec(workload)["volumes"]}
                self.assertEqual(volumes["tmp"], {"name": "tmp", "emptyDir": {}})
                mounts = {
                    m["name"]: m["mountPath"] for m in containers(workload)[0]["volumeMounts"]
                }
                self.assertEqual(PurePosixPath(mounts["tmp"]).parts, ("/", "tmp"))


class LifecycleTest(unittest.TestCase):
    def test_api_and_worker_have_startup_probes(self) -> None:
        found = workloads(render())
        for process in ["api", "worker"]:
            with self.subTest(process=process):
                probe = containers(found[process])[0]["startupProbe"]
                self.assertEqual(probe["httpGet"], {"path": "/healthz", "port": "ops"})
                self.assertEqual(probe["failureThreshold"], 30)

    def test_startup_probe_can_be_disabled(self) -> None:
        api = workloads(render("--set", "api.startupProbe.enabled=false"))["api"]
        self.assertNotIn("startupProbe", containers(api)[0])

    def test_api_sleeps_before_stop_and_drains_in_the_rest_of_the_grace_period(self) -> None:
        api = containers(workloads(render())["api"])[0]
        self.assertEqual(api["lifecycle"]["preStop"], {"sleep": {"seconds": 5}})
        env = {e["name"]: e.get("value") for e in api["env"]}
        self.assertEqual(env["CHART_OPTIONS__SHUTDOWN__DRAIN_TIMEOUT"], "25")

    def test_pre_stop_sleep_must_stay_below_the_grace_period(self) -> None:
        with self.assertRaisesRegex(AssertionError, "api.preStopSleepSeconds must be lower"):
            render("--set", "api.preStopSleepSeconds=30")

    def test_serving_processes_spread_softly_across_nodes(self) -> None:
        found = workloads(render())
        for process in ["api", "worker", "web"]:
            with self.subTest(process=process):
                [constraint] = pod_spec(found[process])["topologySpreadConstraints"]
                self.assertEqual(constraint["topologyKey"], "kubernetes.io/hostname")
                self.assertEqual(constraint["whenUnsatisfiable"], "ScheduleAnyway")
                self.assertEqual(
                    constraint["labelSelector"]["matchLabels"],
                    found[process]["spec"]["selector"]["matchLabels"],
                )
        for process in ["migrate", "seed"]:
            self.assertNotIn("topologySpreadConstraints", pod_spec(found[process]))

    def test_spread_can_be_disabled(self) -> None:
        api = workloads(render("--set", "topologySpread.enabled=false"))["api"]
        self.assertNotIn("topologySpreadConstraints", pod_spec(api))

    def test_release_jobs_expire(self) -> None:
        found = workloads(render())
        for process in ["migrate", "seed"]:
            with self.subTest(process=process):
                self.assertEqual(found[process]["spec"]["ttlSecondsAfterFinished"], 86400)


if __name__ == "__main__":
    unittest.main()
