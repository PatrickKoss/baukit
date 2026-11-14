from __future__ import annotations

import subprocess
import unittest
from pathlib import Path

import yaml
from process_runner import run_process

ROOT = Path(__file__).resolve().parents[1]
CHART = ROOT / "deploy/chart/baukit-app"
IMAGES = CHART / "tests/images-values.yaml"
TEMPLATE_VALUES = ROOT / "templates/backend/deploy/values.yaml"
DIGEST = "sha256:" + "ab" * 32
RELEASE = "values-test"


def helm(*args: str) -> subprocess.CompletedProcess:
    return run_process(["helm", *args], capture_output=True, text=True, check=False)


def render(*args: str) -> subprocess.CompletedProcess:
    return helm("template", RELEASE, str(CHART), "-f", str(IMAGES), *args)


def containers(stdout: str) -> dict[str, dict]:
    found = {}
    for doc in yaml.safe_load_all(stdout):
        if doc and doc["kind"] in {"Deployment", "StatefulSet", "Job"}:
            for container in doc["spec"]["template"]["spec"]["containers"]:
                found[container["name"]] = container
    return found


class SchemaTest(unittest.TestCase):
    def test_unknown_keys_fail_at_every_described_level(self) -> None:
        for key in ["unknownTopLevel", "api.unknown", "api.image.registry", "networkPolicy.extra"]:
            with self.subTest(key=key):
                result = render("--set", f"{key}=1")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("additional properties", result.stderr)

    def test_lint_passes_for_shipped_values(self) -> None:
        for values in [
            CHART / "tests/all-options-values.yaml",
            CHART.parent / "examples/minimal-api-values.yaml",
        ]:
            with self.subTest(values=values.name):
                result = helm("lint", "--strict", str(CHART), "-f", str(IMAGES), "-f", str(values))
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_wrong_types_fail(self) -> None:
        result = render("--set", "deploymentEnvironment=prod")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("/deploymentEnvironment", result.stderr)


class ImageTest(unittest.TestCase):
    def test_latest_fails_the_schema(self) -> None:
        result = render("--set", "api.image.tag=latest")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("/api/image/tag", result.stderr)

    def test_latest_fails_the_template_without_the_schema(self) -> None:
        for process in ["api", "migration"]:
            with self.subTest(process=process):
                result = render("--skip-schema-validation", "--set", f"{process}.image.tag=latest")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"{process}.image.tag must not be latest", result.stderr)

    def test_an_image_needs_a_tag_or_digest(self) -> None:
        result = helm("template", RELEASE, str(CHART))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("image.tag or", result.stderr)
        self.assertIn("image.digest is required", result.stderr)

    def test_digest_wins_over_tag(self) -> None:
        result = render("--set", f"api.image.digest={DIGEST}")
        self.assertEqual(result.returncode, 0, result.stderr)
        found = containers(result.stdout)
        self.assertEqual(found["api"]["image"], f"ghcr.io/example/minimal-api@{DIGEST}")
        self.assertEqual(found["migrate"]["image"], "ghcr.io/example/minimal-migrate:1.0.0")

    def test_malformed_digest_fails(self) -> None:
        result = render("--skip-schema-validation", "--set", "api.image.digest=sha256:abc")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("api.image.digest must look like sha256:", result.stderr)
        self.assertNotEqual(render("--set", "api.image.digest=sha256:abc").returncode, 0)

    def test_template_placeholder_is_pinned_and_unset(self) -> None:
        text = TEMPLATE_VALUES.read_text()
        self.assertNotIn("tag: latest", text)
        self.assertEqual(text.count('tag: "0.0.0-unset"'), 3)


if __name__ == "__main__":
    unittest.main()
