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
    flags = {"backend": backend, "auth_oidc": False, "auth_enabled": False}
    pattern = r"{% if context\.(\w+) %}((?:(?!{% if).)*?){% endif (?:%}|-%}\s*)"
    while "{% if" in source:
        def choose(match):
            yes, _, no = match[2].partition("{% else %}")
            return yes if flags[match[1]] else no
        source, count = re.subn(pattern, choose, source, flags=re.S)
        if not count:
            raise ValueError("unsupported template condition")
    for key, value in (("app_name", "test"), ("app_crate", "test"), ("app_env", "TEST"), ("template_version", "test")):
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
            "BAUKIT_QA_POLL_SECONDS": "0",
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
    if [[ "$FAILURE" == health ]]; then
      printf 'ANR in com.android.systemui\nPID: 123\nReason: Input dispatching timed out\nprivate stack frame\n'
    fi ;;
  *'get-state')
    if [[ -f "$ATTACHED" ]]; then echo device; else exit 1; fi ;;
  *'pidof com.android.systemui') echo 321 ;;
  *'dumpsys window displays') echo '  mCurrentFocus=Window{1 u0 com.android.launcher3/.Launcher}' ;;
  *'cat /proc/stat') printf 'cpu  10 0 10 80 0 0 0 0 0 0\ncpu0 10 0 10 80 0 0 0 0 0 0\n' ;;
  *'ip -4 route get 10.0.2.2')
    if [[ "$FAILURE" != wifi ]]; then echo '10.0.2.2 dev wlan0 src 10.0.2.16 uid 0'; else echo '10.0.2.2 dev eth0 src 10.0.2.15 uid 0'; fi ;;
  *'emu avd snapshot save '*) mkdir -p "$ANDROID_AVD_HOME/test-qa.avd/snapshots/${@: -1}" ;;
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

    def test_first_boot_is_cold_and_saves_a_snapshot_later_boots_load(self):
        self.prepare_start()
        for expected in ("-no-snapshot-load", "-snapshot baukit-clean"):
            with self.subTest(expected=expected):
                self.events.unlink(missing_ok=True)
                self.executable(self.sdk / "emulator/emulator", '{ flock 9; printf \'{"args": "%s"}\\n\' "$*" >> "$EVENTS"; } 9>"$EVENTS.lock"\n')
                result = self.run_script("android-env.sh", "start")
                self.assertEqual(result.returncode, 0, result.stderr)
                events = self.events.read_text().splitlines()
                launch = next(json.loads(event)["args"] for event in events if event.startswith("{"))
                self.assertIn(expected, launch)
                self.assertIn("-no-snapshot-save", launch)
                saved = [i for i, event in enumerate(events) if "snapshot save baukit-clean" in event]
                if expected == "-no-snapshot-load":
                    health = next(i for i, event in enumerate(events) if "lastanr" in event)
                    self.assertEqual(len(saved), 1)
                    self.assertLess(health, saved[0])
                else:
                    self.assertEqual(saved, [])
                self.assertEqual(self.run_script("android-env.sh", "stop").returncode, 0)

    def test_snapshot_settles_clears_logcat_and_replaces_stale_or_unfinished_snapshots(self):
        self.prepare_start()
        avd = self.root / "avd/test-qa.avd"
        (avd / "snapshots/baukit-clean").mkdir(parents=True)
        (avd / "baukit-clean-000000000000.ready").touch()
        name = subprocess.run(
            [sys.executable, str(self.scripts / "android-adb.py"), "--snapshot-name",
             str(avd / "config.ini"), str(self.sdk), "swiftshader_indirect", "-WiFiPacketStream", "test"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
        (avd / "snapshots" / name).mkdir()
        (avd / "snapshots" / name / "partial").touch()
        result = self.run_script("android-env.sh", "start")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"qa: saved quick-boot snapshot {name}", result.stdout)
        self.assertEqual(sorted(path.name for path in (avd / "snapshots").iterdir()), [name])
        self.assertEqual(sorted(path.name for path in avd.glob("*.ready")), [f"{name}.ready"])
        self.assertFalse((avd / "snapshots" / name / "partial").exists())
        events = self.events.read_text().splitlines()
        order = [
            next(i for i, event in enumerate(events) if marker in event)
            for marker in ("bg-dexopt-job", "cat /proc/stat", "logcat -b all -c", f"snapshot save {name}")
        ]
        self.assertEqual(order, sorted(order))
        self.assertGreater(order[1], max(i for i, event in enumerate(events[: order[0]]) if "lastanr" in event))
        self.assertEqual(self.run_script("android-env.sh", "stop").returncode, 0)

    def test_emulators_launch_without_wifi_packet_stream_and_wait_for_the_wifi_route(self):
        self.prepare_start()
        self.executable(self.sdk / "emulator/emulator", '{ flock 9; printf \'{"args": "%s"}\\n\' "$*" >> "$EVENTS"; } 9>"$EVENTS.lock"\n')
        result = self.run_script("android-env.sh", "start")
        self.assertEqual(result.returncode, 0, result.stderr)
        events = self.events.read_text().splitlines()
        launches = [json.loads(event)["args"] for event in events if event.startswith("{")]
        self.assertTrue(launches)
        for launch in launches:
            self.assertIn("-feature -WiFiPacketStream", launch)
        route = next(i for i, event in enumerate(events) if "route get 10.0.2.2" in event)
        self.assertLess(next(i for i, event in enumerate(events) if "svc wifi enable" in event), route)
        self.assertLess(route, next(i for i, event in enumerate(events) if "snapshot save" in event))
        self.assertEqual(self.run_script("android-env.sh", "stop").returncode, 0)

    def test_missing_wifi_route_fails_the_start(self):
        self.prepare_start()
        result = self.run_script("android-env.sh", "start", FAILURE="wifi", BAUKIT_QA_WIFI_TIMEOUT_SECONDS="0.2")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("has no Wi-Fi route to 10.0.2.2; offline flows need Wi-Fi", result.stderr)
        self.assertNotIn("snapshot save", self.events.read_text())
        self.run_script("android-env.sh", "stop")

    def test_boot_waits_for_systemui_idle_before_health(self):
        self.prepare_start()
        result = self.run_script("android-env.sh", "start")
        self.assertEqual(result.returncode, 0, result.stderr)
        events = [event for event in self.events.read_text().splitlines() if "emulator-5556" in event]
        events = events[: next(i for i, event in enumerate(events) if "emu kill" in event)]
        boot = next(i for i, event in enumerate(events) if "getprop" in event)
        idle = [i for i, event in enumerate(events) if "dumpsys window displays" in event]
        health = next(i for i, event in enumerate(events) if "logcat" in event)
        self.assertGreaterEqual(len(idle), 3)
        self.assertLess(boot, idle[0])
        self.assertLess(idle[-1], health)

    def test_port_comes_from_manifest_and_environment_overrides_it(self):
        self.prepare_start()
        (self.root / "baukit.toml").write_text("[quality]\nandroid_emulator_port = 5600\n")
        for environment, serial in (({}, "emulator-5600"), ({"BAUKIT_QA_ANDROID_PORT": "5610"}, "emulator-5610")):
            with self.subTest(serial=serial):
                self.events.unlink(missing_ok=True)
                result = self.run_script("android-env.sh", "start", **environment)
                self.assertEqual(result.returncode, 0, result.stderr)
                port = int(serial.removeprefix("emulator-"))
                self.assertEqual((self.state / "android-serial").read_text(), f"{serial}\nemulator-{port + 2}\n")
                self.assertIn(f"adb -s {serial} install -r", self.events.read_text())
                self.attached.touch()
                self.run_script("android-env.sh", "stop")

    def test_emulator_count_comes_from_manifest_and_extra_emulators_are_read_only(self):
        self.prepare_start()
        (self.root / "baukit.toml").write_text("[quality]\nandroid_emulator_port = 5600\nandroid_emulators = 3\n")
        self.executable(self.sdk / "emulator/emulator", '{ flock 9; printf \'{"args": "%s"}\\n\' "$*" >> "$EVENTS"; } 9>"$EVENTS.lock"\n')
        result = self.run_script("android-env.sh", "start")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.state / "android-serial").read_text(), "emulator-5600\nemulator-5602\nemulator-5604\n")
        events = self.events.read_text().splitlines()
        launches = [json.loads(event)["args"] for event in events if event.startswith("{")]
        self.assertEqual(len(launches), 4)
        self.assertNotIn("-read-only", launches[0])
        self.assertIn("-no-snapshot-load", launches[0])
        kill = next(i for i, event in enumerate(events) if "emulator-5600 emu kill" in event)
        save = next(i for i, event in enumerate(events) if "snapshot save" in event)
        self.assertLess(save, kill)
        for launch in launches[1:]:
            self.assertIn("-read-only", launch)
            self.assertIn("-snapshot baukit-clean", launch)
        for serial in ("emulator-5600", "emulator-5602", "emulator-5604"):
            self.assertIn(f"adb -s {serial} install -r", self.events.read_text())
            self.assertTrue(any(serial in event and "lastanr" in event for event in events), serial)
        self.assertEqual(self.run_script("android-env.sh", "start", BAUKIT_QA_ANDROID_EMULATORS="0").returncode, 2)

    def sharded(self, maestro):
        (self.state / "android-owned").touch()
        (self.state / "android-serial").write_text("emulator-5556\nemulator-5558\n")
        self.attached.touch()
        flows = self.mobile / ".maestro"
        flows.mkdir()
        for name in ("config.yaml", "a-first.yaml", "b-second.yaml", "c-third.yaml"):
            (flows / name).write_text("appId: test\n")
        self.executable(self.tools / "maestro", maestro)
        return self.run_script("run-maestro.sh", "android")

    def test_maestro_shards_flows_across_every_emulator(self):
        result = self.sharded('echo "maestro $*" >> "$EVENTS"\n')
        self.assertEqual(result.returncode, 0, result.stderr)
        runs = [event for event in self.events.read_text().splitlines() if event.startswith("maestro")]
        self.assertEqual(len(runs), 1)
        self.assertRegex(runs[0], "--device emulator-5556,emulator-5558 --shard-split 2 ")
        self.assertTrue(runs[0].endswith(".maestro/a-first.yaml .maestro/b-second.yaml .maestro/c-third.yaml"))
        events = self.events.read_text()
        for serial in ("emulator-5556", "emulator-5558"):
            self.assertIn(f"adb -s {serial} shell dumpsys activity lastanr", events)

    def test_sharded_run_stops_when_an_emulator_dies(self):
        result = self.sharded('rm -f "$ATTACHED"\nsleep 3\necho finished >> "$EVENTS"\n')
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("died during the sharded run", result.stderr)
        self.assertNotIn("finished", self.events.read_text())

    def test_maestro_stops_when_the_emulator_dies_between_flows(self):
        self.owned()
        flows = self.mobile / ".maestro"
        flows.mkdir()
        for name in ("a-first.yaml", "b-second.yaml", "c-third.yaml"):
            (flows / name).write_text("appId: test\n")
        self.executable(self.tools / "maestro", 'echo "maestro ${*: -1}" >> "$EVENTS"\nrm -f "$ATTACHED"\n')
        result = self.run_script("run-maestro.sh", "android")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("emulator emulator-5556 died before b-second.yaml", result.stderr)
        runs = [event for event in self.events.read_text().splitlines() if event.startswith("maestro")]
        self.assertEqual(runs, ["maestro .maestro/a-first.yaml"])

    def test_maestro_runs_every_flow_once_while_attached(self):
        self.owned()
        flows = self.mobile / ".maestro"
        flows.mkdir()
        for name in ("config.yaml", "a-first.yaml", "b-second.yml"):
            (flows / name).write_text("appId: test\n")
        self.executable(self.tools / "maestro", 'echo "maestro ${*: -1}" >> "$EVENTS"\n')
        result = self.run_script("run-maestro.sh", "android")
        self.assertEqual(result.returncode, 0, result.stderr)
        runs = [event for event in self.events.read_text().splitlines() if event.startswith("maestro")]
        self.assertEqual(runs, ["maestro .maestro/a-first.yaml", "maestro .maestro/b-second.yml"])

    def flows(self, *names):
        self.owned()
        flows = self.mobile / ".maestro"
        flows.mkdir()
        for name in names:
            (flows / name).write_text("appId: test\n")

    def run_watched(self, maestro):
        self.executable(self.tools / "maestro", maestro)
        return subprocess.run(
            ["bash", str(self.scripts / "run-maestro.sh"), "android"],
            env={**self.environment, "BAUKIT_QA_MAESTRO_STALL_SECONDS": "1", "BAUKIT_QA_POLL_SECONDS": "0.1"},
            capture_output=True, text=True, timeout=20, check=False,
        )

    def test_a_flow_without_progress_is_stopped_and_keeps_its_debug_output(self):
        self.flows("a-first.yaml", "b-second.yaml")
        child = self.root / "child"
        result = self.run_watched(f'''
echo "maestro ${{*: -1}}" >> "$EVENTS"
debug="${{*: -2:1}}"
[[ "${{*: -1}}" == .maestro/a-first.yaml ]] || exit 0
echo "Launch app" && touch "$debug/screenshot-1.png"
sleep 60 &
echo $! > {child}
wait
''')
        self.assertEqual(result.returncode, 124, result.stdout + result.stderr)
        self.assertIn("qa: a-first.yaml made no progress for 1 s; stopped it and kept", result.stderr)
        self.assertIn("Launch app", result.stdout)
        debug = self.mobile / "maestro-debug"
        self.assertTrue((debug / "screenshot-1.png").exists())
        self.assertIn("Launch app", (debug / "maestro-output.log").read_text())
        runs = [event for event in self.events.read_text().splitlines() if event.startswith("maestro")]
        self.assertEqual(runs, ["maestro .maestro/a-first.yaml"])
        alive = subprocess.run(["kill", "-0", child.read_text().strip()], capture_output=True, check=False)
        self.assertNotEqual(alive.returncode, 0)

    def test_a_slow_flow_that_keeps_making_progress_passes(self):
        self.flows("a-first.yaml")
        for progress in ('echo "step $step"', 'touch "${*: -2:1}/commands-$step.json"'):
            with self.subTest(progress=progress):
                result = self.run_watched(f'''
for step in 1 2 3 4 5 6 7 8 9 10; do
  {progress}
  sleep 0.4
done
echo finished >> "$EVENTS"
''')
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("finished", self.events.read_text())
                self.events.unlink()

    def test_a_sharded_run_without_progress_is_stopped(self):
        (self.state / "android-owned").touch()
        (self.state / "android-serial").write_text("emulator-5556\nemulator-5558\n")
        self.attached.touch()
        flows = self.mobile / ".maestro"
        flows.mkdir()
        for name in ("a-first.yaml", "b-second.yaml"):
            (flows / name).write_text("appId: test\n")
        result = self.run_watched('sleep 60\n')
        self.assertEqual(result.returncode, 124, result.stdout + result.stderr)
        self.assertIn("qa: the sharded run made no progress for 1 s", result.stderr)

    def test_stall_deadline_must_be_a_positive_integer(self):
        self.flows("a-first.yaml")
        self.executable(self.tools / "maestro", 'echo maestro >> "$EVENTS"\n')
        for value in ("0", "-1", "1.5", "soon"):
            with self.subTest(value=value):
                result = self.run_script("run-maestro.sh", "android", BAUKIT_QA_MAESTRO_STALL_SECONDS=value)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("BAUKIT_QA_MAESTRO_STALL_SECONDS must be a positive integer", result.stderr)
        self.assertFalse(self.events.exists() and "maestro" in self.events.read_text())

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
