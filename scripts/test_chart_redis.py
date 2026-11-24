from __future__ import annotations

import os
import re
import tempfile
import unittest
from pathlib import Path

import yaml
from process_runner import run_process

CHART = Path(__file__).resolve().parent.parent / "deploy/chart/baukit-app"
REDIS_DIGEST = "sha256:7958cfbb9d152f87dadfbc89f1b0b15938f0b81ff8982feb327476f4911935c3"
REDIS_IMAGE = f"redis:8.10.3-alpine@{REDIS_DIGEST}"


def render(replicas: int) -> str:
    return run_process(
        [
            "helm",
            "template",
            "redis-image-test",
            str(CHART),
            "-f",
            str(CHART / "tests/images-values.yaml"),
            "--set",
            "redis.enabled=true",
            "--set",
            f"redis.replicas={replicas}",
        ],
        capture_output=True,
        text=True,
        check=True,
    ).stdout


def sentinel_script(rendered: str) -> str:
    for document in yaml.safe_load_all(rendered):
        if document and document["kind"] == "StatefulSet":
            containers = document["spec"]["template"]["spec"]["containers"]
            return next(c for c in containers if c["name"] == "sentinel")["args"][0]
    raise AssertionError("no Redis StatefulSet rendered")


class RedisChartImageTest(unittest.TestCase):
    def test_direct_and_sentinel_modes_use_the_documented_patch(self) -> None:
        for replicas, expected_count in [(1, 1), (3, 2)]:
            with self.subTest(replicas=replicas):
                images = re.findall(r'image: ["\']?(redis[:@][^"\'\s]+)', render(replicas))
                self.assertEqual(images, [REDIS_IMAGE] * expected_count)
        readme = (CHART / "README.md").read_text()
        self.assertIn(f"`redis` / `8.10.3-alpine` / `{REDIS_DIGEST}`", readme)


class RedisChartSentinelTest(unittest.TestCase):
    def sentinel_config(self, script: str, hostname: str) -> str:
        with tempfile.TemporaryDirectory() as tmp:
            stub = Path(tmp, "redis-sentinel")
            stub.write_text("#!/bin/sh\nexit 0\n")
            stub.chmod(0o755)
            config = Path(tmp, "sentinel.conf")
            env = {**os.environ, "HOSTNAME": hostname, "PATH": f"{tmp}:{os.environ['PATH']}"}
            local_script = script.replace("/sentinel/sentinel.conf", str(config))
            run_process(["sh", "-ec", local_script], env=env, check=True)
            return config.read_text()

    def test_each_sentinel_waits_a_different_time_before_it_calls_the_master_down(self) -> None:
        script = sentinel_script(render(3))
        delays = []
        for ordinal in range(3):
            config = self.sentinel_config(script, f"redis-image-test-redis-{ordinal}")
            self.assertIn("sentinel failover-timeout mymaster 10000", config)
            delays += re.findall(r"^sentinel down-after-milliseconds mymaster (\d+)$", config, re.M)
        self.assertEqual(delays, ["5000", "7000", "9000"])


if __name__ == "__main__":
    unittest.main()
