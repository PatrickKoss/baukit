from __future__ import annotations

import importlib.util
import re
import unittest
from pathlib import Path

PLATFORM = Path(__file__).resolve().parent.parent
LINT = Path(__file__).resolve().parent
RELEASE = PLATFORM / "kyverno" / "helm-release.yaml"
DURATION = re.compile(r"^(\d+)m$")
SECONDS_PER_MINUTE = 60
SLOW_PULL_SECONDS = 300


def load_release() -> dict:
    spec = importlib.util.spec_from_file_location("helm_yaml", LINT / "helm_yaml.py")
    if spec is None or spec.loader is None:
        raise ImportError("helm_yaml.py")
    helm_yaml = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helm_yaml)
    documents = helm_yaml.parse_documents(helm_yaml.split_documents(RELEASE.read_text()))
    return next(doc for doc in documents if doc and doc.get("kind") == "HelmRelease")


class KyvernoInstallTest(unittest.TestCase):
    def setUp(self) -> None:
        self.spec = load_release()["spec"]

    def test_failed_install_is_retried_without_uninstall(self) -> None:
        self.assertEqual(self.spec["install"]["strategy"]["name"], "RetryOnFailure")

    def test_webhook_cleanup_hooks_stay_enabled_for_real_uninstalls(self) -> None:
        self.assertFalse(self.spec.get("uninstall", {}).get("disableHooks", False))
        self.assertTrue(self.spec["values"].get("webhooksCleanup", {}).get("enabled", True))

    def test_legacy_check_deadline_covers_a_slow_pull_and_fits_the_timeout(self) -> None:
        deadline = self.spec["values"]["upgrade"]["legacyPolicyCheck"]["activeDeadlineSeconds"]
        timeout = DURATION.match(self.spec["timeout"])
        self.assertIsNotNone(timeout)
        self.assertGreaterEqual(deadline, SLOW_PULL_SECONDS)
        self.assertLess(deadline, int(timeout.group(1)) * SECONDS_PER_MINUTE)


if __name__ == "__main__":
    unittest.main()
