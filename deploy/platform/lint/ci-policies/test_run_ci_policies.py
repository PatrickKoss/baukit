import json
import tempfile
import unittest
from pathlib import Path

import run_ci_policies as rcp

RENDERED = """\
apiVersion: v1
kind: Namespace
metadata:
  name: demo
---
apiVersion: helm.toolkit.fluxcd.io/v2
kind: HelmRelease
metadata:
  name: app
---
apiVersion: source.toolkit.fluxcd.io/v1
kind: HelmRepository
metadata:
  name: charts
"""


def report(*results: tuple[str, str]) -> str:
    rows = [
        {"policy": policy, "result": outcome, "message": f"{policy} said {outcome}",
         "resources": [{"kind": "HelmRelease", "namespace": "demo", "name": "app"}]}
        for policy, outcome in results
    ]
    summary = {key: sum(1 for _, outcome in results if outcome == key) for key in rcp.COUNTED}
    return "Applying policies...\n" + json.dumps({"summary": summary, "results": rows})


class FakeKyverno:
    def __init__(self, apply_output: str, apply_code: int = 0, test_code: int = 0) -> None:
        self.apply_output = apply_output
        self.apply_code = apply_code
        self.test_code = test_code
        self.applied: list[str] = []

    def __call__(self, command: list[str]) -> tuple[int, str]:
        if command[1] == "test":
            return self.test_code, "Test Summary: 2 tests passed and 0 tests failed\n"
        self.applied.append(Path(command[command.index("--resource") + 1]).read_text())
        return self.apply_code, self.apply_output


class FluxDocumentsTest(unittest.TestCase):
    def test_keeps_flux_objects_only(self) -> None:
        flux = rcp.flux_documents(RENDERED)
        self.assertIn("kind: HelmRelease", flux)
        self.assertIn("kind: HelmRepository", flux)
        self.assertNotIn("kind: Namespace", flux)


class ApplyTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.rendered = root / "rendered"
        self.work = root / "work"
        self.rendered.mkdir()
        self.work.mkdir()
        (self.rendered / "base-demo.yaml").write_text(RENDERED)
        (self.rendered / "overlay-local.yaml").write_text(RENDERED)
        (self.rendered / "chart-demo-app.yaml").write_text(RENDERED)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_applies_bases_and_overlays_but_not_charts(self) -> None:
        kyverno = FakeKyverno(report(("helmrelease-timeout", "pass")))
        tally, problems = rcp.apply_policies(self.rendered, Path("policies"), kyverno, self.work)
        self.assertEqual(problems, [])
        self.assertEqual(len(kyverno.applied), 2)
        self.assertFalse(tally.failed)
        self.assertEqual(tally.counts["helmrelease-timeout"]["pass"], 2)

    def test_fail_result_is_reported_per_universe(self) -> None:
        kyverno = FakeKyverno(report(("helmrelease-timeout", "fail")), apply_code=1)
        tally, problems = rcp.apply_policies(self.rendered, Path("policies"), kyverno, self.work)
        self.assertEqual(problems, [])
        self.assertTrue(tally.failed)
        self.assertEqual(
            tally.failures[0], "base-demo: helmrelease-timeout: HelmRelease/demo/app: helmrelease-timeout said fail"
        )

    def test_error_result_fails(self) -> None:
        kyverno = FakeKyverno(report(("helmrelease-timeout", "error")), apply_code=1)
        tally, _ = rcp.apply_policies(self.rendered, Path("policies"), kyverno, self.work)
        self.assertTrue(tally.failed)

    def test_crash_without_report_is_a_problem(self) -> None:
        kyverno = FakeKyverno("panic: namespaces already exists", apply_code=2)
        tally, problems = rcp.apply_policies(self.rendered, Path("policies"), kyverno, self.work)
        self.assertEqual(len(problems), 2)
        self.assertFalse(tally.failed)

    def test_nonzero_exit_with_clean_report_is_a_problem(self) -> None:
        kyverno = FakeKyverno(report(("helmrelease-timeout", "pass")), apply_code=1)
        _, problems = rcp.apply_policies(self.rendered, Path("policies"), kyverno, self.work)
        self.assertEqual(len(problems), 2)

    def test_table_lists_policies_without_results(self) -> None:
        tally = rcp.Tally()
        tally.add("base-demo", json.loads(report(("helmrelease-timeout", "fail")).split("\n", 1)[1]))
        table = tally.table(["helmrelease-timeout", "ocirepository-exact-ref"])
        self.assertRegex(table, r"helmrelease-timeout +0 +1 +0 +0")
        self.assertRegex(table, r"ocirepository-exact-ref +0 +0 +0 +0")


class PolicyNamesTest(unittest.TestCase):
    def test_every_policy_has_a_test_suite(self) -> None:
        names = rcp.policy_names(rcp.HERE / "policies")
        self.assertGreaterEqual(len(names), 12)
        for name in names:
            suite = rcp.HERE / "tests" / name / "kyverno-test.yaml"
            self.assertTrue(suite.is_file(), name)
            text = suite.read_text()
            self.assertIn("result: pass", text, name)
            self.assertIn("result: fail", text, name)


if __name__ == "__main__":
    unittest.main()
