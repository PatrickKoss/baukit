import subprocess
import sys
import unittest
from pathlib import Path

TEST = Path(__file__).resolve().parents[1] / "templates/common/__strict__/scripts/gate.test.py"


class GateTemplateTests(unittest.TestCase):
    def test_template_gate_suite_passes(self):
        result = subprocess.run([sys.executable, str(TEST)], capture_output=True, text=True, check=False, timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("OK", result.stderr)


if __name__ == "__main__":
    unittest.main()
