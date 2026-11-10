#!/usr/bin/env python3

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "osv_resolved_lock", Path(__file__).with_name("osv-resolved-lock.py")
)
osv_resolved_lock = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(osv_resolved_lock)

TREE = """app v0.1.0 (/work/app)
serde v1.0.200
serde_derive v1.0.200 (proc-macro)
tokio v1.40.0 (*)
local v0.2.0 (/work/local)
"""

LOCK = """version = 4

[[package]]
name = "serde"
version = "1.0.200"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "abc"

[[package]]
name = "libsqlite3-sys"
version = "0.37.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "local"
version = "0.2.0"
"""


class ResolvedLockTest(unittest.TestCase):
    def test_tree_lines_become_name_version_pairs(self):
        self.assertEqual(
            osv_resolved_lock.parse_tree_packages(TREE),
            {
                ("app", "0.1.0"),
                ("serde", "1.0.200"),
                ("serde_derive", "1.0.200"),
                ("tokio", "1.40.0"),
                ("local", "0.2.0"),
            },
        )

    def test_lockfile_only_packages_are_dropped(self):
        keep = osv_resolved_lock.parse_tree_packages(TREE)
        pruned = osv_resolved_lock.pruned_lock(LOCK, keep)
        self.assertIn('name = "serde"', pruned)
        self.assertIn('name = "local"', pruned)
        self.assertNotIn("libsqlite3-sys", pruned)
        self.assertIn('source = "registry+https://github.com/rust-lang/crates.io-index"', pruned)

    def test_cargo_resolves_a_real_workspace(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "src").mkdir()
            (root / "src/lib.rs").write_text("")
            (root / "Cargo.toml").write_text(
                '[package]\nname = "probe"\nversion = "0.1.0"\nedition = "2021"\n[dependencies]\n'
            )
            (root / "Cargo.lock").write_text(
                'version = 4\n\n[[package]]\nname = "probe"\nversion = "0.1.0"\n\n'
                '[[package]]\nname = "unused"\nversion = "1.0.0"\n'
            )
            keep = osv_resolved_lock.resolved_packages(root / "Cargo.toml")
            self.assertEqual(keep, {("probe", "0.1.0")})


if __name__ == "__main__":
    unittest.main()
