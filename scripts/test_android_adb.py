import contextlib
import importlib.util
import io
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
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
        with (
            patch.dict(android_adb.os.environ, {"BAUKIT_QA_ADB_TIMEOUT_SECONDS": "10"}),
            patch.object(android_adb.sys, "argv", [str(SOURCE), *command]),
            patch.object(
                android_adb.subprocess, "run", side_effect=subprocess.TimeoutExpired(command, 10)
            ),
            contextlib.redirect_stderr(stderr),
        ):
            self.assertEqual(android_adb.main(), 124)
        self.assertEqual(stderr.getvalue(), f"qa: ADB command timed out: {' '.join(command)}\n")

    def test_command_exit_status_is_preserved(self):
        command = ["adb", "devices"]
        with (
            patch.dict(android_adb.os.environ, {"BAUKIT_QA_ADB_TIMEOUT_SECONDS": "10"}),
            patch.object(android_adb.sys, "argv", [str(SOURCE), *command]),
            patch.object(
                android_adb.subprocess, "run", return_value=subprocess.CompletedProcess(command, 7)
            ) as run,
        ):
            self.assertEqual(android_adb.main(), 7)
        run.assert_called_once_with(command, timeout=10, check=False)


class AndroidHealthTests(unittest.TestCase):
    def probe(
        self,
        output="",
        *,
        code=0,
        timeout=False,
        diagnostics=None,
        diagnostics_code=0,
        diagnostics_timeout=False,
        report=None,
    ):
        stderr = io.StringIO()
        command = [
            "adb",
            "-s",
            "emulator-5556",
            "logcat",
            "-d",
            "-b",
            "events",
            "-s",
            "am_anr",
            "am_crash",
        ]
        dumpsys = ["adb", "-s", "emulator-5556", "shell", "dumpsys", "activity", "lastanr"]
        result = subprocess.CompletedProcess(command, code, stdout=output)
        results = [subprocess.TimeoutExpired(command, 10) if timeout else result]
        calls = [call(command, capture_output=True, text=True, check=False, timeout=10)]
        if not timeout and code == 0:
            details = subprocess.CompletedProcess(
                dumpsys, diagnostics_code, stdout=diagnostics or ""
            )
            results.append(
                subprocess.TimeoutExpired(dumpsys, 10) if diagnostics_timeout else details
            )
            calls.append(call(dumpsys, capture_output=True, text=True, check=False, timeout=10))
        with (
            patch.object(android_adb.subprocess, "run", side_effect=results) as run,
            contextlib.redirect_stderr(stderr),
        ):
            if report is None:
                status = android_adb.check_health("adb", "emulator-5556", 10)
            else:
                status = android_adb.check_health("adb", "emulator-5556", 10, report)
        self.assertEqual(run.call_args_list, calls)
        return status, stderr.getvalue()

    def test_anrs_and_crashes_in_system_ui_and_keyboards_fail(self):
        for event in ("am_anr", "am_crash"):
            for package in (
                "com.android.systemui",
                "com.google.android.inputmethod.latin",
                "com.android.inputmethod.latin",
            ):
                with self.subTest(event=event, package=package):
                    status, message = self.probe(
                        f"10-06 12:00:00 {event}: [0,123,{package},1,reason]\n"
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
            "am_anr: [0,123,com.android.systemui,1,reason]",
            diagnostics=summary,
        )
        self.assertEqual(status, 1)
        self.assertIn(summary, stderr)

    def test_unrelated_anr_summary_is_not_retained(self):
        for package in ("dev.baukit.example", "com.android.systemui.extra"):
            with self.subTest(package=package):
                diagnostics = f"ANR in {package}\nPID: 456\nReason: unrelated reason\nstack: com.android.systemui\n"
                with tempfile.TemporaryDirectory() as directory:
                    report = Path(directory) / "state/android-health.log"
                    status, stderr = self.probe(
                        "am_anr: [0,123,com.android.systemui,1,reason]",
                        diagnostics=diagnostics,
                        report=report,
                    )
                    self.assertEqual(status, 1)
                    self.assertIn("dumpsys lastanr has no record of it", stderr)
                    self.assertNotIn("PID: 456", stderr)
                    self.assertNotIn(package, report.read_text())

    def test_dumpsys_anr_in_another_critical_package_is_reported(self):
        diagnostics = "ANR in com.android.inputmethod.latin\nPID: 456\nReason: keyboard\n"
        status, stderr = self.probe(
            "am_anr: [0,123,com.android.systemui,1,reason]", diagnostics=diagnostics
        )
        self.assertEqual(status, 1)
        self.assertIn("dumpsys lastanr reports an ANR in com.android.inputmethod.latin", stderr)
        self.assertIn("PID: 456", stderr)

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

    def test_crash_with_quiet_dumpsys_reports_only_the_crash(self):
        event = "am_crash: [0,123,com.android.systemui,1,reason]"
        status, stderr = self.probe(event, diagnostics="")
        self.assertEqual(status, 1)
        self.assertIn(event, stderr)
        self.assertNotIn("dumpsys", stderr)

    def test_dumpsys_anr_fails_when_logcat_rotated(self):
        summary = "ANR in com.android.systemui\nPID: 123\nReason: Input dispatching timed out\n"
        status, stderr = self.probe("--------- beginning of events\n", diagnostics=summary)
        self.assertEqual(status, 1)
        self.assertIn("dumpsys lastanr reports an ANR in com.android.systemui", stderr)
        self.assertIn(summary, stderr)

    def test_logcat_anr_without_dumpsys_record_still_fails(self):
        event = "11-02 10:18:44.491 698 2117 I am_anr  : [0,984,com.android.systemui,550026765,executing service com.android.systemui/.SystemUIService, waited 20092ms]"
        status, stderr = self.probe(event, diagnostics="")
        self.assertEqual(status, 1)
        self.assertIn(
            "logcat am_anr shows the ANR above; dumpsys lastanr has no record of it", stderr
        )

    def test_manual_health_invocation_without_report_keeps_evidence(self):
        event = "am_crash: [0,123,com.android.systemui,1,reason]"
        with (
            patch.object(
                android_adb.sys, "argv", [str(SOURCE), "--check-health", "adb", "emulator-5556"]
            ),
            patch.object(
                android_adb.subprocess,
                "run",
                side_effect=[
                    subprocess.CompletedProcess([], 0, stdout=event),
                    subprocess.CompletedProcess([], 0, stdout=""),
                ],
            ),
            contextlib.redirect_stderr(io.StringIO()) as stderr,
        ):
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
        self.assertIn(
            "emulator-5556 status=124\nqa: Android system health probe timed out\n", evidence
        )
        self.assertTrue(evidence.endswith("emulator-5556 status=0\n\n"))

    def test_healthy_system_and_unrelated_events_pass(self):
        for output in (
            "",
            "--------- beginning of events\n",
            "am_anr: [0,123,dev.baukit.example,1,reason]\n",
        ):
            with self.subTest(output=output):
                self.assertEqual(self.probe(output), (0, ""))

    def test_probe_failure_and_timeout_fail_closed(self):
        self.assertEqual(self.probe(code=7), (7, "qa: Android system health probe failed\n"))
        self.assertEqual(
            self.probe(timeout=True), (124, "qa: Android system health probe timed out\n")
        )

    def test_shutdown_timeout_is_bounded(self):
        attached = subprocess.CompletedProcess(
            ["adb", "devices"], 0, stdout="emulator-5556 device\n"
        )
        stderr = io.StringIO()
        with (
            patch.object(android_adb.subprocess, "run", return_value=attached),
            patch.object(android_adb.time, "monotonic", side_effect=[0, 0, 0, 11]),
            patch.object(android_adb.time, "sleep"),
            contextlib.redirect_stderr(stderr),
        ):
            self.assertEqual(android_adb.wait_for_stop("adb", "emulator-5556", 10), 124)
        self.assertIn("did not stop before the ADB deadline", stderr.getvalue())


def stat(busy: int, idle: int, cores: int = 4) -> str:
    lines = [f"cpu  {busy} 0 0 {idle} 0 0 0 0 0 0"]
    lines += [f"cpu{index} 0 0 0 0 0 0 0 0 0 0" for index in range(cores)]
    return "\n".join(lines) + "\n"


class FakeClock:
    def __init__(self):
        self.now = 0.0

    def __call__(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


class AndroidSettleTests(unittest.TestCase):
    def settle(self, samples, attached=0, timeout=60):
        clock = FakeClock()
        commands = []
        last = samples[-1]

        def shell(adb, serial, timeout, *command):
            commands.append(command)
            if command[0] == "cmd":
                clock.now += 5
                return ""
            return samples.pop(0) if samples else last

        output = io.StringIO()
        error = io.StringIO()
        with (
            patch.object(android_adb, "shell", side_effect=shell),
            patch.object(android_adb, "attached", return_value=attached),
            patch.dict(android_adb.os.environ, {"BAUKIT_QA_POLL_SECONDS": "2"}),
            contextlib.redirect_stdout(output),
            contextlib.redirect_stderr(error),
        ):
            status = android_adb.settle(
                "adb",
                "emulator-5556",
                10,
                timeout,
                timing=android_adb.Timing(clock=clock, sleep=clock.sleep),
            )
        return status, output.getvalue(), error.getvalue(), commands, clock.now

    def test_quiet_device_settles_after_dexopt(self):
        status, output, _, commands, _ = self.settle([stat(0, 0), stat(200, 1000)])
        self.assertEqual(status, 0)
        self.assertEqual(commands[0], ("cmd", "package", "bg-dexopt-job"))
        self.assertEqual(output, "qa: emulator-5556 settled in 15 s\n")

    def test_busy_window_is_not_quiet(self):
        status, output, _, _, _ = self.settle([stat(0, 0), stat(3000, 1000), stat(3100, 2000)])
        self.assertEqual(status, 0)
        self.assertEqual(output, "qa: emulator-5556 settled in 25 s\n")

    def test_busy_device_saves_anyway_after_the_deadline(self):
        samples = [stat(busy * 1000, busy * 100) for busy in range(1, 50)]
        status, output, _, _, now = self.settle(samples, timeout=60)
        self.assertEqual(status, 0)
        self.assertEqual(
            output, "qa: emulator-5556 did not settle within 60 s; saving the snapshot anyway\n"
        )
        self.assertEqual(now, 60)

    def test_lost_device_fails(self):
        status, output, error, _, _ = self.settle([None], attached=1)
        self.assertEqual(status, 1)
        self.assertEqual(output, "")
        self.assertEqual(error, "qa: lost emulator-5556 while it settled\n")

    def test_failed_probe_on_an_attached_device_keeps_waiting(self):
        status, output, _, _, _ = self.settle([stat(0, 0), None, stat(10, 990), stat(20, 1980)])
        self.assertEqual(status, 0)
        self.assertEqual(output, "qa: emulator-5556 settled in 35 s\n")

    def test_busy_cores_scale_with_the_guest_core_count(self):
        before = android_adb.cpu_counters(stat(0, 0, cores=4))
        after = android_adb.cpu_counters(stat(50, 50, cores=4))
        self.assertEqual(android_adb.busy_cores(before, after), 2.0)
        self.assertEqual(android_adb.busy_cores(after, after), 0.0)
        self.assertIsNone(android_adb.cpu_counters("garbage"))

    def test_settle_deadline_rejects_nonpositive_values(self):
        with (
            patch.dict(android_adb.os.environ, {"BAUKIT_QA_SETTLE_TIMEOUT_SECONDS": "0"}),
            patch.object(
                android_adb.sys, "argv", ["android-adb.py", "--settle", "adb", "emulator-5556"]
            ),
            self.assertRaisesRegex(
                ValueError, "BAUKIT_QA_SETTLE_TIMEOUT_SECONDS must be a positive finite number"
            ),
        ):
            android_adb.main()


class AndroidSnapshotTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.sdk = self.root / "sdk"
        self.image = self.sdk / "system-images/android-36/google_apis/x86_64"
        self.image.mkdir(parents=True)
        (self.image / "source.properties").write_text("Pkg.Revision=7\n")
        self.config = self.root / "config.ini"
        self.config.write_text(
            "image.sysdir.1=system-images/android-36/google_apis/x86_64/\n"
            "hw.ramSize=2048\nhw.cpu.ncore=4\navd.ini.displayname=test\n"
        )

    def name(self, gpu="swiftshader_indirect", features="-WiFiPacketStream", version="0.17.1"):
        return android_adb.snapshot_name(self.config, self.sdk, gpu, features, version)

    def test_same_inputs_give_the_same_name(self):
        self.assertEqual(self.name(), self.name())
        self.assertRegex(self.name(), r"^baukit-clean-[0-9a-f]{12}$")

    def test_each_input_changes_the_name(self):
        original = self.name()
        self.assertNotEqual(self.name(gpu="swiftshader"), original)
        self.assertNotEqual(self.name(version="0.17.2"), original)
        self.assertNotEqual(self.name(features=""), original)
        (self.image / "source.properties").write_text("Pkg.Revision=8\n")
        self.assertNotEqual(self.name(), original)
        (self.image / "source.properties").write_text("Pkg.Revision=7\n")
        self.assertEqual(self.name(), original)
        text = self.config.read_text()
        self.config.write_text(text.replace("hw.ramSize=2048", "hw.ramSize=4096"))
        self.assertNotEqual(self.name(), original)
        self.config.write_text(text.replace("android-36", "android-35"))
        self.assertNotEqual(self.name(), original)

    def test_non_hardware_settings_and_order_keep_the_name(self):
        original = self.name()
        lines = self.config.read_text().splitlines()
        self.config.write_text(
            "\n".join(reversed(lines)).replace("displayname=test", "displayname=other") + "\n"
        )
        self.assertEqual(self.name(), original)

    def snapshot(self, avd, name, ready=True, age=0):
        directory = avd / "snapshots" / name
        directory.mkdir(parents=True)
        os.utime(directory, (1_000_000 - age, 1_000_000 - age))
        if ready:
            (avd / f"{name}.ready").touch()

    def prune(self, avd, keep, loaded=frozenset()):
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(android_adb.prune_snapshots(avd, keep, set(loaded)), 0)
        return (
            sorted(path.name for path in (avd / "snapshots").iterdir()),
            sorted(path.name for path in avd.glob("*.ready")),
        )

    def test_prune_removes_older_baukit_snapshots_but_keeps_the_newest_complete_one(self):
        avd = self.root / "test-qa.avd"
        self.snapshot(avd, "baukit-clean", ready=False)
        self.snapshot(avd, "baukit-clean-aaaaaaaaaaaa", age=20)
        self.snapshot(avd, "baukit-clean-bbbbbbbbbbbb", age=10)
        self.snapshot(avd, "baukit-clean-cccccccccccc", ready=False, age=1)
        self.snapshot(avd, "baukit-clean-dddddddddddd")
        (avd / "snapshots/default_boot").mkdir()
        (avd / "baukit-clean-eeeeeeeeeeee.ready").touch()
        self.assertEqual(
            self.prune(avd, "baukit-clean-dddddddddddd"),
            (
                ["baukit-clean-bbbbbbbbbbbb", "baukit-clean-dddddddddddd", "default_boot"],
                ["baukit-clean-bbbbbbbbbbbb.ready", "baukit-clean-dddddddddddd.ready"],
            ),
        )
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(
                android_adb.prune_snapshots(self.root / "missing", "baukit-clean-x", set()), 0
            )

    def test_prune_keeps_snapshots_that_running_emulators_loaded(self):
        avd = self.root / "test-qa.avd"
        self.snapshot(avd, "baukit-clean-aaaaaaaaaaaa", age=20)
        self.snapshot(avd, "baukit-clean-bbbbbbbbbbbb", age=10)
        self.snapshot(avd, "baukit-clean-dddddddddddd")
        self.assertEqual(
            self.prune(avd, "baukit-clean-dddddddddddd", {"baukit-clean-aaaaaaaaaaaa"})[0],
            [
                "baukit-clean-aaaaaaaaaaaa",
                "baukit-clean-bbbbbbbbbbbb",
                "baukit-clean-dddddddddddd",
            ],
        )

    def test_loaded_snapshots_reads_emulator_command_lines_for_the_avd(self):
        proc = self.root / "proc"
        for pid, args in (
            (
                "10",
                [
                    "emulator",
                    "-avd",
                    "test-qa",
                    "-port",
                    "5790",
                    "-snapshot",
                    "baukit-clean-aaaaaaaaaaaa",
                ],
            ),
            ("11", ["qemu", "-avd", "other-qa", "-snapshot", "baukit-clean-bbbbbbbbbbbb"]),
            ("12", ["emulator", "-avd", "test-qa", "-no-snapshot-load"]),
            ("self", ["python3"]),
        ):
            (proc / pid).mkdir(parents=True)
            (proc / pid / "cmdline").write_bytes("\0".join(args).encode() + b"\0")
        self.assertEqual(
            android_adb.loaded_snapshots("test-qa", proc), {"baukit-clean-aaaaaaaaaaaa"}
        )
