import re
import tempfile
import unittest
from pathlib import Path

from process_runner import run_process

ROOT = Path(__file__).resolve().parents[1]
GATE = ROOT / "templates/common/__strict__/scripts/quality-gate.sh"
FAKE_UV = '#!/bin/sh\nprintf \'%s\\n\' "$*" > "$UV_ARGS"\n'


def gate_function(name):
    match = re.search(rf"^{name}\(\) \{{\n.*?^\}}\n", GATE.read_text(), re.M | re.S)
    assert match, f"{name} is missing from the strict gate"
    return match.group(0)


def deploy_values_python():
    return gate_function("deploy_values_python")


class DeployValuesPythonTests(unittest.TestCase):
    def uv_args(self, *product_files):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in product_files:
                (root / name).write_text("")
            bin_dir = root / "bin"
            bin_dir.mkdir()
            (bin_dir / "uv").write_text(FAKE_UV)
            (bin_dir / "uv").chmod(0o755)
            args_file = root / "args"
            result = run_process(
                ["sh", "-c", deploy_values_python() + "deploy_values_python check.py . chart"],
                capture_output=True,
                text=True,
                check=False,
                cwd=root,
                env={"PATH": f"{bin_dir}:/usr/bin:/bin", "UV_ARGS": str(args_file)},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            return args_file.read_text().split()

    def test_a_locked_uv_project_runs_its_own_environment(self):
        self.assertEqual(
            self.uv_args("pyproject.toml", "uv.lock"),
            ["run", "--locked", "--project", ".", "python3", "check.py", ".", "chart"],
        )

    def test_a_product_without_a_uv_lock_gets_a_pinned_pyyaml(self):
        for files in ((), ("pyproject.toml",)):
            self.assertEqual(
                self.uv_args(*files),
                [
                    "run",
                    "--no-project",
                    "--with",
                    "PyYAML==6.0.3",
                    "python3",
                    "check.py",
                    ".",
                    "chart",
                ],
                files,
            )


class RequireNodeModulesTests(unittest.TestCase):
    def check(self, installed):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "web").mkdir()
            if installed:
                (root / "web/node_modules").mkdir()
            return run_process(
                [
                    "sh",
                    "-c",
                    gate_function("require_node_modules") + "require_node_modules web web-install",
                ],
                capture_output=True,
                text=True,
                check=False,
                cwd=root,
            )

    def test_installed_modules_pass(self):
        self.assertEqual(self.check(installed=True).returncode, 0)

    def test_missing_modules_name_the_install_step(self):
        result = self.check(installed=False)
        self.assertEqual(result.returncode, 1)
        self.assertIn("web/node_modules is missing", result.stderr)
        self.assertIn("--step web-install", result.stderr)


if __name__ == "__main__":
    unittest.main()
