import contextlib
import importlib.util
import io
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch


SOURCE = Path(__file__).resolve().parents[1] / "templates/mobile/mobile/scripts/qa/android-adb.py"
spec = importlib.util.spec_from_file_location("android_adb", SOURCE)
assert spec is not None and spec.loader is not None
android_adb = importlib.util.module_from_spec(spec)
spec.loader.exec_module(android_adb)


class AndroidAdbTimeoutTest(unittest.TestCase):
    def test_timeout_identifies_the_command_and_returns_124(self):
        command = ["adb", "-s", "emulator-5556", "shell", "echo", "two words"]
        stderr = io.StringIO()
        with patch.dict(android_adb.os.environ, {"BAUKIT_QA_ADB_TIMEOUT_SECONDS": "10"}), \
                patch.object(android_adb.sys, "argv", [str(SOURCE), *command]), \
                patch.object(android_adb.subprocess, "run", side_effect=subprocess.TimeoutExpired(command, 10)), \
                contextlib.redirect_stderr(stderr):
            self.assertEqual(android_adb.main(), 124)
        self.assertEqual(stderr.getvalue(), f"qa: ADB command timed out: {' '.join(command)}\n")

    def test_command_exit_status_is_preserved(self):
        command = ["adb", "devices"]
        with patch.dict(android_adb.os.environ, {"BAUKIT_QA_ADB_TIMEOUT_SECONDS": "10"}), \
                patch.object(android_adb.sys, "argv", [str(SOURCE), *command]), \
                patch.object(android_adb.subprocess, "run", return_value=subprocess.CompletedProcess(command, 7)) as run:
            self.assertEqual(android_adb.main(), 7)
        run.assert_called_once_with(command, timeout=10, check=False)
