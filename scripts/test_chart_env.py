from __future__ import annotations

import unittest
from pathlib import Path

import yaml
from process_runner import run_process

CHART = Path(__file__).resolve().parents[1] / "deploy/chart/baukit-app"
IMAGES = CHART / "tests/images-values.yaml"


def containers(*settings: str) -> dict[str, dict]:
    command = ["helm", "template", "env-test", str(CHART), "-f", str(IMAGES)]
    for setting in ["worker.enabled=true", "seed.enabled=true", *settings]:
        command.extend(["--set", setting])
    result = run_process(command, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    found = {}
    for doc in yaml.safe_load_all(result.stdout):
        if doc and doc["kind"] in {"Deployment", "Job"}:
            for container in doc["spec"]["template"]["spec"]["containers"]:
                found[container["name"]] = container
    return found


def env(container: dict) -> dict[str, object]:
    return {e["name"]: e.get("value", e.get("valueFrom")) for e in container["env"]}


class EnvContractTest(unittest.TestCase):
    def test_prefix_derives_from_the_product(self) -> None:
        for process, container in containers("product=my-shop.app").items():
            with self.subTest(process=process):
                self.assertEqual(env(container)["MY_SHOP_APP_ENVIRONMENT"], "local")

    def test_explicit_prefix_wins(self) -> None:
        api = env(containers("config.envPrefix=shop")["api"])
        self.assertIn("SHOP_ENVIRONMENT", api)
        self.assertIn("SHOP__HTTP__PORT", api)

    def test_overrides_are_prefixed_and_normalized(self) -> None:
        found = containers("config.overrides.database__max-connections=7")
        for process, container in found.items():
            with self.subTest(process=process):
                self.assertEqual(env(container)["MINIMAL_API__DATABASE__MAX_CONNECTIONS"], "7")

    def test_listener_and_drain_values_follow_the_process(self) -> None:
        found = containers()
        api = env(found["api"])
        self.assertEqual(api["MINIMAL_API__HTTP__PORT"], "8080")
        self.assertEqual(api["MINIMAL_API__OPS__PORT"], "9090")
        worker = env(found["worker"])
        self.assertNotIn("MINIMAL_API__HTTP__PORT", worker)
        self.assertEqual(worker["MINIMAL_API__SHUTDOWN__DRAIN_TIMEOUT"], "45")
        for job in ["migrate", "seed"]:
            self.assertNotIn("MINIMAL_API__OPS__PORT", env(found[job]))

    def test_secrets_arrive_only_as_references(self) -> None:
        found = containers(
            "config.existingSecretRefs[0].name=runtime",
            "platform.receiverSecretRef.name=receiver",
        )
        for process, container in found.items():
            with self.subTest(process=process):
                self.assertEqual(container["envFrom"], [{"secretRef": {"name": "runtime"}}])
        receiver = env(found["api"])["MINIMAL_API__PLATFORM__RECEIVER_SECRET"]
        self.assertEqual(receiver, {"secretKeyRef": {"name": "receiver", "key": "receiver-secret"}})
        self.assertNotIn("MINIMAL_API__PLATFORM__RECEIVER_SECRET", env(found["worker"]))

    def test_process_env_is_appended_after_the_chart_env(self) -> None:
        api = containers("api.env[0].name=EXTRA", "api.env[0].value=on")["api"]
        self.assertEqual(api["env"][-1], {"name": "EXTRA", "value": "on"})

    def test_environment_must_be_known(self) -> None:
        with self.assertRaises(AssertionError):
            containers("deploymentEnvironment=qa")


if __name__ == "__main__":
    unittest.main()
