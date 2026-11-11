from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml
from process_runner import run_process

CHART = Path(__file__).resolve().parent.parent / "deploy/chart/baukit-app"
POLICY = "default-src 'self'; script-src 'self'; style-src 'self'; frame-ancestors 'none'"


def render(*args: str) -> subprocess.CompletedProcess[str]:
    return run_process(
        ["helm", "template", "web-test", str(CHART), "--set", "web.enabled=true", *args],
        capture_output=True,
        text=True,
        check=False,
    )


class WebChartTest(unittest.TestCase):
    def test_the_build_policy_reaches_nginx_unchanged(self) -> None:
        with tempfile.NamedTemporaryFile("w", suffix=".txt") as policy_file:
            policy_file.write(f"{POLICY}\n")
            policy_file.flush()
            result = render(
                "--set-file",
                f"web.contentSecurityPolicy={policy_file.name}",
                "--set",
                "web.ingress.enabled=true",
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        documents = [doc for doc in yaml.safe_load_all(result.stdout) if doc]
        config = next(
            doc
            for doc in documents
            if doc["kind"] == "ConfigMap" and doc["metadata"]["name"].endswith("-web")
        )
        conf = config["data"]["default.conf"]
        self.assertIn(f'add_header Content-Security-Policy "{POLICY}" always;', conf)
        self.assertIn('add_header X-Content-Type-Options "nosniff" always;', conf)
        self.assertIn('add_header Cross-Origin-Opener-Policy "same-origin" always;', conf)
        self.assertIn('add_header Cross-Origin-Resource-Policy "same-origin" always;', conf)
        self.assertNotIn("Cross-Origin-Embedder-Policy", conf)
        kinds = {
            doc["kind"]
            for doc in documents
            if doc["metadata"]["name"].startswith("web-test-baukit-app-web")
        }
        self.assertEqual(kinds, {"ConfigMap", "Deployment", "Service", "Ingress", "NetworkPolicy"})

    def test_a_missing_policy_fails_the_render(self) -> None:
        result = render()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("web.contentSecurityPolicy is required", result.stderr)

    def test_inline_scripts_and_header_breaking_policies_fail_the_render(self) -> None:
        for policy in ["script-src 'self' 'unsafe-inline'", 'default-src "self"']:
            with self.subTest(policy=policy):
                result = render("--set-string", f"web.contentSecurityPolicy={policy}")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("must be one line", result.stderr)


if __name__ == "__main__":
    unittest.main()
