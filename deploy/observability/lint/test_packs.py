"""Tests for check-rules.sh and the pack charts. Need promtool, pint and helm in .tools."""

from __future__ import annotations

import importlib.util
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CHECK_RULES = ROOT / "deploy/observability/lint/check-rules.sh"
APP_PACK = ROOT / "deploy/observability"
PLATFORM_PACK = ROOT / "deploy/observability-platform"
HELM = ROOT / ".tools/bin/helm"

ALERT = """groups:
  - name: example
    rules:
      - alert: BaukitExampleDown
        expr: up == 0
        for: 5m
        labels:
          severity: warning
{component}        annotations:
          summary: Example is down
          description: Example target {{{{ $labels.instance }}}} is down.
          runbook_url: RUNBOOK_BASE_URL/example-down
"""

TEST = """rule_files:
  - ../alerts/example.rules.yml
evaluation_interval: 1m
tests:
  - interval: 1m
    input_series:
      - series: 'up{{instance="a"}}'
        values: '0x10'
    alert_rule_test:
      - eval_time: 10m
        alertname: BaukitExampleDown
        exp_alerts:
          - exp_labels:
              severity: warning
              instance: a
{component_label}            exp_annotations:
              summary: Example is down
              description: Example target a is down.
              runbook_url: RUNBOOK_BASE_URL/example-down
"""


def load_process_runner():
    spec = importlib.util.spec_from_file_location(
        "process_runner", ROOT / "scripts/process_runner.py"
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PROCESS_RUNNER = load_process_runner()


def run(*command: str | Path) -> subprocess.CompletedProcess[str]:
    return PROCESS_RUNNER.run_process(
        [str(part) for part in command], capture_output=True, text=True, cwd=ROOT
    )


class Pack:
    """A throwaway pack directory; its name decides which pint rules match."""

    def __init__(self, test: unittest.TestCase, name: str) -> None:
        directory = tempfile.TemporaryDirectory()
        test.addCleanup(directory.cleanup)
        self.root = Path(directory.name) / name
        (self.root / "alerts").mkdir(parents=True)
        (self.root / "tests").mkdir()

    def write(self, relative: str, content: str) -> None:
        (self.root / relative).write_text(content, encoding="utf-8")

    def write_example(self, component: bool = True, test: bool = True) -> None:
        self.write(
            "alerts/example.rules.yml",
            ALERT.format(component="          component: example\n" if component else ""),
        )
        if test:
            self.write(
                "tests/example.test.yml",
                TEST.format(
                    component_label="              component: example\n" if component else ""
                ),
            )


class CheckRulesTests(unittest.TestCase):
    def test_both_repository_packs_pass(self) -> None:
        result = run(CHECK_RULES, APP_PACK, PLATFORM_PACK)
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)

    def test_a_tested_pack_passes(self) -> None:
        pack = Pack(self, "observability-example")
        pack.write_example()
        result = run(CHECK_RULES, pack.root)
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)

    def test_a_rule_file_without_a_test_fails(self) -> None:
        pack = Pack(self, "observability-example")
        pack.write_example(test=False)
        result = run(CHECK_RULES, pack.root)
        self.assertEqual(result.returncode, 1)
        self.assertIn("example.rules.yml is not loaded by any", result.stderr)

    def test_a_rule_file_no_test_loads_fails(self) -> None:
        pack = Pack(self, "observability-example")
        pack.write_example()
        pack.write("alerts/other.rules.yml", "groups: []\n")
        result = run(CHECK_RULES, pack.root)
        self.assertEqual(result.returncode, 1)
        self.assertIn("other.rules.yml is not loaded by any", result.stderr)

    def test_a_failing_promtool_test_fails(self) -> None:
        pack = Pack(self, "observability-example")
        pack.write_example()
        test_file = pack.root / "tests/example.test.yml"
        test_file.write_text(test_file.read_text().replace("'0x10'", "'1x10'"))
        result = run(CHECK_RULES, pack.root)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("FAILED", result.stdout + result.stderr)

    def test_pint_requires_a_component_label_in_the_platform_pack(self) -> None:
        pack = Pack(self, "observability-platform")
        pack.write_example(component=False)
        result = run(CHECK_RULES, pack.root)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("component", result.stdout + result.stderr)

    def test_pint_rejects_a_severity_outside_the_scheme(self) -> None:
        pack = Pack(self, "observability-example")
        pack.write_example()
        for relative in ("alerts/example.rules.yml", "tests/example.test.yml"):
            path = pack.root / relative
            path.write_text(path.read_text().replace("severity: warning", "severity: page"))
        result = run(CHECK_RULES, pack.root)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("severity", result.stdout + result.stderr)


def helm_template(chart: Path, *arguments: str) -> subprocess.CompletedProcess[str]:
    return run(
        HELM,
        "template",
        "pack",
        chart,
        "--api-versions",
        "monitoring.coreos.com/v1/PrometheusRule",
        *arguments,
    )


class PlatformChartTests(unittest.TestCase):
    def render(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return helm_template(
            PLATFORM_PACK, "--set", "runbookBaseUrl=https://runbooks.test/platform/", *arguments
        )

    def test_one_prometheus_rule_per_rule_file_with_runbook_urls(self) -> None:
        result = self.render()
        self.assertEqual(result.returncode, 0, result.stderr)
        rule_files = sorted((PLATFORM_PACK / "alerts").glob("*.yml"))
        rule_files += sorted((PLATFORM_PACK / "recording-rules").glob("*.yml"))
        self.assertEqual(result.stdout.count("kind: PrometheusRule"), len(rule_files))
        self.assertIn(
            "runbook_url: https://runbooks.test/platform/platform-target-down", result.stdout
        )
        self.assertNotIn("RUNBOOK_BASE_URL", result.stdout)

    def test_a_disabled_component_renders_nothing(self) -> None:
        result = self.render("--set", "components.platform=false")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("kind: PrometheusRule", result.stdout)

    def test_runbook_base_url_is_required(self) -> None:
        result = helm_template(PLATFORM_PACK)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("runbookBaseUrl is required", result.stderr)

    def test_a_file_without_a_component_switch_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            chart = Path(directory) / "chart"
            shutil.copytree(PLATFORM_PACK, chart)
            (chart / "dashboards/flux.json").write_text('{"panels": []}\n', encoding="utf-8")
            result = helm_template(chart, "--set", "runbookBaseUrl=https://runbooks.test")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('component "flux", which is missing', result.stderr)
            result = helm_template(
                chart,
                "--set",
                "runbookBaseUrl=https://runbooks.test",
                "--set",
                "components.flux=true",
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(
                'k8s-sidecar-target-directory: "/tmp/dashboards/baukit-platform/flux"',
                result.stdout,
            )
            self.assertIn("flux.json: |", result.stdout)


APP_RUNBOOKS = "runbookBaseUrl=https://runbooks.test/app/"


class AppChartTests(unittest.TestCase):
    def test_every_pack_file_renders(self) -> None:
        result = helm_template(APP_PACK, "--set", APP_RUNBOOKS)
        self.assertEqual(result.returncode, 0, result.stderr)
        for directory in ("alerts", "recording-rules", "dashboards"):
            for path in (APP_PACK / directory).glob("*.*"):
                if path.suffix in {".yml", ".yaml", ".json"}:
                    with self.subTest(path=path.name):
                        self.assertIn(f"{path.name}: |", result.stdout)

    def test_a_new_rule_file_lands_in_the_prometheus_rule(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            chart = Path(directory) / "chart"
            shutil.copytree(APP_PACK, chart)
            (chart / "alerts/extra.rules.yml").write_text(
                "groups:\n  - name: extra-group\n    rules: []\n", encoding="utf-8"
            )
            result = helm_template(chart, "--set", APP_RUNBOOKS)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("extra.rules.yml: |", result.stdout)
            self.assertIn("- name: extra-group", result.stdout)

    def test_runbook_base_url_is_required(self) -> None:
        result = helm_template(APP_PACK)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("runbookBaseUrl is required", result.stderr)

    def test_runbook_urls_resolve_in_the_rule_and_the_config_map(self) -> None:
        result = helm_template(APP_PACK, "--set", APP_RUNBOOKS)
        self.assertEqual(result.returncode, 0, result.stderr)
        alerts = (APP_PACK / "alerts/baukit.rules.yml").read_text(encoding="utf-8")
        tokens = alerts.count("RUNBOOK_BASE_URL/service-scrape-down")
        self.assertEqual(tokens, 1)
        self.assertEqual(
            result.stdout.count("runbook_url: https://runbooks.test/app/service-scrape-down"), 2
        )
        self.assertNotIn("RUNBOOK_BASE_URL", result.stdout)


if __name__ == "__main__":
    unittest.main()
