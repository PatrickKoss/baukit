from __future__ import annotations

import unittest
from pathlib import Path

import yaml
from process_runner import run_process

CHART = Path(__file__).resolve().parents[1] / "deploy/chart/baukit-app"
IMAGES = CHART / "tests/images-values.yaml"
RELEASE = "backing"
URL = "MINIMAL_API__RATE_LIMIT__REDIS_URL"
NAMESPACE = "MINIMAL_API__RATE_LIMIT__REDIS_KEY_NAMESPACE"
CLIENT_LABEL = "baukit.dev/redis-client"
PLATFORM_REDIS = {"kubernetes.io/metadata.name": "platform-redis"}


def render(mode: str, *args: str) -> list[dict]:
    result = run_process(
        [
            "helm",
            "template",
            RELEASE,
            str(CHART),
            "-f",
            str(IMAGES),
            "--set",
            f"backing.redis.mode={mode}",
            "--set",
            "worker.enabled=true",
            "--set",
            "seed.enabled=true",
            *args,
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    return [document for document in yaml.safe_load_all(result.stdout) if document]


def pods(documents: list[dict]) -> dict[str, dict]:
    templates = {}
    for document in documents:
        if document["kind"] not in ("Deployment", "Job"):
            continue
        template = document["spec"]["template"]
        process = template["metadata"]["labels"].get("baukit.dev/process")
        if process != "redis":
            templates[process] = template
    return templates


def env_names(template: dict) -> set[str]:
    [container] = template["spec"]["containers"]
    return {entry["name"] for entry in container.get("env", [])}


def policies(documents: list[dict]) -> dict[str, dict]:
    return {
        document["metadata"]["name"]: document["spec"]
        for document in documents
        if document["kind"] == "NetworkPolicy"
    }


def processes_of(peers: list[dict]) -> list[str]:
    return [peer["podSelector"]["matchLabels"]["baukit.dev/process"] for peer in peers]


def reaches_platform_redis(spec: dict | None) -> bool:
    if spec is None:
        return False
    return any(
        peer.get("namespaceSelector", {}).get("matchLabels") == PLATFORM_REDIS
        for rule in spec["egress"]
        for peer in rule["to"]
    )


class DedicatedRedisTest(unittest.TestCase):
    def test_worker_gets_the_url_namespace_and_network_path_by_default(self) -> None:
        documents = render("dedicated")
        templates = pods(documents)
        for process in ("api", "worker"):
            with self.subTest(process=process):
                self.assertLessEqual({URL, NAMESPACE}, env_names(templates[process]))
        for process in ("migrate", "seed"):
            with self.subTest(process=process):
                self.assertFalse({URL, NAMESPACE} & env_names(templates[process]))
        specs = policies(documents)
        [ingress] = specs[f"{RELEASE}-redis-ingress"]["ingress"]
        self.assertEqual(processes_of(ingress["from"]), ["api", "worker"])
        for process in ("api", "worker"):
            with self.subTest(policy=f"{process}-egress"):
                spec = specs[f"{RELEASE}-redis-{process}-egress"]
                self.assertEqual(spec["podSelector"]["matchLabels"]["baukit.dev/process"], process)
                [rule] = spec["egress"]
                self.assertEqual(processes_of(rule["to"]), ["redis"])
                self.assertEqual(rule["ports"], [{"port": 6379, "protocol": "TCP"}])

    def test_sentinel_mode_opens_both_ports_to_the_worker(self) -> None:
        specs = policies(render("dedicated", "--set", "redis.replicas=3"))
        [rule] = specs[f"{RELEASE}-redis-worker-egress"]["egress"]
        self.assertEqual(
            rule["ports"],
            [{"port": 6379, "protocol": "TCP"}, {"port": 26379, "protocol": "TCP"}],
        )

    def test_switch_keeps_the_worker_off_redis(self) -> None:
        documents = render("dedicated", "--set", "worker.redis.enabled=false")
        templates = pods(documents)
        self.assertLessEqual({URL, NAMESPACE}, env_names(templates["api"]))
        self.assertFalse({URL, NAMESPACE} & env_names(templates["worker"]))
        specs = policies(documents)
        [ingress] = specs[f"{RELEASE}-redis-ingress"]["ingress"]
        self.assertEqual(processes_of(ingress["from"]), ["api"])
        self.assertNotIn(f"{RELEASE}-redis-worker-egress", specs)

    def test_disabled_worker_opens_nothing(self) -> None:
        specs = policies(render("dedicated", "--set", "worker.enabled=false"))
        [ingress] = specs[f"{RELEASE}-redis-ingress"]["ingress"]
        self.assertEqual(processes_of(ingress["from"]), ["api"])
        self.assertNotIn(f"{RELEASE}-redis-worker-egress", specs)


class SharedRedisTest(unittest.TestCase):
    SECRET = ["--set", "config.existingSecretRefs[0].name=minimal-api-redis"]

    def test_worker_gets_the_label_namespace_secret_and_egress_by_default(self) -> None:
        documents = render("shared", *self.SECRET)
        templates = pods(documents)
        specs = policies(documents)
        for process in ("api", "worker"):
            with self.subTest(process=process):
                template = templates[process]
                self.assertEqual(template["metadata"]["labels"].get(CLIENT_LABEL), "true")
                self.assertIn(NAMESPACE, env_names(template))
                self.assertNotIn(URL, env_names(template))
                [container] = template["spec"]["containers"]
                self.assertIn({"secretRef": {"name": "minimal-api-redis"}}, container["envFrom"])
                self.assertTrue(
                    reaches_platform_redis(
                        specs.get(f"{RELEASE}-baukit-app-{process}-backing-egress")
                    )
                )
        for process in ("migrate", "seed"):
            with self.subTest(process=process):
                self.assertNotIn(CLIENT_LABEL, templates[process]["metadata"]["labels"])
                self.assertFalse(
                    reaches_platform_redis(
                        specs.get(f"{RELEASE}-baukit-app-{process}-backing-egress")
                    )
                )

    def test_switch_keeps_the_worker_off_redis(self) -> None:
        documents = render("shared", *self.SECRET, "--set", "worker.redis.enabled=false")
        worker = pods(documents)["worker"]
        self.assertNotIn(CLIENT_LABEL, worker["metadata"]["labels"])
        self.assertNotIn(NAMESPACE, env_names(worker))
        specs = policies(documents)
        self.assertFalse(
            reaches_platform_redis(specs.get(f"{RELEASE}-baukit-app-worker-backing-egress"))
        )
        self.assertTrue(
            reaches_platform_redis(specs.get(f"{RELEASE}-baukit-app-api-backing-egress"))
        )


if __name__ == "__main__":
    unittest.main()
