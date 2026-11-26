#!/usr/bin/env python3

import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
SCRIPT = Path(__file__).with_name("repo-lint.sh")
FAKE_TOOL = """#!/bin/sh
echo "$(basename "$0") $*" >> "$REPO_LINT_LOG"
case " $* " in
  *" ${REPO_LINT_FAIL_MANIFEST:-none} "*) exit 1 ;;
esac
"""
FAKE_RESOLVER = """import shutil
import sys
from pathlib import Path

output = Path(sys.argv[2])
output.mkdir(parents=True, exist_ok=True)
shutil.copy(Path(sys.argv[1]).with_name("Cargo.lock"), output / "Cargo.lock")
"""
WORKSPACE_FILES = (
    "backend/Cargo.toml",
    "backend/Cargo.lock",
    "backend/deny.toml",
    "agent/Cargo.toml",
    "agent/Cargo.lock",
    "tools/bench/Cargo.toml",
    "tools/bench/Cargo.lock",
    "tools/bench/deny.toml",
    "legacy/Cargo.lock",
    "backend/crates/scanner/tests/fixtures/cargo/Cargo.toml",
    "backend/crates/scanner/tests/fixtures/cargo/Cargo.lock",
    "backend/vendor/upstream/Cargo.toml",
    "backend/vendor/upstream/Cargo.lock",
    "agent/target/package/agent-0.1.0/Cargo.lock",
)


class RepoLintTest(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp()).resolve()
        self.addCleanup(shutil.rmtree, self.root)
        (self.root / "scripts").mkdir()
        shutil.copy(SCRIPT, self.root / "scripts" / "repo-lint.sh")
        (self.root / "scripts" / "osv-resolved-lock.py").write_text(FAKE_RESOLVER)
        (self.root / ".gitignore").write_text("target/\n")
        subprocess.run(["git", "init", "--quiet"], cwd=self.root, check=True)
        tools = Path(tempfile.mkdtemp()).resolve()
        self.addCleanup(shutil.rmtree, tools)
        self.log = tools / "calls.log"
        self.temporary = tools / "tmp"
        self.temporary.mkdir()
        bin_directory = tools / "bin"
        bin_directory.mkdir()
        for tool in ("cargo", "osv-scanner"):
            path = bin_directory / tool
            path.write_text(FAKE_TOOL)
            path.chmod(path.stat().st_mode | stat.S_IEXEC)
        self.environment = {
            **os.environ,
            "PATH": f"{bin_directory}{os.pathsep}{os.environ['PATH']}",
            "REPO_LINT_LOG": str(self.log),
            "TMPDIR": str(self.temporary),
        }

    def add_workspaces(self) -> None:
        for name in WORKSPACE_FILES:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("")

    def run_step(self, step: str, **environment: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["sh", "scripts/repo-lint.sh", step],
            cwd=self.root,
            env={**self.environment, **environment},
            check=False,
            capture_output=True,
            text=True,
        )

    def calls(self) -> list[str]:
        return self.log.read_text().splitlines() if self.log.exists() else []

    def test_osv_scans_every_cargo_lockfile_outside_fixtures_vendor_and_ignored_files(self) -> None:
        self.add_workspaces()

        result = self.run_step("osv")

        self.assertEqual(result.returncode, 0, result.stderr)
        [call] = self.calls()
        arguments = call.split()
        lockfiles = [arguments[i + 1] for i, flag in enumerate(arguments) if flag == "-L"]
        resolved = [
            Path(*Path(path).relative_to(self.temporary).parts[1:]).as_posix()
            for path in lockfiles
            if path.startswith(str(self.temporary))
        ]
        self.assertEqual(
            sorted(resolved),
            ["agent/Cargo.lock", "backend/Cargo.lock", "tools/bench/Cargo.lock"],
        )
        self.assertIn("legacy/Cargo.lock", lockfiles)
        self.assertEqual(len(lockfiles), 4)

    def test_deny_checks_each_workspace_with_its_own_config_or_the_backend_one(self) -> None:
        self.add_workspaces()

        result = self.run_step("deny")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            self.calls(),
            [
                "cargo deny --manifest-path backend/Cargo.toml --config backend/deny.toml check",
                "cargo deny --manifest-path agent/Cargo.toml --config backend/deny.toml check",
                "cargo deny --manifest-path tools/bench/Cargo.toml --config tools/bench/deny.toml check",
            ],
        )

    def test_deny_fails_when_one_workspace_fails_and_still_checks_the_others(self) -> None:
        self.add_workspaces()

        result = self.run_step("deny", REPO_LINT_FAIL_MANIFEST="agent/Cargo.toml")

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(self.calls()), 3)
        self.assertIn("cargo deny failed for: agent", result.stderr)

    def test_steps_without_rust_workspaces_pass_without_running_tools(self) -> None:
        for step, message in (
            ("osv", "no lockfile to scan"),
            ("deny", "no Rust workspace to check"),
        ):
            with self.subTest(step=step):
                result = self.run_step(step)

                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(message, result.stdout)
                self.assertEqual(self.calls(), [])


if __name__ == "__main__":
    unittest.main()
