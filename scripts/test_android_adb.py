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


class AndroidHealthTests(unittest.TestCase):
    def probe(self, output="", code=0, timeout=False):
        stderr = io.StringIO()
        command = ["adb", "-s", "emulator-5556", "logcat", "-d", "-b", "events", "-s", "am_anr", "am_crash"]
        result = subprocess.CompletedProcess(command, code, stdout=output)
        with patch.object(android_adb.subprocess, "run", side_effect=subprocess.TimeoutExpired(command, 10) if timeout else None, return_value=result) as run, contextlib.redirect_stderr(stderr):
            status = android_adb.check_health("adb", "emulator-5556", 10)
        run.assert_called_once_with(command, capture_output=True, text=True, check=False, timeout=10)
        return status, stderr.getvalue()

    def test_anrs_and_crashes_in_system_ui_and_keyboards_fail(self):
        for event in ("am_anr", "am_crash"):
            for package in ("com.android.systemui", "com.google.android.inputmethod.latin", "com.android.inputmethod.latin"):
                with self.subTest(event=event, package=package):
                    status, message = self.probe(f"10-06 12:00:00 {event}: [0,123,{package},1,reason]\n")
                    self.assertEqual(status, 1)
                    self.assertIn(f"Android system process is unhealthy: {package}", message)

    def test_healthy_system_and_unrelated_events_pass(self):
        for output in ("", "--------- beginning of events\n", "am_anr: [0,123,dev.baukit.example,1,reason]\n"):
            with self.subTest(output=output):
                self.assertEqual(self.probe(output), (0, ""))

    def test_probe_failure_and_timeout_fail_closed(self):
        self.assertEqual(self.probe(code=7), (7, "qa: Android system health probe failed\n"))
        self.assertEqual(self.probe(timeout=True), (124, "qa: Android system health probe timed out\n"))

    def test_shutdown_timeout_is_bounded(self):
        attached = subprocess.CompletedProcess(["adb", "devices"], 0, stdout="emulator-5556 device\n")
        stderr = io.StringIO()
        with patch.object(android_adb.subprocess, "run", return_value=attached), \
                patch.object(android_adb.time, "monotonic", side_effect=[0, 0, 0, 11]), \
                patch.object(android_adb.time, "sleep"), contextlib.redirect_stderr(stderr):
            self.assertEqual(android_adb.wait_for_stop("adb", "emulator-5556", 10), 124)
        self.assertIn("did not stop before the ADB deadline", stderr.getvalue())
