from __future__ import annotations

import contextlib
import copy
import importlib.util
import io
import json
import shutil
import tempfile
import unittest
from pathlib import Path

LINT = Path(__file__).resolve().parent
ROOT = LINT.parents[2]
PACK_DASHBOARDS = (
    ROOT / "deploy/observability/dashboards",
    ROOT / "deploy/observability-platform/dashboards",
)
PLATFORM_PACK = ROOT / "deploy/observability-platform"


def load(name: str, file_name: str):
    spec = importlib.util.spec_from_file_location(name, LINT / file_name)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


checker = load("check_dashboards", "check-dashboards.py")
metric_lint = load("check_metric_names_for_dashboards", "check-metric-names.py")


def run(module, *arguments: str) -> tuple[int, str, str]:
    stdout = io.StringIO()
    stderr = io.StringIO()
    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
        status = module.main(arguments)
    return status, stdout.getvalue(), stderr.getvalue()


def valid_dashboard() -> dict:
    datasource = {"type": "prometheus", "uid": "${DS_PROMETHEUS}"}
    return {
        "title": "Example",
        "uid": "example",
        "schemaVersion": 39,
        "templating": {"list": [{"name": "DS_PROMETHEUS", "type": "datasource"}]},
        "annotations": {"list": [{"datasource": {"uid": "-- Grafana --"}}]},
        "panels": [
            {"id": 1, "type": "row", "title": "Row", "panels": []},
            {"id": 2, "type": "text", "title": "Notes"},
            {
                "id": 3,
                "type": "timeseries",
                "title": "Up",
                "datasource": datasource,
                "targets": [
                    {"datasource": datasource, "expr": "up", "refId": "A"},
                    {"datasource": {"type": "__expr__", "uid": "__expr__"}, "refId": "B"},
                ],
            },
        ],
    }


class DashboardDirectory:
    def __init__(self, test: unittest.TestCase) -> None:
        directory = tempfile.TemporaryDirectory()
        test.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def write(self, document: object, name: str = "example.json") -> None:
        text = document if isinstance(document, str) else json.dumps(document)
        (self.root / name).write_text(text, encoding="utf-8")


class CheckDashboardsTest(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = DashboardDirectory(self)

    def check(self) -> tuple[int, str]:
        status, _, stderr = run(checker, str(self.directory.root))
        return status, stderr

    def assert_fails(self, document: object, message: str) -> None:
        self.directory.write(document)
        status, stderr = self.check()
        self.assertEqual(status, 1)
        self.assertIn(message, stderr)

    def test_real_pack_dashboards_pass(self) -> None:
        status, stdout, stderr = run(checker, *map(str, PACK_DASHBOARDS))

        self.assertEqual(status, 0, stderr)
        self.assertRegex(stdout, r"passed: \d+ dashboard")

    def test_valid_dashboard_passes(self) -> None:
        self.directory.write(valid_dashboard())

        self.assertEqual(self.check(), (0, ""))

    def test_missing_top_level_keys_fail(self) -> None:
        for key in checker.REQUIRED_KEYS:
            with self.subTest(key=key):
                document = valid_dashboard()
                del document[key]
                self.assert_fails(document, f"missing top-level key {key!r}")

    def test_schema_version_must_be_an_integer(self) -> None:
        document = valid_dashboard()
        document["schemaVersion"] = "39"
        self.assert_fails(document, "schemaVersion must be an integer")

    def test_duplicate_panel_ids_fail_including_nested_rows(self) -> None:
        document = valid_dashboard()
        document["panels"][0]["panels"] = [copy.deepcopy(document["panels"][2])]
        self.assert_fails(document, "duplicate panel id 3")

    def test_panel_without_id_fails(self) -> None:
        document = valid_dashboard()
        del document["panels"][2]["id"]
        self.assert_fails(document, "panel 'Up' has no integer id")

    def test_query_panel_without_targets_fails(self) -> None:
        document = valid_dashboard()
        document["panels"][2]["targets"] = []
        self.assert_fails(document, "panel 'Up' has no targets")

    def test_hard_coded_datasource_uid_fails(self) -> None:
        document = valid_dashboard()
        document["panels"][2]["targets"][0]["datasource"] = {"uid": "prometheus"}
        self.assert_fails(document, "hard-coded datasource uid 'prometheus'")

    def test_plain_string_datasource_uid_fails(self) -> None:
        document = valid_dashboard()
        document["panels"][2]["datasource"] = "P1809F7CD0C75ACF3"
        self.assert_fails(document, "hard-coded datasource uid 'P1809F7CD0C75ACF3'")

    def test_undeclared_datasource_variable_fails(self) -> None:
        document = valid_dashboard()
        document["panels"][2]["datasource"] = {"uid": "${DS_LOKI}"}
        self.assert_fails(document, "datasource variable 'DS_LOKI' is not declared")

    def test_datasource_without_uid_fails(self) -> None:
        document = valid_dashboard()
        document["panels"][2]["datasource"] = {"type": "prometheus"}
        self.assert_fails(document, "datasource has no uid")

    def test_invalid_json_fails(self) -> None:
        self.assert_fails("{", "invalid dashboard JSON")

    def test_duplicate_dashboard_uid_across_files_fails(self) -> None:
        self.directory.write(valid_dashboard(), "a.json")
        self.directory.write(valid_dashboard(), "b.json")

        status, stderr = self.check()

        self.assertEqual(status, 1)
        self.assertIn("dashboard uid 'example' is also used by", stderr)

    def test_missing_directory_is_a_usage_error(self) -> None:
        status, _, stderr = run(checker, str(self.directory.root / "missing"))

        self.assertEqual(status, 2)
        self.assertIn("is not a directory", stderr)


class PlatformDashboardMetricLintTest(unittest.TestCase):
    """Every query in every platform dashboard reaches the metric lint."""

    def lint_copy_with(self, dashboard: Path, mutate) -> tuple[int, str]:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        pack = Path(directory.name) / "pack"
        (pack / "dashboards").mkdir(parents=True)
        document = json.loads(dashboard.read_text(encoding="utf-8"))
        mutate(document)
        (pack / "dashboards" / dashboard.name).write_text(json.dumps(document), encoding="utf-8")
        allowlist = Path(directory.name) / "metrics.txt"
        shutil.copy(PLATFORM_PACK / "metrics.txt", allowlist)
        status, _, stderr = run(
            metric_lint,
            "--observability-root",
            str(pack),
            "--allowlist",
            str(allowlist),
            "--no-catalog",
        )
        return status, stderr

    def test_each_dashboard_passes_and_every_prometheus_query_is_linted(self) -> None:
        for dashboard in sorted((PLATFORM_PACK / "dashboards").glob("*.json")):
            status, stderr = self.lint_copy_with(dashboard, lambda _: None)
            self.assertEqual(status, 0, f"{dashboard.name}: {stderr}")
            count = len(prometheus_targets(json.loads(dashboard.read_text(encoding="utf-8"))))
            self.assertGreater(count, 0, dashboard.name)
            for index in range(count):
                with self.subTest(dashboard=dashboard.name, query=index):
                    status, stderr = self.lint_copy_with(
                        dashboard, lambda document, at=index: add_unknown_metric(document, at)
                    )
                    self.assertEqual(status, 1)
                    self.assertIn("unknown metric 'baukit_not_a_platform_metric'", stderr)


def prometheus_targets(document: dict) -> list[dict]:
    """Targets with an expr whose datasource is Prometheus, from the target or its panel."""
    found = []
    for panel in checker.all_panels(document["panels"]):
        for target in panel.get("targets", []):
            datasource = target.get("datasource") or panel.get("datasource") or {}
            if (
                isinstance(target.get("expr"), str)
                and target["expr"]
                and (
                    datasource.get("type") == "prometheus"
                    or datasource.get("uid") == "${DS_PROMETHEUS}"
                )
            ):
                found.append(target)
    return found


def add_unknown_metric(document: dict, index: int) -> None:
    prometheus_targets(document)[index]["expr"] += " or baukit_not_a_platform_metric"


if __name__ == "__main__":
    unittest.main()
