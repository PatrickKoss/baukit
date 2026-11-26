import shutil
import sys
import tempfile
import unittest
from pathlib import Path

from process_runner import run_process

TEMPLATE_SCRIPTS = Path(__file__).resolve().parents[1] / "templates/common/scripts"


class RepoLintTemplateTests(unittest.TestCase):
    def test_template_repo_lint_suite_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            for name in ("repo-lint.sh", "repo-lint.test.py"):
                shutil.copy(TEMPLATE_SCRIPTS / name, directory)
            result = run_process(
                [sys.executable, str(Path(directory) / "repo-lint.test.py")],
                capture_output=True,
                text=True,
                check=False,
                timeout=120,
            )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("OK", result.stderr)


if __name__ == "__main__":
    unittest.main()
