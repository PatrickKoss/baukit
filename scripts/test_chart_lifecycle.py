from __future__ import annotations

import unittest
from pathlib import Path

import yaml
from process_runner import run_process

CHART = Path(__file__).resolve().parents[1] / "deploy/chart/baukit-app"
ALL_OPTIONS = CHART / "tests/all-options-values.yaml"
RELEASE = "lifecycle-test"
NAMESPACE = "shop"
PREFIX = "CHART_OPTIONS"
FULLNAME = "chart-options"


def render(*settings: str) -> list[dict]:
    command = ["helm", "template", RELEASE, str(CHART), "-n", NAMESPACE, "-f", str(ALL_OPTIONS)]
    for setting in settings:
        command.extend(["--set", setting])
    result = run_process(command, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    return [doc for doc in yaml.safe_load_all(result.stdout) if doc]


def by_kind(docs: list[dict], kind: str) -> dict[str, dict]:
    return {doc["metadata"]["name"]: doc for doc in docs if doc["kind"] == kind}


def deployment(docs: list[dict], process: str) -> dict:
    [found] = [
        doc
        for doc in docs
        if doc["kind"] == "Deployment"
        and doc["metadata"]["labels"]["baukit.dev/process"] == process
    ]
    return found


def container(workload: dict) -> dict:
    return workload["spec"]["template"]["spec"]["containers"][0]


def env(workload: dict) -> dict[str, str]:
    return {entry["name"]: entry.get("value") for entry in container(workload)["env"]}


class DrainBudgetTest(unittest.TestCase):
    def test_api_grace_leaves_drain_above_the_request_timeout(self) -> None:
        api = deployment(render(), "api")
        self.assertEqual(api["spec"]["template"]["spec"]["terminationGracePeriodSeconds"], 45)
        self.assertEqual(env(api)[f"{PREFIX}__SHUTDOWN__DRAIN_TIMEOUT"], "35")

    def test_api_drain_follows_grace_minus_pre_stop_and_slack(self) -> None:
        api = deployment(
            render("api.terminationGracePeriodSeconds=60", "api.preStopSleepSeconds=10"), "api"
        )
        self.assertEqual(env(api)[f"{PREFIX}__SHUTDOWN__DRAIN_TIMEOUT"], "45")

    def test_api_grace_that_leaves_no_drain_fails(self) -> None:
        for grace in ["10", "9"]:
            with self.subTest(grace=grace), self.assertRaisesRegex(AssertionError, "must exceed"):
                render(f"api.terminationGracePeriodSeconds={grace}", "api.preStopSleepSeconds=5")

    def test_worker_drains_fifteen_seconds_before_the_grace_ends(self) -> None:
        worker = deployment(render(), "worker")
        self.assertEqual(worker["spec"]["template"]["spec"]["terminationGracePeriodSeconds"], 60)
        self.assertEqual(env(worker)[f"{PREFIX}__SHUTDOWN__DRAIN_TIMEOUT"], "45")

    def test_worker_grace_without_room_for_the_reserve_is_rejected(self) -> None:
        with self.assertRaisesRegex(AssertionError, "terminationGracePeriodSeconds"):
            render("worker.terminationGracePeriodSeconds=15")

    def test_readiness_timeout_exceeds_the_two_second_check(self) -> None:
        docs = render()
        for process in ["api", "worker"]:
            with self.subTest(process=process):
                probe = container(deployment(docs, process))["readinessProbe"]
                self.assertEqual(probe["timeoutSeconds"], 4)


class WebLifecycleTest(unittest.TestCase):
    def test_web_sleeps_before_stop_and_has_a_startup_probe(self) -> None:
        web = deployment(render(), "web")
        self.assertEqual(web["spec"]["template"]["spec"]["terminationGracePeriodSeconds"], 30)
        nginx = container(web)
        self.assertEqual(nginx["lifecycle"]["preStop"], {"sleep": {"seconds": 5}})
        self.assertEqual(nginx["startupProbe"]["httpGet"], {"path": "/healthz", "port": "http"})

    def test_web_startup_probe_can_be_disabled(self) -> None:
        web = deployment(render("web.startupProbe.enabled=false"), "web")
        self.assertNotIn("startupProbe", container(web))

    def test_web_pre_stop_must_stay_below_the_grace_period(self) -> None:
        with self.assertRaisesRegex(AssertionError, "web.preStopSleepSeconds"):
            render("web.preStopSleepSeconds=30")


class AutoscalingTest(unittest.TestCase):
    def test_hpa_keeps_two_pods_and_damps_scaling(self) -> None:
        [hpa] = by_kind(render("autoscaling.enabled=true"), "HorizontalPodAutoscaler").values()
        spec = hpa["spec"]
        self.assertEqual(spec["minReplicas"], 2)
        self.assertEqual(spec["behavior"]["scaleDown"], {"stabilizationWindowSeconds": 300})
        self.assertEqual(
            spec["behavior"]["scaleUp"]["policies"],
            [{"type": "Percent", "value": 100, "periodSeconds": 60}],
        )


class RetryMiddlewareTest(unittest.TestCase):
    MIDDLEWARE = f"{FULLNAME}-retry"

    def test_api_ingress_retries_through_a_traefik_middleware(self) -> None:
        docs = render("ingress.enabled=true")
        middleware = by_kind(docs, "Middleware")[self.MIDDLEWARE]
        self.assertEqual(middleware["apiVersion"], "traefik.io/v1alpha1")
        self.assertEqual(middleware["spec"], {"retry": {"attempts": 2, "initialInterval": "100ms"}})
        ingresses = by_kind(docs, "Ingress")
        api = ingresses[FULLNAME]["metadata"]["annotations"]
        chain = api["traefik.ingress.kubernetes.io/router.middlewares"].split(",")
        self.assertEqual(chain[-1], f"{NAMESPACE}-{self.MIDDLEWARE}@kubernetescrd")
        web = ingresses[f"{FULLNAME}-web"]["metadata"].get("annotations", {})
        self.assertNotIn(
            self.MIDDLEWARE, web.get("traefik.ingress.kubernetes.io/router.middlewares", "")
        )

    def test_rate_limit_runs_before_the_retry(self) -> None:
        docs = render(
            "ingress.enabled=true", "ingress.rateLimitMiddleware=shop-limit@kubernetescrd"
        )
        api = by_kind(docs, "Ingress")[FULLNAME]["metadata"]["annotations"]
        self.assertEqual(
            api["traefik.ingress.kubernetes.io/router.middlewares"],
            f"shop-limit@kubernetescrd,{NAMESPACE}-{self.MIDDLEWARE}@kubernetescrd",
        )

    def test_retry_can_be_disabled(self) -> None:
        docs = render("ingress.enabled=true", "ingress.retry.enabled=false")
        self.assertEqual(by_kind(docs, "Middleware"), {})

    def test_no_ingress_means_no_middleware(self) -> None:
        self.assertEqual(by_kind(render("ingress.enabled=false"), "Middleware"), {})


if __name__ == "__main__":
    unittest.main()
