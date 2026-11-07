from __future__ import annotations

import subprocess
import unittest
from pathlib import Path

CHART = Path(__file__).resolve().parents[1] / "deploy/chart/baukit-app"
RELEASE = "pdb-test"


class PodDisruptionBudgetChartTest(unittest.TestCase):
    def render(self, *settings: str) -> subprocess.CompletedProcess:
        command = ["helm", "template", RELEASE, str(CHART)]
        for setting in settings:
            command.extend(["--set", setting])
        return subprocess.run(command, capture_output=True, text=True, check=False)

    def budgets(self, *settings: str) -> dict[str, str]:
        result = self.render(*settings)
        self.assertEqual(result.returncode, 0, result.stderr)
        docs = [doc for doc in result.stdout.split("---") if "kind: PodDisruptionBudget" in doc]
        return {self.name_of(doc): doc for doc in docs}

    @staticmethod
    def name_of(doc: str) -> str:
        for line in doc.splitlines():
            if line.startswith("  name: "):
                return line.removeprefix("  name: ").strip()
        raise AssertionError(f"budget without a name:\n{doc}")

    def test_budgets_are_off_by_default(self) -> None:
        self.assertEqual(self.budgets("worker.enabled=true"), {})

    def test_api_budget_selects_the_api_pods(self) -> None:
        budgets = self.budgets(
            "api.podDisruptionBudget.enabled=true", "api.podDisruptionBudget.minAvailable=1"
        )
        self.assertEqual(list(budgets), [f"{RELEASE}-baukit-app-api"])
        api = budgets[f"{RELEASE}-baukit-app-api"]
        self.assertIn("apiVersion: policy/v1", api)
        self.assertIn("  minAvailable: 1\n", api)
        self.assertNotIn("maxUnavailable", api)
        self.assertIn("baukit.dev/process: api", api)
        self.assertIn(f"app.kubernetes.io/instance: {RELEASE}", api)

    def test_worker_budget_selects_the_worker_pods(self) -> None:
        budgets = self.budgets(
            "worker.enabled=true",
            "worker.podDisruptionBudget.enabled=true",
            "worker.podDisruptionBudget.maxUnavailable=50%",
        )
        self.assertEqual(list(budgets), [f"{RELEASE}-baukit-app-worker"])
        worker = budgets[f"{RELEASE}-baukit-app-worker"]
        self.assertIn('  maxUnavailable: "50%"\n', worker)
        self.assertNotIn("minAvailable", worker)
        self.assertIn("baukit.dev/process: worker", worker)

    def test_selectors_match_the_deployment_selectors(self) -> None:
        result = self.render(
            "worker.enabled=true",
            "api.podDisruptionBudget.enabled=true",
            "api.podDisruptionBudget.minAvailable=1",
            "worker.podDisruptionBudget.enabled=true",
            "worker.podDisruptionBudget.minAvailable=1",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        docs = result.stdout.split("---")
        for process in ["api", "worker"]:
            with self.subTest(process=process):
                name = f"name: {RELEASE}-baukit-app-{process}\n"
                deployment = next(d for d in docs if "kind: Deployment" in d and name in d)
                budget = next(d for d in docs if "kind: PodDisruptionBudget" in d and name in d)
                self.assertEqual(self.match_labels(budget), self.match_labels(deployment))

    @staticmethod
    def match_labels(doc: str) -> list[str]:
        lines = doc.splitlines()
        start = next(i for i, line in enumerate(lines) if line.strip() == "matchLabels:")
        indent = len(lines[start]) - len(lines[start].lstrip()) + 2
        labels = []
        for line in lines[start + 1 :]:
            if len(line) - len(line.lstrip()) != indent:
                break
            labels.append(line.strip())
        return labels

    def test_worker_budget_needs_the_worker(self) -> None:
        self.assertEqual(
            self.budgets(
                "worker.podDisruptionBudget.enabled=true",
                "worker.podDisruptionBudget.minAvailable=1",
            ),
            {},
        )

    def test_both_fields_fail(self) -> None:
        for process in ["api", "worker"]:
            with self.subTest(process=process):
                result = self.render(
                    "worker.enabled=true",
                    f"{process}.podDisruptionBudget.enabled=true",
                    f"{process}.podDisruptionBudget.minAvailable=1",
                    f"{process}.podDisruptionBudget.maxUnavailable=1",
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(
                    f"{process}.podDisruptionBudget sets both minAvailable and maxUnavailable",
                    result.stderr,
                )

    def test_neither_field_fails(self) -> None:
        result = self.render("api.podDisruptionBudget.enabled=true")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("api.podDisruptionBudget needs minAvailable or maxUnavailable", result.stderr)

    def test_budget_renders_alongside_the_hpa(self) -> None:
        result = self.render(
            "autoscaling.enabled=true",
            "api.podDisruptionBudget.enabled=true",
            "api.podDisruptionBudget.minAvailable=1",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("kind: HorizontalPodAutoscaler", result.stdout)
        self.assertEqual(result.stdout.count("kind: PodDisruptionBudget"), 1)

    def test_readme_documents_the_values(self) -> None:
        readme = (CHART / "README.md").read_text()
        for process in ["api", "worker"]:
            self.assertIn(f"`{process}.podDisruptionBudget.enabled`", readme)


if __name__ == "__main__":
    unittest.main()
