from __future__ import annotations

import re
import unittest
from pathlib import Path

from process_runner import run_process

CHART = Path(__file__).resolve().parent.parent / "deploy/chart/baukit-app"
REDIS_DIGEST = "sha256:7958cfbb9d152f87dadfbc89f1b0b15938f0b81ff8982feb327476f4911935c3"
REDIS_IMAGE = f"redis:8.10.3-alpine@{REDIS_DIGEST}"


class RedisChartImageTest(unittest.TestCase):
    def test_direct_and_sentinel_modes_use_the_documented_patch(self) -> None:
        for replicas, expected_count in [(1, 1), (3, 2)]:
            with self.subTest(replicas=replicas):
                result = run_process(
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
                )
                images = re.findall(r'image: ["\']?(redis[:@][^"\'\s]+)', result.stdout)
                self.assertEqual(images, [REDIS_IMAGE] * expected_count)
        readme = (CHART / "README.md").read_text()
        self.assertIn(f"`redis` / `8.10.3-alpine` / `{REDIS_DIGEST}`", readme)


if __name__ == "__main__":
    unittest.main()
