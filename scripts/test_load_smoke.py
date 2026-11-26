from __future__ import annotations

import re
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path

from process_runner import run_process

REPO_ROOT = Path(__file__).resolve().parents[1]
RUNNER = REPO_ROOT / "deploy" / "smoke" / "run-load-smoke.sh"
SCRIPT = REPO_ROOT / "deploy" / "smoke" / "api-load.js"
MANIFEST = REPO_ROOT / "deploy" / "tools.toml"
FAKE_DOCKER = """#!/bin/sh
printf '%s\\n' "$@" >"$DOCKER_ARGS"
env | sort >"$DOCKER_ENV"
exit "${DOCKER_EXIT:-0}"
"""


class LoadSmokeRunnerTest(unittest.TestCase):
    def setUp(self) -> None:
        self.work = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.work)
        bin_dir = self.work / "bin"
        bin_dir.mkdir()
        docker = bin_dir / "docker"
        docker.write_text(FAKE_DOCKER, encoding="utf-8")
        docker.chmod(0o755)
        self.args_file = self.work / "args"
        self.env = {
            "PATH": f"{bin_dir}:/usr/bin:/bin",
            "HOME": str(self.work),
            "DOCKER_ARGS": str(self.args_file),
            "DOCKER_ENV": str(self.work / "env"),
        }

    def run_runner(self, *arguments: str, **env: str) -> subprocess.CompletedProcess:
        return run_process(
            ["bash", str(RUNNER), *arguments],
            env={**self.env, **env},
            capture_output=True,
            text=True,
        )

    def docker_args(self) -> list[str]:
        return self.args_file.read_text(encoding="utf-8").splitlines()

    def test_runs_the_pinned_image_on_the_host_network(self) -> None:
        pinned = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))["images"]["k6"]
        repository, digest = pinned.split(":", 1)[0], pinned.split("@", 1)[1]

        result = self.run_runner()

        self.assertEqual(result.returncode, 0, result.stderr)
        args = self.docker_args()
        self.assertEqual(args[:4], ["run", "--rm", "--network", "host"])
        self.assertIn(f"docker.io/{repository}@{digest}", args)
        self.assertIn(f"{RUNNER.parent}:/smoke:ro", args)
        self.assertEqual(args[-3:], ["run", "--quiet", "/smoke/api-load.js"])

    def test_passes_only_the_settings_that_are_set(self) -> None:
        self.run_runner(API_URL="http://127.0.0.1:18080", SMOKE_TOKEN="secret-token")

        args = self.docker_args()
        self.assertIn("API_URL", args)
        self.assertIn("SMOKE_TOKEN", args)
        self.assertNotIn("PEAK_RPS", args)
        self.assertNotIn("secret-token", " ".join(args))

    def test_forwards_extra_k6_arguments(self) -> None:
        self.run_runner("--summary-export", "/tmp/summary.json")

        self.assertEqual(self.docker_args()[-2:], ["--summary-export", "/tmp/summary.json"])

    def test_returns_the_k6_exit_status(self) -> None:
        result = self.run_runner(DOCKER_EXIT="99")

        self.assertEqual(result.returncode, 99)

    def test_fails_without_docker(self) -> None:
        bare = self.work / "bare"
        bare.mkdir()
        for tool in ["dirname", "python3"]:
            (bare / tool).symlink_to(shutil.which(tool) or tool)
        result = run_process(
            ["/bin/bash", str(RUNNER)],
            env={"PATH": str(bare), "HOME": str(self.work)},
            capture_output=True,
            text=True,
        )

        self.assertEqual(result.returncode, 2)
        self.assertIn("docker is not on PATH", result.stderr)


class LoadSmokeScriptTest(unittest.TestCase):
    def test_declares_every_threshold_the_readme_names(self) -> None:
        script = SCRIPT.read_text(encoding="utf-8")
        thresholds = re.search(r"thresholds: \{(.*?)\n  \},", script, re.S)
        assert thresholds is not None
        for metric, condition in [
            ("shed_responses", '"count>0"'),
            ("shed_without_retry_after", '"rate==0"'),
            ("unexpected_status", '"rate==0"'),
            ("admitted_duration", "`p(99)<${ADMITTED_P99_MS}`"),
            ("process_up", '"rate==1"'),
        ]:
            with self.subTest(metric=metric):
                self.assertIn(f"{metric}: [{condition}]", thresholds.group(1))

    def test_admitted_p99_target_defaults_to_queue_timeout_plus_handler_budget(self) -> None:
        script = SCRIPT.read_text(encoding="utf-8")
        self.assertIn('number("ADMITTED_P99_MS", 1500)', script)

    def test_is_not_part_of_ci(self) -> None:
        makefile = (REPO_ROOT / "Makefile").read_text(encoding="utf-8")
        ci = next(line for line in makefile.splitlines() if line.startswith("ci:"))
        self.assertNotIn("load-smoke", ci)
        self.assertIn("load-smoke:\n\t./deploy/smoke/run-load-smoke.sh", makefile)


if __name__ == "__main__":
    unittest.main()
