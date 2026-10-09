import contextlib
import importlib.util
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import call, patch


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
    def probe(self, output="", code=0, timeout=False, diagnostics=None, diagnostics_code=0,
              diagnostics_timeout=False, report=None):
        stderr = io.StringIO()
        command = ["adb", "-s", "emulator-5556", "logcat", "-d", "-b", "events", "-s", "am_anr", "am_crash"]
        dumpsys = ["adb", "-s", "emulator-5556", "shell", "dumpsys", "activity", "lastanr"]
        result = subprocess.CompletedProcess(command, code, stdout=output)
        results = [subprocess.TimeoutExpired(command, 10) if timeout else result]
        calls = [call(command, capture_output=True, text=True, check=False, timeout=10)]
        if diagnostics is not None or diagnostics_code or diagnostics_timeout:
            details = subprocess.CompletedProcess(dumpsys, diagnostics_code, stdout=diagnostics or "")
            results.append(subprocess.TimeoutExpired(dumpsys, 10) if diagnostics_timeout else details)
            calls.append(call(dumpsys, capture_output=True, text=True, check=False, timeout=10))
        with patch.object(android_adb.subprocess, "run", side_effect=results) as run, contextlib.redirect_stderr(stderr):
            if report is None:
                status = android_adb.check_health("adb", "emulator-5556", 10)
            else:
                status = android_adb.check_health("adb", "emulator-5556", 10, report)
        self.assertEqual(run.call_args_list, calls)
        return status, stderr.getvalue()

    def test_anrs_and_crashes_in_system_ui_and_keyboards_fail(self):
        for event in ("am_anr", "am_crash"):
            for package in ("com.android.systemui", "com.google.android.inputmethod.latin", "com.android.inputmethod.latin"):
                with self.subTest(event=event, package=package):
                    status, message = self.probe(
                        f"10-06 12:00:00 {event}: [0,123,{package},1,reason]\n",
                        diagnostics="" if event == "am_anr" else None,
                    )
                    self.assertEqual(status, 1)
                    self.assertIn(f"Android system process is unhealthy: {package}", message)
                    self.assertIn(f"10-06 12:00:00 {event}: [0,123,{package},1,reason]\n", message)

    def test_all_triggering_events_and_anr_summary_are_retained(self):
        events = (
            "10-06 12:00:00 am_anr: [0,123,com.android.systemui,1,first reason]",
            "10-06 12:00:01 am_crash: [0,124,com.android.inputmethod.latin,1,second reason]",
            "10-06 12:00:02 am_anr: [0,123,com.android.systemui,1,third reason]",
        )
        summary = "ANR in com.android.systemui\nPID: 123\nReason: Input dispatching timed out\n"
        diagnostics = summary + "CPU usage: unrelated details\nprivate stack frame\n"
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "state/android-health.log"
            report.parent.mkdir()
            report.write_text("previous probe\n")
            status, stderr = self.probe("\n".join(events), diagnostics=diagnostics, report=report)
            self.assertEqual(status, 1)
            evidence = report.read_text()
        self.assertTrue(evidence.startswith("previous probe\n"))
        self.assertRegex(evidence, r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z emulator-5556 status=1\n")
        for event in events:
            self.assertIn(event, stderr)
            self.assertIn(event, evidence)
        self.assertIn(summary, stderr)
        self.assertTrue(evidence.endswith(stderr))
        self.assertNotIn("private stack frame", stderr)
        self.assertNotIn("CPU usage", evidence)

    def test_service_anr_summary_matches_the_reason_package(self):
        summary = (
            "ANR time: 2026-10-06 12:00:00\n"
            "PID: 123\nReason: executing service com.android.systemui/.SystemUIService\n"
            "ErrorId: 1234\nFrozen: false\n"
        )
        status, stderr = self.probe(
            "am_anr: [0,123,com.android.systemui,1,reason]", diagnostics=summary,
        )
        self.assertEqual(status, 1)
        self.assertIn(summary, stderr)

    def test_unrelated_anr_summary_is_not_retained(self):
        for package in ("dev.baukit.example", "com.android.systemui.extra", "com.android.inputmethod.latin"):
            with self.subTest(package=package):
                diagnostics = f"ANR in {package}\nPID: 456\nReason: unrelated reason\nstack: com.android.systemui\n"
                with tempfile.TemporaryDirectory() as directory:
                    report = Path(directory) / "state/android-health.log"
                    status, stderr = self.probe(
                        "am_anr: [0,123,com.android.systemui,1,reason]",
                        diagnostics=diagnostics, report=report,
                    )
                    self.assertEqual(status, 1)
                    self.assertIn("no matching critical-process ANR", stderr)
                    self.assertNotIn("PID: 456", stderr)
                    self.assertNotIn(package, report.read_text())

    def test_diagnostics_failure_and_timeout_keep_the_unhealthy_status(self):
        event = "am_anr: [0,123,com.android.systemui,1,reason]"
        for settings, message in (
            ({"diagnostics_code": 7}, "qa: Android ANR diagnostics failed: exit 7"),
            ({"diagnostics_timeout": True}, "qa: Android ANR diagnostics timed out"),
        ):
            with self.subTest(settings=settings), tempfile.TemporaryDirectory() as directory:
                report = Path(directory) / "android-health.log"
                status, stderr = self.probe(event, report=report, **settings)
                self.assertEqual(status, 1)
                self.assertIn(event, stderr)
                self.assertIn(message, stderr)
                self.assertIn(message, report.read_text())

    def test_crash_does_not_request_anr_diagnostics(self):
        event = "am_crash: [0,123,com.android.systemui,1,reason]"
        with patch.object(android_adb.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, stdout=event)) as run, \
                contextlib.redirect_stderr(io.StringIO()) as stderr:
            self.assertEqual(android_adb.check_health("adb", "emulator-5556", 10), 1)
        self.assertEqual(run.call_count, 1)
        self.assertIn(event, stderr.getvalue())

    def test_manual_health_invocation_without_report_keeps_evidence(self):
        event = "am_crash: [0,123,com.android.systemui,1,reason]"
        with patch.object(android_adb.sys, "argv", [str(SOURCE), "--check-health", "adb", "emulator-5556"]), \
                patch.object(android_adb.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, stdout=event)), \
                contextlib.redirect_stderr(io.StringIO()) as stderr:
            self.assertEqual(android_adb.main(), 1)
        self.assertIn(event, stderr.getvalue())

    def test_reports_record_probe_failure_timeout_and_healthy_status(self):
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "state/android-health.log"
            self.assertEqual(self.probe(code=7, report=report)[0], 7)
            self.assertEqual(self.probe(timeout=True, report=report)[0], 124)
            self.assertEqual(self.probe(report=report), (0, ""))
            evidence = report.read_text()
        self.assertIn("emulator-5556 status=7\nqa: Android system health probe failed\n", evidence)
        self.assertIn("emulator-5556 status=124\nqa: Android system health probe timed out\n", evidence)
        self.assertTrue(evidence.endswith("emulator-5556 status=0\n\n"))

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
