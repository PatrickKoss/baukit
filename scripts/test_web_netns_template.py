import re
import shutil
import tempfile
import unittest
from pathlib import Path

from process_runner import run_process

ROOT = Path(__file__).resolve().parents[1]
GATE = ROOT / "templates/common/__strict__/scripts/quality-gate.sh"
CI = ROOT / "templates/common/.github/workflows/ci.yml"
HELPER = ROOT / "templates/web/scripts/with-loopback-network.sh"
HELPER_TEST = ROOT / "templates/web/scripts/with-loopback-network.test.py"
SYSCTL = "kernel.apparmor_restrict_unprivileged_userns=0"


def function_body(source: str, name: str) -> str:
    match = re.search(rf"^{name}\(\) \{{\n(.*?)^\}}", source, re.S | re.M)
    assert match, name
    return match.group(1)


class WebNetnsTemplateTest(unittest.TestCase):
    def test_helper_matches_the_baukit_copy(self):
        self.assertEqual(HELPER.read_text(), (ROOT / "scripts/with-loopback-network.sh").read_text())

    def test_playwright_steps_run_through_the_helper(self):
        gate = GATE.read_text()
        for step in ("step_playwright", "step_webkit_repeats"):
            body = function_body(gate, step)
            self.assertIn("run_on_loopback ", body, step)
            self.assertRegex(body, r"run_on_loopback (env \S+ )?corepack .* playwright test", step)

    def test_helper_is_never_silently_skipped(self):
        body = function_body(GATE.read_text(), "run_on_loopback")
        self.assertIn('BAUKIT_E2E_NETWORK_ISOLATION:-1}" = "0"', body)
        self.assertIn("sh scripts/with-loopback-network.sh", body)

    def test_gate_runs_the_helper_test_for_web_products(self):
        self.assertIn("with-loopback-network.test.py", function_body(GATE.read_text(), "step_scripts_tests"))

    def test_every_job_running_the_helper_enables_user_namespaces(self):
        ci = CI.read_text()
        jobs = re.split(r"^  (?=[a-z][a-z0-9-]*:\n)", ci, flags=re.M)
        checked = 0
        for job in jobs:
            if "scripts/quality-gate.sh" in job or "with-loopback-network.sh" in job:
                checked += 1
                self.assertIn(SYSCTL, job, job.splitlines()[0])
                before = job.index(SYSCTL)
                after = min(i for i in (job.find("quality-gate.sh"), job.find("with-loopback-network.sh")) if i >= 0)
                self.assertLess(before, after)
        self.assertGreaterEqual(checked, 2)

    def test_helper_suite_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            for source in (HELPER, HELPER_TEST, ROOT / "templates/common/scripts/process_runner.py"):
                shutil.copy(source, directory)
            result = run_process(
                ["python3", str(Path(directory) / HELPER_TEST.name)],
                capture_output=True, text=True, check=False, timeout=120,
            )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
