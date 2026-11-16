from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from process_runner import run_process

CHART = Path(__file__).resolve().parents[1] / "deploy/chart/baukit-app"
IMAGES = CHART / "tests/images-values.yaml"
TMP_VOLUME = {"name": "tmp", "emptyDir": {}}
TMP_MOUNT_VALUES = CHART / "tests/tmp-mount-values.yaml"
CACHE_VOLUME = {"name": "cache", "emptyDir": {}}
CACHE_MOUNT = {"name": "cache", "mountPath": "/var/cache/app"}


class TmpVolumeChartTest(unittest.TestCase):
    def render(self, values: dict, *files: Path) -> subprocess.CompletedProcess:
        with tempfile.NamedTemporaryFile("w", suffix=".yaml") as handle:
            json.dump(values, handle)
            handle.flush()
            command = ["helm", "template", "tmp-test", str(CHART), "-f", str(IMAGES)]
            command += ["-f", handle.name]
            for file in files:
                command += ["-f", str(file)]
            return run_process(command, capture_output=True, text=True, check=False)

    def process_values(self, process: str, **extra: list) -> dict:
        values: dict = {process: extra}
        if process in {"worker", "seed"}:
            values[process]["enabled"] = True
        return values

    def test_a_values_tmp_volume_fails_for_every_process(self) -> None:
        for process in ("api", "worker", "seed", "migration"):
            with self.subTest(process=process):
                result = self.render(self.process_values(process, volumes=[TMP_VOLUME]))
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(
                    f"{process}.volumes must not define a volume named tmp", result.stderr
                )

    def test_a_values_tmp_mount_fails(self) -> None:
        result = self.render({}, TMP_MOUNT_VALUES)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("api.volumeMounts must not mount tmp or /tmp", result.stderr)

    def test_other_volumes_render_with_one_tmp_entry(self) -> None:
        values = self.process_values("api", volumes=[CACHE_VOLUME], volumeMounts=[CACHE_MOUNT])
        result = self.render(values)
        self.assertEqual(result.returncode, 0, result.stderr)
        api = next(
            doc
            for doc in result.stdout.split("---")
            if "name: tmp-test-baukit-app-api\n" in doc and "kind: Deployment" in doc
        )
        self.assertEqual(api.count("- name: tmp\n"), 2)
        self.assertIn("mountPath: /var/cache/app", api)


if __name__ == "__main__":
    unittest.main()
