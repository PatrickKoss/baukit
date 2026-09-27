from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import tempfile
import unittest
from pathlib import Path

LINTER_PATH = Path(__file__).resolve().parent / "check-metric-names.py"
SPEC = importlib.util.spec_from_file_location("check_metric_names", LINTER_PATH)
assert SPEC is not None and SPEC.loader is not None
linter = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(linter)


def run(*arguments: str) -> tuple[int, str, str]:
    stdout = io.StringIO()
    stderr = io.StringIO()
    with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
        status = linter.main(arguments)
    return status, stdout.getvalue(), stderr.getvalue()


def dashboard(*expressions: str) -> str:
    panels = [{"targets": [{"expr": expression}]} for expression in expressions]
    return json.dumps({"panels": panels})


class ProductObservability:
    def __init__(self, root: Path) -> None:
        self.root = root / "observability"
        (self.root / "dashboards").mkdir(parents=True)
        (self.root / "alerts").mkdir()
        self.allowlist = root / "product-metrics.txt"

    def write_dashboard(self, *expressions: str) -> None:
        (self.root / "dashboards" / "product.json").write_text(
            dashboard(*expressions), encoding="utf-8"
        )

    def write_alert(self, expression: str) -> None:
        (self.root / "alerts" / "product.rules.yml").write_text(
            "groups:\n"
            "  - name: product\n"
            "    rules:\n"
            "      - alert: ProductStalled\n"
            f"        expr: {expression}\n",
            encoding="utf-8",
        )

    def write_allowlist(self, content: str) -> None:
        self.allowlist.write_text(content, encoding="utf-8")

    def lint(self, *extra: str) -> tuple[int, str, str]:
        return run(
            "--observability-root",
            str(self.root),
            "--allowlist",
            str(self.allowlist),
            *extra,
        )


class BaukitInvocationTests(unittest.TestCase):
    def test_default_invocation_lints_baukit_observability(self) -> None:
        status, stdout, stderr = run()

        self.assertEqual(status, 0, stderr)
        self.assertRegex(
            stdout,
            r"^Observability metric-name lint passed: 1 dashboard\(s\), "
            r"\d+ rule file\(s\), \d+ local recording rule\(s\)\.\n$",
        )


class ProductInvocationTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.product = ProductObservability(Path(directory.name))

    def test_accepts_product_metrics_and_histogram_series(self) -> None:
        self.product.write_allowlist(
            "# product metrics\n"
            "shop_orders_total\n"
            "shop_checkout_duration_seconds histogram  # latency\n"
        )
        self.product.write_dashboard(
            "sum(rate(shop_orders_total[5m]))",
            "histogram_quantile(0.95, sum by (le) "
            "(rate(shop_checkout_duration_seconds_bucket[5m])))",
            "sum(rate(http_requests_total[5m]))",
        )
        self.product.write_alert("rate(shop_orders_total[10m]) == 0")

        status, stdout, stderr = self.product.lint()

        self.assertEqual(status, 0, stderr)
        self.assertIn("1 dashboard(s), 1 rule file(s)", stdout)

    def test_rejects_metrics_missing_from_the_allowlist(self) -> None:
        self.product.write_allowlist("shop_orders_total\n")
        self.product.write_dashboard("sum(rate(shop_refunds_total[5m]))")

        status, _, stderr = self.product.lint()

        self.assertEqual(status, 1)
        self.assertIn("unknown metric 'shop_refunds_total'", stderr)

    def test_histogram_series_need_the_histogram_marker(self) -> None:
        self.product.write_allowlist("shop_checkout_duration_seconds\n")
        self.product.write_dashboard("rate(shop_checkout_duration_seconds_bucket[5m])")

        status, _, stderr = self.product.lint()

        self.assertEqual(status, 1)
        self.assertIn("unknown metric 'shop_checkout_duration_seconds_bucket'", stderr)

    def test_lints_extra_rule_files_outside_the_root(self) -> None:
        self.product.write_allowlist("shop_orders_total\n")
        extra_rules = self.product.allowlist.parent / "alerts.yml"
        extra_rules.write_text(
            "groups:\n"
            "  - name: extra\n"
            "    rules:\n"
            "      - alert: Unknown\n"
            "        expr: shop_unknown_total > 0\n",
            encoding="utf-8",
        )

        status, _, stderr = self.product.lint("--rules", str(extra_rules))

        self.assertEqual(status, 1)
        self.assertIn("alerts.yml:5: unknown metric 'shop_unknown_total'", stderr)

    def test_rejects_invalid_allowlist_entries(self) -> None:
        cases = {
            "shop_orders_total\nshop_orders_total\n": "duplicate metric name",
            "shop_orders_total counter\n": "optional 'histogram'",
            "shop-orders-total\n": "invalid metric name",
        }
        for content, message in cases.items():
            with self.subTest(content=content):
                self.product.write_allowlist(content)

                status, _, stderr = self.product.lint()

                self.assertEqual(status, 2)
                self.assertIn(message, stderr)

    def test_rejects_a_missing_observability_root(self) -> None:
        status, _, stderr = run("--observability-root", str(self.product.root / "missing"))

        self.assertEqual(status, 2)
        self.assertIn("observability root is not a directory", stderr)


if __name__ == "__main__":
    unittest.main()
