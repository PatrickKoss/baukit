import re
import tempfile
import unittest
from pathlib import Path

from process_runner import run_process

ROOT = Path(__file__).resolve().parents[1]
GATE = ROOT / "templates/common/__strict__/scripts/quality-gate.sh"
FAKE_UV = '#!/bin/sh\nprintf \'%s\\n\' "$*" > "$UV_ARGS"\n'


def deploy_values_python():
    match = re.search(r"^deploy_values_python\(\) \{\n.*?^\}\n", GATE.read_text(), re.M | re.S)
    assert match, "deploy_values_python is missing from the strict gate"
    return match.group(0)


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


if __name__ == "__main__":
    unittest.main()
