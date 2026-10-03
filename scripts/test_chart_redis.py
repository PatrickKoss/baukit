from __future__ import annotations

import re
import subprocess
import unittest
from pathlib import Path

CHART = Path(__file__).resolve().parent.parent / "deploy/chart/baukit-app"
REDIS_IMAGE = "redis:8.10.2-alpine"


class RedisChartImageTest(unittest.TestCase):
    def test_direct_and_sentinel_modes_use_the_documented_patch(self) -> None:
        for replicas, expected_count in [(1, 1), (3, 2)]:
            with self.subTest(replicas=replicas):
                result = subprocess.run(
                    [
                        "helm", "template", "redis-image-test", str(CHART),
                        "--set", "redis.enabled=true",
                        "--set", f"redis.replicas={replicas}",
                    ],
                    capture_output=True,
                    text=True,
                    check=True,
                )
                images = re.findall(r'image: ["\']?(redis:[^"\'\s]+)', result.stdout)
                self.assertEqual(images, [REDIS_IMAGE] * expected_count)
        self.assertIn("`redis` / `8.10.2-alpine`", (CHART / "README.md").read_text())


if __name__ == "__main__":
    unittest.main()
