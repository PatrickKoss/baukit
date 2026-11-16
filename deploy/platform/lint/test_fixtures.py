from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

import helm_yaml

LINT_DIR = Path(__file__).resolve().parent
FIXTURES = LINT_DIR / "fixtures"
VALIDATE = LINT_DIR.parent / "validate.sh"
EXPECTED = {
    "helmrelease": "retires",
    "servicemonitor": "scrapeTimout",
    "cnpg-cluster": "enableSuperUserAccess",
}


def kubeconform(fixture: Path) -> subprocess.CompletedProcess[str]:
    return helm_yaml.process_runner.run_process(
        [str(VALIDATE), "--kubeconform", str(fixture)], capture_output=True, text=True
    )


class MisspelledFieldFixturesTest(unittest.TestCase):
    def test_every_expected_fixture_pair_exists(self) -> None:
        names = {path.name.split(".")[0] for path in FIXTURES.glob("*.yaml")}
        self.assertEqual(names, set(EXPECTED))
        for name in EXPECTED:
            self.assertTrue((FIXTURES / f"{name}.valid.yaml").is_file(), name)
            self.assertTrue((FIXTURES / f"{name}.invalid.yaml").is_file(), name)

    def test_misspelled_field_fails_and_corrected_twin_passes(self) -> None:
        for name, misspelling in EXPECTED.items():
            with self.subTest(fixture=name):
                invalid = kubeconform(FIXTURES / f"{name}.invalid.yaml")
                self.assertNotEqual(invalid.returncode, 0, invalid.stdout + invalid.stderr)
                self.assertIn("Invalid: 1", invalid.stdout)
                self.assertIn(f"'{misspelling}' not allowed", invalid.stdout)

                valid = kubeconform(FIXTURES / f"{name}.valid.yaml")
                self.assertEqual(valid.returncode, 0, valid.stdout + valid.stderr)
                self.assertIn("Valid: 1, Invalid: 0, Errors: 0, Skipped: 0", valid.stdout)

    def test_a_kind_without_a_schema_fails(self) -> None:
        with tempfile.NamedTemporaryFile("w", suffix=".yaml") as manifest:
            manifest.write("apiVersion: example.com/v1\nkind: Unknown\nmetadata:\n  name: x\n")
            manifest.flush()
            result = kubeconform(Path(manifest.name))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("could not find schema for Unknown", result.stdout)


if __name__ == "__main__":
    unittest.main()
