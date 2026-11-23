from __future__ import annotations

import contextlib
import importlib.util
import io
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

CHECK_PATH = Path(__file__).resolve().parent / "check-deploy-values.py"
SPEC = importlib.util.spec_from_file_location("check_deploy_values", CHECK_PATH)
assert SPEC is not None and SPEC.loader is not None
check = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = check
SPEC.loader.exec_module(check)

CHART = Path(__file__).resolve().parents[1] / "deploy/chart/baukit-app"
KUSTOMIZATION = "apiVersion: kustomize.config.k8s.io/v1beta1\nkind: Kustomization\n"

BASE_RELEASE = """\
apiVersion: helm.toolkit.fluxcd.io/v2
kind: HelmRelease
metadata:
  name: demo
spec:
  releaseName: demo
  chart:
    spec:
      chart: ./deploy/chart/baukit-app
  values:
    product: demo
    api:
      image: {repository: registry.example/demo-api}
    worker:
      enabled: true
      image: {repository: registry.example/demo-worker}
    migration:
      image: {repository: registry.example/demo-migrate}
"""

ENVIRONMENT = """\
apiVersion: helm.toolkit.fluxcd.io/v2
kind: HelmRelease
metadata:
  name: demo
spec:
  values:
    deploymentEnvironment: testing
    backing:
      redis:
        mode: dedicated
    api:
      image: {tag: 1.0.0}
    worker:
      image: {tag: 1.0.0}
    migration:
      image: {tag: 1.0.0}
"""


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(textwrap.dedent(text))


def product(root: Path, environment: str = ENVIRONMENT) -> Path:
    gitops = root / "deploy/gitops"
    write(gitops / "base/helmrelease.yaml", BASE_RELEASE)
    write(gitops / "base/kustomization.yaml", f"{KUSTOMIZATION}resources:\n  - helmrelease.yaml\n")
    write(gitops / "testing/environment.yaml", environment)
    write(
        gitops / "testing/kustomization.yaml",
        f"{KUSTOMIZATION}namespace: demo-testing\n"
        "resources:\n  - ../base\npatches:\n  - path: environment.yaml\n",
    )
    write(
        gitops / "shared/kustomization.yaml",
        "apiVersion: kustomize.config.k8s.io/v1alpha1\nkind: Component\n",
    )
    return root


def run_main(root: Path) -> tuple[int, str]:
    stderr = io.StringIO()
    with contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(io.StringIO()):
        status = check.main(["check-deploy-values.py", str(root), str(CHART)])
    return status, stderr.getvalue()


class DeployValuesTest(unittest.TestCase):
    def test_overlays_skip_base_and_components(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = product(Path(directory))
            self.assertEqual(check.overlays(root), [root / "deploy/gitops/testing"])

    def test_clean_overlay_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(run_main(product(Path(directory))), (0, ""))

    def test_worker_env_that_repeats_a_chart_variable_fails(self) -> None:
        worker = "    worker:\n      image: {tag: 1.0.0}\n"
        environment = ENVIRONMENT.replace(
            worker,
            worker
            + "      env:\n        - {name: DEMO__RATE_LIMIT__REDIS_URL, value: redis://demo-redis:6379}\n",
        )
        with tempfile.TemporaryDirectory() as directory:
            status, stderr = run_main(product(Path(directory), environment))
        self.assertEqual(status, 1)
        self.assertIn(
            "deploy/gitops/testing: HelmRelease demo: Deployment demo-baukit-app-worker", stderr
        )
        self.assertIn("sets DEMO__RATE_LIMIT__REDIS_URL 2 times", stderr)

    def test_values_the_chart_schema_rejects_fail(self) -> None:
        environment = ENVIRONMENT + "      unknownKey: true\n"
        with tempfile.TemporaryDirectory() as directory:
            status, stderr = run_main(product(Path(directory), environment))
        self.assertEqual(status, 1)
        self.assertIn("deploy/gitops/testing: helm lint failed", stderr)

    def test_missing_overlays_fail(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            status, stderr = run_main(Path(directory))
        self.assertEqual(status, 1)
        self.assertIn("deploy/gitops has no overlay", stderr)

    def test_duplicate_env_checks_init_containers_and_jobs(self) -> None:
        job = {
            "kind": "Job",
            "metadata": {"name": "migrate"},
            "spec": {
                "template": {
                    "spec": {
                        "initContainers": [{"name": "wait", "env": [{"name": "A"}, {"name": "A"}]}],
                        "containers": [{"name": "migrate", "env": [{"name": "A"}]}],
                    }
                }
            },
        }
        service = {"kind": "Service", "metadata": {"name": "api"}, "spec": {}}
        self.assertEqual(
            check.duplicate_env([job, service]),
            [
                "Job migrate container wait sets A 2 times; "
                "server-side apply rejects duplicate env entries"
            ],
        )


if __name__ == "__main__":
    unittest.main()
