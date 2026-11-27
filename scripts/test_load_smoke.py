from __future__ import annotations

import gzip
import importlib.util
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

from process_runner import run_process

REPO_ROOT = Path(__file__).resolve().parents[1]
RUNNER = REPO_ROOT / "deploy" / "smoke" / "run-load-smoke.sh"
SCRIPT = REPO_ROOT / "deploy" / "smoke" / "api-load.js"
CHECKER = REPO_ROOT / "deploy" / "smoke" / "admitted_latency.py"
MANIFEST = REPO_ROOT / "deploy" / "tools.toml"
FAKE_TOKEN = "not-a-real-token"
FAKE_DOCKER = """#!/bin/sh
printf '%s\\n' "$@" >"$DOCKER_ARGS"
env | sort >"$DOCKER_ENV"
for arg in "$@"; do
  case "$arg" in *:/out) out="${arg%:/out}" ;; esac
done
[ -n "${DOCKER_SAMPLES:-}" ] && cp "$DOCKER_SAMPLES" "$out/samples.csv.gz"
exit "${DOCKER_EXIT:-0}"
"""
CSV_HEADER = "metric_name,timestamp,metric_value,scenario\n"


def load_checker():
    spec = importlib.util.spec_from_file_location("admitted_latency", CHECKER)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def write_samples(path: Path, rows: list[tuple[str, float, float]]) -> Path:
    lines = [CSV_HEADER, *(f"{name},{ended},{value},overload\n" for name, ended, value in rows)]
    with gzip.open(path, "wt", encoding="utf-8") as handle:
        handle.writelines(lines)
    return path


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
            "DOCKER_SAMPLES": str(
                write_samples(self.work / "fast.csv.gz", [("admitted_duration", 1.0, 5.0)])
            ),
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
        self.assertEqual(
            args[-5:], ["run", "--quiet", "--out", "csv=/out/samples.csv.gz", "/smoke/api-load.js"]
        )

    def test_passes_only_the_settings_that_are_set(self) -> None:
        self.run_runner(API_URL="http://127.0.0.1:18080", SMOKE_TOKEN=FAKE_TOKEN)

        args = self.docker_args()
        self.assertIn("API_URL", args)
        self.assertIn("SMOKE_TOKEN", args)
        self.assertNotIn("PEAK_RPS", args)
        self.assertNotIn("ADMITTED_P99_MS", args)
        self.assertNotIn("secret-token", " ".join(args))

    def test_forwards_extra_k6_arguments(self) -> None:
        self.run_runner("--summary-export", "summary.json")

        self.assertEqual(self.docker_args()[-2:], ["--summary-export", "summary.json"])

    def test_returns_the_k6_exit_status(self) -> None:
        result = self.run_runner(DOCKER_EXIT="99")

        self.assertEqual(result.returncode, 99)

    def test_fails_when_the_admitted_p99_misses_the_target(self) -> None:
        rows = [("admitted_duration", 1000.0 + index, 1600.0) for index in range(10)]
        samples = write_samples(self.work / "slow.csv.gz", rows)

        result = self.run_runner(DOCKER_SAMPLES=str(samples))

        self.assertEqual(result.returncode, 99, result.stdout)
        self.assertIn("p99=1600.0ms", result.stdout)

    def test_passes_the_admitted_p99_target_through(self) -> None:
        rows = [("admitted_duration", 1000.0 + index, 1600.0) for index in range(10)]
        samples = write_samples(self.work / "slow.csv.gz", rows)

        result = self.run_runner(DOCKER_SAMPLES=str(samples), ADMITTED_P99_MS="2000")

        self.assertEqual(result.returncode, 0, result.stdout)

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
            ("process_up", '"rate==1"'),
        ]:
            with self.subTest(metric=metric):
                self.assertIn(f"{metric}: [{condition}]", thresholds.group(1))

    def test_admitted_p99_target_defaults_to_queue_timeout_plus_handler_budget(self) -> None:
        runner = RUNNER.read_text(encoding="utf-8")
        self.assertIn("admitted_p99_ms=${ADMITTED_P99_MS:-1500}", runner)

    def test_is_not_part_of_ci(self) -> None:
        makefile = (REPO_ROOT / "Makefile").read_text(encoding="utf-8")
        ci = next(line for line in makefile.splitlines() if line.startswith("ci:"))
        self.assertNotIn("load-smoke", ci)
        self.assertIn("load-smoke:\n\t./deploy/smoke/run-load-smoke.sh", makefile)


class AdmittedLatencyTest(unittest.TestCase):
    def setUp(self) -> None:
        self.checker = load_checker()
        self.work = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.work)

    def check(self, rows: list[tuple[str, float, float]], target: str = "1500") -> int:
        samples = write_samples(self.work / "s.csv.gz", rows)
        return self.checker.main([str(samples), "--p99-ms", target])

    def test_removes_a_wall_clock_step_from_the_requests_that_span_it(self) -> None:
        step_ms = 3600.0
        tick_ended = 10_000.0
        spanning = [("admitted_duration", tick_ended + 200, 1000.0 + step_ms)] * 50
        steady = [("admitted_duration", 5_000.0, 1000.0)] * 50
        clock = [("clock_lag", tick_ended, step_ms), ("clock_lag", 4_000.0, 2.0)]

        result = self.checker.report(
            self.checker.read_samples(
                write_samples(self.work / "s.csv.gz", spanning + steady + clock)
            )
        )

        self.assertEqual(result.p99_ms, 1000.0)
        self.assertEqual(result.raw_p99_ms, 1000.0 + step_ms)
        self.assertEqual(result.corrected, 50)

    def test_keeps_slow_requests_that_do_not_span_a_step(self) -> None:
        rows = [
            ("clock_lag", 10_000.0, 3600.0),
            *[("admitted_duration", 5_000.0, 4600.0)] * 2,
            *[("admitted_duration", 20_000.0, 1000.0)] * 98,
        ]

        self.assertEqual(self.check(rows), 99)

    def test_ignores_scheduling_jitter_below_the_step_tolerance(self) -> None:
        rows = [
            ("clock_lag", 10_000.0, 200.0),
            *[("admitted_duration", 10_100.0, 1700.0)] * 2,
            *[("admitted_duration", 20_000.0, 1000.0)] * 98,
        ]

        self.assertEqual(self.check(rows), 99)

    def test_fails_without_admitted_samples(self) -> None:
        self.assertEqual(self.check([("clock_lag", 1.0, 1.0)]), 2)

    def test_fails_without_a_samples_file(self) -> None:
        missing = self.work / "missing.csv.gz"
        self.assertEqual(self.checker.main([str(missing), "--p99-ms", "1500"]), 2)


if __name__ == "__main__":
    unittest.main()
