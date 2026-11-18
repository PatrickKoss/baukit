from __future__ import annotations

import re
import unittest
from pathlib import Path

GATE = Path(__file__).resolve().parents[1] / "scripts" / "generated-fixture-gate.sh"
MISE_CALL = re.compile(r"(?:^|[\s(&;])mise\s")


class FixtureMiseScopeTests(unittest.TestCase):
    def test_every_mise_call_stops_at_the_fixture_parent(self):
        code = [line for line in GATE.read_text().splitlines() if not line.lstrip().startswith("#")]
        calls = [line for line in code if MISE_CALL.search(line)]
        self.assertTrue(calls)
        for line in calls:
            with self.subTest(line=line.strip()):
                self.assertIn('MISE_CEILING_PATHS="$mise_ceiling" mise ', line)

    def test_ceiling_is_the_directory_that_holds_the_fixture(self):
        self.assertIn('mise_ceiling="$fixture_parent/$flavor"', GATE.read_text())
        self.assertIn('fixture="$fixture_parent/$flavor/$name"', GATE.read_text())


if __name__ == "__main__":
    unittest.main()
