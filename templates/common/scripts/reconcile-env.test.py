#!/usr/bin/env python3

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


sys.dont_write_bytecode = True
SCRIPT = Path(__file__).with_name("reconcile-env.py")
SPEC = importlib.util.spec_from_file_location("reconcile_env", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {SCRIPT}")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
reconcile = MODULE.reconcile


class ReconcileEnvTest(unittest.TestCase):
    def test_cli_reports_names_without_values(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            example = root / ".env.example"
            env = root / ".env"
            example.write_bytes(b"SECRET=do-not-print-this\n")

            completed = subprocess.run(
                [sys.executable, str(SCRIPT), str(example), str(env)],
                check=True,
                capture_output=True,
                text=True,
            )

            self.assertEqual(completed.stdout, "added SECRET\n")
            self.assertNotIn("do-not-print-this", completed.stdout)
            self.assertEqual(completed.stderr, "")

    def test_preserves_existing_bytes_and_appends_in_example_order(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            example = root / ".env.example"
            env = root / ".env"
            example.write_bytes(
                b"# defaults\nFIRST=example\nexport SECOND=\"two words\"\n"
                b"BLANK=\n# DISABLED=no\n"
            )
            original = b"# local choices\r\nFIRST='custom value'\r\nBLANK=\r\nTAIL=yes"
            env.write_bytes(original)

            added = reconcile(example, env)

            self.assertEqual(added, ["SECOND"])
            self.assertEqual(
                env.read_bytes(), original + b'\nexport SECOND="two words"\n'
            )

    def test_creates_file_from_active_assignments(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            example = root / ".env.example"
            env = root / ".env"
            example.write_bytes(b"# comment\nONE=1\n\nexport TWO='2'\n")

            self.assertEqual(reconcile(example, env), ["ONE", "TWO"])
            self.assertEqual(env.read_bytes(), b"ONE=1\nexport TWO='2'\n")

    def test_existing_duplicate_is_preserved_and_counts_as_present(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            example = root / ".env.example"
            env = root / ".env"
            example.write_bytes(b"TOKEN=example\n")
            original = b"export TOKEN=first\nTOKEN=second\n"
            env.write_bytes(original)

            self.assertEqual(reconcile(example, env), [])
            self.assertEqual(env.read_bytes(), original)

    def test_first_example_duplicate_wins_and_second_run_is_no_op(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            example = root / ".env.example"
            env = root / ".env"
            example.write_bytes(b"DUPLICATE=first\nDUPLICATE=second\nNEXT=3\n")

            self.assertEqual(reconcile(example, env), ["DUPLICATE", "NEXT"])
            first_result = env.read_bytes()
            self.assertEqual(first_result, b"DUPLICATE=first\nNEXT=3\n")
            self.assertEqual(reconcile(example, env), [])
            self.assertEqual(env.read_bytes(), first_result)


if __name__ == "__main__":
    unittest.main()
