import json
import os
from pathlib import Path
import re
import sys
import subprocess
import tempfile
import unittest


TEMPLATES = Path(__file__).resolve().parents[1] / "templates/mobile/mobile"


def render(source: str, backend: bool) -> str:
    flags = {"backend": backend, "auth_oidc": False}
    pattern = r"{% if context\.(\w+) %}((?:(?!{% if).)*?){% endif (?:%}|-%}\s*)"
    while "{% if" in source:
        def choose(match):
            yes, _, no = match[2].partition("{% else %}")
            return yes if flags[match[1]] else no
        source, count = re.subn(pattern, choose, source, flags=re.S)
        if not count:
            raise ValueError("unsupported template condition")
    for key, value in (("app_name", "test"), ("app_crate", "test"), ("app_env", "TEST")):
        source = source.replace("{{ context." + key + " }}", value)
    return source


class AndroidQaTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.mobile = self.root / "mobile"
        self.scripts = self.mobile / "scripts/qa"
        self.scripts.mkdir(parents=True)
        for name in ("android-env.sh", "services.sh", "android-adb.py", "run-maestro.sh"):
            (self.scripts / name).write_text(render((TEMPLATES / "scripts/qa" / name).read_text(), True))
        self.state = self.mobile / ".qa"
        self.state.mkdir()
        self.sdk = self.root / "sdk"
        self.tools = self.root / "tools"
        self.tools.mkdir()
        self.events = self.root / "events"
        self.attached = self.root / "attached"
        self.environment = {
            **os.environ,
            "PATH": f"{self.tools}:{os.environ['PATH']}",
            "ANDROID_HOME": str(self.sdk),
            "ANDROID_SDK_ROOT": str(self.sdk),
            "ANDROID_AVD_HOME": str(self.root / "avd"),
            "EVENTS": str(self.events),
            "ATTACHED": str(self.attached),
            "FAILURE": "",
            "BAUKIT_QA_ADB_TIMEOUT_SECONDS": "0.15",
            "BAUKIT_QA_SKIP_BUILD": "1",
        }
        self.environment.pop("BAUKIT_QA_RENDERER_THREADS", None)
        self.executable(self.sdk / "platform-tools/adb", r'''
printf 'adb %s\n' "$*" >> "$EVENTS"
case "$*" in
  devices)
    [[ "$FAILURE" != devices ]] || exit 7
    if [[ -f "$ATTACHED" ]]; then echo 'emulator-5556 device'; fi ;;
  *'emu avd name')
    [[ "$FAILURE" != avd ]] || exit 8
    if [[ "$FAILURE" == foreign ]]; then echo other-product; else echo test-qa; fi ;;
  *'emu kill')
    [[ "$FAILURE" != kill ]] || exit 9
    if [[ "$FAILURE" != disconnect ]]; then rm -f "$ATTACHED"; fi ;;
  *'shell am start '*) [[ "$FAILURE" != launch ]] || exit 17 ;;
  *'getprop sys.boot_completed') if grep -q '^{' "$EVENTS"; then echo 1; fi ;;
  *logcat*)
    if [[ "$FAILURE" == health ]]; then echo 'am_anr: [0,123,com.android.systemui,1,reason]'; fi ;;
  *'dumpsys activity lastanr')
    printf 'ANR in com.android.systemui\nPID: 123\nReason: Input dispatching timed out\nprivate stack frame\n' ;;
esac
exit 0
''')
        self.executable(self.tools / "docker", r'''
printf 'docker %s\n' "$*" >> "$EVENTS"
[[ "$FAILURE" != compose ]] || exit 10
''')
        self.executable(self.sdk / "emulator/emulator", '''
python3 - <<'RENDERER'
import json, os
from pathlib import Path
with Path(os.environ['EVENTS']).open('a') as output:
    output.write(json.dumps({'configuration': Path('SwiftShader.ini').read_text(), 'threads': os.environ['LP_NUM_THREADS']}) + '\\n')
RENDERER
''')

    def executable(self, path, body):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("#!/usr/bin/env bash\nset -eu\n" + body)
        path.chmod(0o755)

    def owned(self):
        (self.state / "android-owned").touch()
        (self.state / "android-serial").write_text("emulator-5556\n")
        self.attached.touch()

    def run_script(self, name, action, **environment):
        return subprocess.run(
            ["bash", str(self.scripts / name), action],
            env={**self.environment, **environment},
            capture_output=True, text=True, timeout=5, check=False,
        )

    def test_failed_cleanup_retains_ownership_and_reports_failure(self):
        for failure, code in (("devices", 7), ("avd", 8), ("foreign", 1), ("kill", 9), ("disconnect", 124), ("compose", 10)):
            with self.subTest(failure=failure):
                self.owned()
                self.events.unlink(missing_ok=True)
                result = self.run_script("android-env.sh", "stop", FAILURE=failure)
                self.assertEqual(result.returncode, code, result.stdout + result.stderr)
                self.assertTrue((self.state / "android-owned").exists())
                self.assertTrue((self.state / "android-serial").exists())
                self.assertIn("ownership state was kept", result.stderr)
                self.assertNotIn("Android environment stopped", result.stdout)
                self.assertNotIn("disposable services stopped", result.stdout if failure == "compose" else "")
                self.assertIn("down --volumes", self.events.read_text())
                if failure in ("devices", "avd", "foreign"):
                    self.assertNotIn("emu kill", self.events.read_text())

    def test_missing_sdk_preserves_ownership(self):
        self.owned()
        result = self.run_script("android-env.sh", "stop", ANDROID_HOME="", ANDROID_SDK_ROOT="")
        self.assertEqual(result.returncode, 1)
        self.assertTrue((self.state / "android-owned").exists())
        self.assertIn("cannot locate", result.stderr)

    def test_stale_ownership_is_cleared_without_killing_a_device(self):
        self.owned()
        self.attached.unlink()
        result = self.run_script("android-env.sh", "stop")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.state / "android-owned").exists())
        self.assertFalse((self.state / "android-serial").exists())
        self.assertNotIn("emu kill", self.events.read_text())

    def test_successful_cleanup_waits_for_disconnection(self):
        self.owned()
        result = self.run_script("android-env.sh", "stop")
        self.assertEqual(result.returncode, 0, result.stderr)
        events = self.events.read_text()
        self.assertGreater(events.rindex("adb devices"), events.index("emu kill"))
        self.assertFalse((self.state / "android-owned").exists())

    def prepare_start(self):
        self.executable(self.scripts / "services.sh", 'echo "services $*" >> "$EVENTS"\n')
        apk = self.mobile / "android/app/build/outputs/apk/release/app-release.apk"
        apk.parent.mkdir(parents=True)
        apk.touch()

    def test_restart_refreshes_devices_and_bounds_renderer_threads(self):
        self.prepare_start()
        for threads in ("4", "2"):
            with self.subTest(threads=threads):
                self.owned()
                self.events.unlink(missing_ok=True)
                result = self.run_script("android-env.sh", "start", **({} if threads == "4" else {"BAUKIT_QA_RENDERER_THREADS": threads}))
                self.assertEqual(result.returncode, 0, result.stderr)
                events = self.events.read_text().splitlines()
                kill = next(i for i, event in enumerate(events) if "emu kill" in event)
                start = events.index("services start")
                self.assertIn("adb devices", events[kill + 1:start])
                configuration = next(json.loads(event) for event in events if event.startswith("{"))
                self.assertEqual(configuration["threads"], threads)
                self.assertEqual(configuration["configuration"], f"[Processor]\nThreadCount={threads}\n[Testing]\nDisableServer=1\n")
                boot = next(i for i, event in enumerate(events) if "getprop" in event)
                health = next(i for i, event in enumerate(events) if "logcat" in event)
                install = next(i for i, event in enumerate(events) if "install -r" in event)
                self.assertLess(boot, health)
                self.assertLess(health, install)
                stopped = self.run_script("android-env.sh", "stop")
                self.assertEqual(stopped.returncode, 0, stopped.stderr)
                self.assertFalse((self.state / "renderer").exists())

    def test_start_launches_main_activity_and_waits_for_it(self):
        self.prepare_start()
        result = self.run_script("android-env.sh", "start")
        self.assertEqual(result.returncode, 0, result.stderr)
        events = self.events.read_text().splitlines()
        launch = "adb -s emulator-5556 shell am start -n dev.baukit.test/.MainActivity -W"
        self.assertIn(launch, events)
        install = next(i for i, event in enumerate(events) if "install -r" in event)
        self.assertLess(install, events.index(launch))
        self.assertFalse(any("monkey" in event for event in events))

    def test_failed_activity_launch_does_not_report_ready(self):
        self.prepare_start()
        result = self.run_script("android-env.sh", "start", FAILURE="launch")
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertNotIn("Android app ready", result.stdout)

    def test_invalid_renderer_limits_fail_before_starting_services(self):
        self.prepare_start()
        for threads in ("0", "-1", "1.5", "four"):
            with self.subTest(threads=threads):
                result = self.run_script("android-env.sh", "start", BAUKIT_QA_RENDERER_THREADS=threads)
                self.assertEqual(result.returncode, 2)
                self.assertIn("must be a positive integer", result.stderr)
                self.assertFalse(self.events.exists())

    def test_unhealthy_boot_keeps_ownership_and_does_not_install(self):
        self.prepare_start()
        result = self.run_script("android-env.sh", "start", FAILURE="health")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("system process is unhealthy: com.android.systemui", result.stderr)
        self.assert_health_evidence(result)
        self.assertTrue((self.state / "android-owned").exists())
        self.assertNotIn("install -r", self.events.read_text())

    def test_live_flows_recheck_health_before_maestro(self):
        self.owned()
        (self.mobile / ".maestro").mkdir()
        self.executable(self.tools / "maestro", 'echo maestro >> "$EVENTS"\n')
        result = self.run_script("run-maestro.sh", "android", FAILURE="health")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assert_health_evidence(result)
        self.assertNotIn("maestro", self.events.read_text())

    def assert_health_evidence(self, result):
        event = "am_anr: [0,123,com.android.systemui,1,reason]"
        self.assertIn(event, result.stderr)
        self.assertIn("Reason: Input dispatching timed out", result.stderr)
        report = (self.state / "android-health.log").read_text()
        self.assertRegex(report, r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z emulator-5556 status=1\n")
        self.assertIn(event, report)
        self.assertIn("Reason: Input dispatching timed out", report)
        self.assertNotIn("private stack frame", report)

    def test_backend_invalid_pid_is_kept_and_compose_cleanup_is_attempted(self):
        (self.state / "backend.pid").write_text("invalid\n")
        result = self.run_script("services.sh", "stop")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertTrue((self.state / "backend.pid").exists())
        self.assertIn("down --volumes", self.events.read_text())
        self.assertNotIn("disposable services stopped", result.stdout)

    def test_backend_that_ignores_termination_keeps_its_pid(self):
        process = subprocess.Popen(
            [sys.executable, "-c", "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); print('ready', flush=True); time.sleep(60)"],
            stdout=subprocess.PIPE, text=True,
        )
        self.addCleanup(process.stdout.close)
        self.addCleanup(process.wait)
        self.addCleanup(lambda: process.kill() if process.poll() is None else None)
        self.assertEqual(process.stdout.readline().strip(), "ready")
        (self.state / "backend.pid").write_text(f"{process.pid}\n")
        self.executable(self.tools / "sleep", "exit 0\n")
        result = self.run_script("services.sh", "stop")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("did not stop", result.stderr)
        self.assertEqual((self.state / "backend.pid").read_text(), f"{process.pid}\n")
        self.assertIsNone(process.poll())

    def test_android_make_target_preserves_flow_and_cleanup_failures(self):
        recipes = []
        for path in (TEMPLATES / "Makefile", TEMPLATES.parents[1] / "backend/Makefile"):
            source = path.read_text()
            recipes.append(source[source.index("e2e-android:\n"):source.index("\ne2e-ios:")])
        self.executable(self.tools / "make", r'''
case "$*" in
  *qa-android-down*) exit "${DOWN_STATUS:-0}" ;;
  *e2e-android-live*) exit "${FLOW_STATUS:-0}" ;;
esac
''')
        for recipe, flow, down, expected in (
            (recipe, flow, down, expected)
            for recipe in recipes
            for flow, down, expected in ((0, 0, 0), (7, 0, 7), (0, 9, 9))
        ):
            with self.subTest(recipe=recipe, flow=flow, down=down):
                (self.mobile / "Makefile").write_text(recipe)
                result = subprocess.run(
                    ["/usr/bin/make", "--no-print-directory", "-C", str(self.mobile), "e2e-android", f"MAKE={self.tools / 'make'}"],
                    env={**self.environment, "FLOW_STATUS": str(flow), "DOWN_STATUS": str(down)},
                    capture_output=True, text=True, check=False, timeout=5,
                )
                self.assertEqual(result.returncode, 0 if expected == 0 else 2, result.stderr)
                if expected:
                    self.assertIn(f"Error {expected}", result.stderr)
