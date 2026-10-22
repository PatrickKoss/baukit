from __future__ import annotations

import subprocess
import unittest
from pathlib import Path

CHART = Path(__file__).resolve().parents[1] / "deploy/chart/baukit-app"


class PlatformChartTest(unittest.TestCase):
    def render(self, *settings: str) -> subprocess.CompletedProcess:
        command = ["helm", "template", "platform-test", str(CHART)]
        for setting in settings:
            command.extend(["--set", setting])
        return subprocess.run(command, capture_output=True, text=True, check=False)

    def test_receiver_secret_is_opt_in_and_api_only(self) -> None:
        disabled = self.render()
        self.assertEqual(disabled.returncode, 0, disabled.stderr)
        self.assertNotIn("PLATFORM__RECEIVER_SECRET", disabled.stdout)
        enabled = self.render("worker.enabled=true", "seed.enabled=true",
                              "config.envPrefix=MY_PRODUCT",
                              "platform.receiverSecretRef.name=hub-receiver",
                              "platform.receiverSecretRef.key=current",
                              "platform.receiverSecretRef.previousKey=previous")
        self.assertEqual(enabled.returncode, 0, enabled.stderr)
        self.assertEqual(enabled.stdout.count("MY_PRODUCT__PLATFORM__RECEIVER_SECRET"), 1)
        self.assertEqual(enabled.stdout.count("MY_PRODUCT__PLATFORM__PREVIOUS_RECEIVER_SECRET"), 1)
        api = next(doc for doc in enabled.stdout.split("---")
                   if "kind: Deployment" in doc and "name: platform-test-baukit-app-api" in doc)
        self.assertIn('name: "hub-receiver"', api)
        self.assertIn('key: "current"', api)
        self.assertIn('key: "previous"', api)

    def test_previous_key_is_optional_and_current_key_is_required(self) -> None:
        current = self.render("platform.receiverSecretRef.name=hub-receiver")
        self.assertEqual(current.returncode, 0, current.stderr)
        self.assertIn('key: "receiver-secret"', current.stdout)
        self.assertNotIn("PREVIOUS_RECEIVER_SECRET", current.stdout)
        invalid = self.render("platform.receiverSecretRef.name=hub-receiver",
                              "platform.receiverSecretRef.key=")
        self.assertNotEqual(invalid.returncode, 0)
        self.assertIn("platform.receiverSecretRef.key is required", invalid.stderr)


if __name__ == "__main__":
    unittest.main()
