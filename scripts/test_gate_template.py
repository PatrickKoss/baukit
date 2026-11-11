import shutil
import sys
import tempfile
import unittest
from pathlib import Path

from process_runner import run_process

TEMPLATES = Path(__file__).resolve().parents[1] / "templates/common"
SCRIPTS = (
    TEMPLATES / "__strict__/scripts/gate.py",
    TEMPLATES / "__strict__/scripts/gate.test.py",
    TEMPLATES / "scripts/process_runner.py",
)


class GateTemplateTests(unittest.TestCase):
    def test_template_gate_suite_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            for source in SCRIPTS:
                shutil.copy(source, directory)
            result = run_process(
                [sys.executable, str(Path(directory) / "gate.test.py")],
                capture_output=True,
                text=True,
                check=False,
                timeout=120,
            )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("OK", result.stderr)


if __name__ == "__main__":
    unittest.main()
