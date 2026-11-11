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
SCRIPT = Path(__file__).with_name("lockfiles.sh")
FAKE_TOOL = '#!/bin/sh\necho "tool $(basename "$0") $*" >> "$LOCKFILES_LOG"\n'
LOCAL_HOOK = """lockfile_run() {
  echo "run $1" >> "$LOCKFILES_LOG"
  "$@"
}

extra_lockfiles() {
  echo "extra" >> "$LOCKFILES_LOG"
}
"""


class LockfilesTest(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        (self.root / "scripts").mkdir()
        shutil.copy(SCRIPT, self.root / "scripts" / "lockfiles.sh")
        for directory in ("backend", "web", "mobile"):
            (self.root / directory).mkdir()
        self.log = self.root / "calls.log"
        bin_directory = self.root / "bin"
        bin_directory.mkdir()
        for tool in ("cargo", "corepack"):
            path = bin_directory / tool
            path.write_text(FAKE_TOOL)
            path.chmod(path.stat().st_mode | stat.S_IEXEC)
        self.environment = {
            **os.environ,
            "PATH": f"{bin_directory}{os.pathsep}{os.environ['PATH']}",
            "LOCKFILES_LOG": str(self.log),
        }

    def run_script(self) -> list[str]:
        subprocess.run(
            ["sh", "scripts/lockfiles.sh"],
            cwd=self.root,
            env=self.environment,
            check=True,
            capture_output=True,
        )
        return self.log.read_text().splitlines() if self.log.exists() else []

    def test_lockfile_commands_run_without_a_local_hook(self) -> None:
        calls = self.run_script()

        self.assertTrue(calls)
        self.assertTrue(all(call.startswith("tool ") for call in calls))

    def test_local_hook_wraps_every_command_and_adds_extra_lockfiles(self) -> None:
        (self.root / "scripts" / "lockfiles.local.sh").write_text(LOCAL_HOOK)

        calls = self.run_script()

        self.assertEqual(calls[-1], "extra")
        commands = calls[:-1]
        self.assertTrue(commands)
        self.assertEqual(len(commands) % 2, 0)
        for wrapper, tool in zip(commands[::2], commands[1::2], strict=True):
            self.assertTrue(wrapper.startswith("run "))
            self.assertEqual(wrapper.removeprefix("run "), tool.split()[1])


if __name__ == "__main__":
    unittest.main()
