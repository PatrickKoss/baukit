#!/usr/bin/env python3
"""Runs release.sh and promote.sh with --dry-run in a scratch clone of the GitOps files."""

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
COPIED = ["scripts", "deploy/gitops", "deploy/cosign-signing-config.json"]
TESTING = "deploy/gitops/testing/release.yaml"
PRODUCTION = "deploy/gitops/production/release.yaml"


def git(root: Path, *arguments: str) -> str:
    return subprocess.run(
        ["git", "-C", str(root), *arguments], check=True, capture_output=True, text=True
    ).stdout.strip()


def tag(root: Path, release_file: str) -> str:
    return subprocess.run(
        [sys.executable, str(root / "scripts/gitops-pins.py"), "tag", str(root / release_file)],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


class ReleaseDryRunTest(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        for relative in COPIED:
            source, target = ROOT / relative, self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            if source.is_dir():
                shutil.copytree(source, target, ignore=shutil.ignore_patterns("__pycache__"))
            else:
                shutil.copy2(source, target)
        git(self.root, "init", "--quiet", "--initial-branch=main")
        git(self.root, "add", ".")
        git(
            self.root,
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        )

    def tearDown(self) -> None:
        self.directory.cleanup()

    def script(self, name: str, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(self.root / "scripts" / name), "--dry-run", *arguments],
            cwd=self.root,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_release_pins_head_in_testing_and_promote_copies_it(self) -> None:
        source = git(self.root, "rev-parse", "--short=12", "HEAD")
        release = self.script("release.sh")
        self.assertEqual(release.returncode, 0, release.stderr)
        self.assertEqual(tag(self.root, TESTING), source)
        self.assertIn("cosign sign", release.stderr)
        self.assertIn("--target api", release.stderr)
        self.assertEqual(git(self.root, "status", "--porcelain"), "")

        refused = self.script("promote.sh", "0" * 12)
        self.assertNotEqual(refused.returncode, 0)

        promote = self.script("promote.sh", source)
        self.assertEqual(promote.returncode, 0, promote.stderr)
        self.assertIn("cosign verify", promote.stderr)
        self.assertEqual(tag(self.root, PRODUCTION), source)
        self.assertIn("promote", git(self.root, "log", "-1", "--format=%s"))

    def test_release_refuses_a_dirty_tree(self) -> None:
        (self.root / "untracked").write_text("x\n", encoding="utf-8")
        self.assertEqual(self.script("release.sh").returncode, 2)


if __name__ == "__main__":
    unittest.main()
