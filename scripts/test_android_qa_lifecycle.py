import os
import subprocess
import sys
import time
import unittest

import test_android_qa
from process_runner import run_process, start_process

EMULATOR_DESCRIPTORS = """
echo '{}' >> "$EVENTS"
for entry in /dev/fd/*; do
  fd="${entry##*/}"
  if ((fd > 2 && fd != 255)); then echo "$fd $(readlink "$entry")" >> "$DESCRIPTORS"; fi
done
touch "$DESCRIPTORS"
"""
SLEEPING_EMULATOR = """
echo '{}' >> "$EVENTS"
echo "$$" > "$EMULATOR_PID"
exec sleep 60
"""

EXIT_AFTER_FIRST_PROBE = """
case "$*" in
  *'getprop sys.boot_completed')
    if [[ -f "$PROBED" ]]; then echo 1; exit 0; fi
    [[ -s "$EMULATOR_PID" ]] || exit 0
    touch "$PROBED"
    pid="$(<"$EMULATOR_PID")"
    kill -9 "$pid"
    for _ in $(seq 50); do
      state="$(ps -o stat= -p "$pid" || true)"
      [[ -n "$state" && "$state" != Z* ]] || exit 0
      sleep 0.1
    done
    exit 0 ;;
esac
exec "$0-base" "$@"
"""


class AndroidQaLifecycleTests(unittest.TestCase):
    def setUp(self):
        test_android_qa.AndroidQaTests.setUp(self)
        self.environment["BAUKIT_QA_ANDROID_EMULATORS"] = "1"

    executable = test_android_qa.AndroidQaTests.executable
    owned = test_android_qa.AndroidQaTests.owned
    run_script = test_android_qa.AndroidQaTests.run_script
    prepare_start = test_android_qa.AndroidQaTests.prepare_start

    def events_list(self):
        return self.events.read_text().splitlines() if self.events.exists() else []

    def flows(self, *names):
        self.owned()
        flows = self.mobile / ".maestro"
        flows.mkdir()
        for name in names:
            (flows / name).write_text("appId: test\n")

    def test_the_emulator_does_not_inherit_the_callers_descriptors(self):
        self.prepare_start()
        descriptors = self.root / "descriptors"
        self.environment["DESCRIPTORS"] = str(descriptors)
        self.executable(self.sdk / "emulator/emulator", EMULATOR_DESCRIPTORS)
        lock = os.open(self.root / "gate.lock", os.O_CREAT | os.O_WRONLY)
        self.addCleanup(os.close, lock)
        result = run_process(
            ["bash", str(self.scripts / "android-env.sh"), "boot"],
            env=self.environment,
            pass_fds=(lock,),
            capture_output=True,
            text=True,
            timeout=15,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(descriptors.exists())
        self.assertNotIn("gate.lock", descriptors.read_text())

    def test_a_failed_start_stops_the_emulator_it_started(self):
        self.prepare_start()
        pid_file = self.root / "emulator.pid"
        self.environment["EMULATOR_PID"] = str(pid_file)
        self.executable(self.sdk / "emulator/emulator", SLEEPING_EMULATOR)
        result = self.run_script("android-env.sh", "start", FAILURE="health")
        self.assertEqual(result.returncode, 1, result.stderr)
        pid = int(pid_file.read_text())
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and self.process_running(pid):
            time.sleep(0.1)
        self.assertFalse(self.process_running(pid))
        self.assertEqual(list(self.state.glob("android-emulator-*.pid")), [])

    def test_a_successful_boot_leaves_the_emulator_running(self):
        self.prepare_start()
        pid_file = self.root / "emulator.pid"
        self.environment["EMULATOR_PID"] = str(pid_file)
        self.executable(self.sdk / "emulator/emulator", SLEEPING_EMULATOR)
        result = self.run_script("android-env.sh", "boot")
        self.assertEqual(result.returncode, 0, result.stderr)
        pid = int(pid_file.read_text())
        self.addCleanup(os.kill, pid, 9)
        self.assertTrue(self.process_running(pid))

    @staticmethod
    def process_running(pid):
        state = run_process(
            ["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True
        ).stdout.strip()
        return bool(state) and not state.startswith("Z")

    def test_boot_wait_fails_early_when_the_emulator_process_is_gone(self):
        self.prepare_start()
        self.executable(self.sdk / "emulator/emulator", "exit 3\n")
        started = time.monotonic()
        result = self.run_script("android-env.sh", "boot", BAUKIT_QA_BOOT_TIMEOUT_SECONDS="60")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("exited before emulator-5556 booted", result.stderr)
        self.assertLess(time.monotonic() - started, 30)

    def test_boot_wait_counts_a_boot_the_emulator_finished_before_it_exited(self):
        self.prepare_start()
        pid_file = self.root / "emulator.pid"
        self.environment["EMULATOR_PID"] = str(pid_file)
        self.environment["PROBED"] = str(self.root / "probed")
        self.executable(self.sdk / "emulator/emulator", SLEEPING_EMULATOR)
        adb = self.sdk / "platform-tools/adb"
        adb.rename(adb.with_name("adb-base"))
        self.executable(adb, EXIT_AFTER_FIRST_PROBE)
        result = self.run_script("android-env.sh", "boot")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_reverse_is_reapplied_before_every_flow(self):
        self.flows("a.yaml", "b.yaml")
        self.executable(self.tools / "maestro", 'echo "maestro ${*: -1}" >> "$EVENTS"\n')
        result = self.run_script("run-maestro.sh", "android")
        self.assertEqual(result.returncode, 0, result.stderr)
        steps = [
            event
            for event in self.events_list()
            if event.startswith("maestro") or "reverse tcp:18080" in event
        ]
        reverse = "adb -s emulator-5556 reverse tcp:18080 tcp:18080"
        self.assertEqual(
            steps,
            [reverse, "maestro .maestro/a.yaml", reverse, "maestro .maestro/b.yaml"],
        )

    def test_a_detached_device_stops_the_run_before_the_next_flow(self):
        self.flows("a.yaml", "b.yaml")
        self.executable(
            self.tools / "maestro", 'echo "maestro ${*: -1}" >> "$EVENTS"\nrm -f "$ATTACHED"\n'
        )
        result = self.run_script("run-maestro.sh", "android")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("emulator emulator-5556 died before b.yaml", result.stderr)

    def test_flows_hook_runs_each_devices_list_sequentially(self):
        self.flows("a.yaml", "b.yaml", "c.yaml")
        (self.state / "android-serial").write_text("emulator-5556\nemulator-5558\n")
        (self.scripts / "local.sh").write_text(
            "qa_maestro_flows() { if (($1 == 0)); then echo .maestro/a.yaml; echo .maestro/b.yaml;"
            " else echo .maestro/c.yaml; fi; }\n"
            'qa_maestro_env() { export QA_INDEX="$1"; }\n'
        )
        self.executable(self.tools / "maestro", 'echo "maestro $QA_INDEX $*" >> "$EVENTS"\n')
        self.environment["BAUKIT_QA_ANDROID_EMULATORS"] = "2"
        result = self.run_script("run-maestro.sh", "android")
        self.assertEqual(result.returncode, 0, result.stderr)
        runs = sorted(event for event in self.events_list() if event.startswith("maestro"))
        self.assertEqual(len(runs), 3)
        self.assertTrue(
            runs[0].startswith("maestro 0 test --platform android --device emulator-5556")
        )
        self.assertTrue(runs[0].endswith(".maestro/a.yaml"))
        self.assertTrue(runs[1].endswith(".maestro/b.yaml"))
        self.assertIn("--device emulator-5558", runs[2])
        self.assertTrue((self.mobile / "maestro-debug/shard-1/maestro-output.log").exists())

    def test_flows_hook_fails_a_device_when_a_flow_fails(self):
        self.flows("a.yaml", "b.yaml")
        (self.scripts / "local.sh").write_text(
            "qa_maestro_flows() { echo .maestro/a.yaml; echo .maestro/b.yaml; }\n"
        )
        self.executable(
            self.tools / "maestro",
            'echo "maestro ${*: -1}" >> "$EVENTS"\n[[ "${*: -1}" != .maestro/a.yaml ]] || exit 9\n',
        )
        result = self.run_script("run-maestro.sh", "android")
        self.assertEqual(result.returncode, 9, result.stderr)
        self.assertEqual(
            [event for event in self.events_list() if event.startswith("maestro")],
            ["maestro .maestro/a.yaml"],
        )

    def test_flows_hook_and_declared_shards_are_exclusive(self):
        self.flows("a.yaml")
        (self.scripts / "local.sh").write_text(
            "qa_maestro_flows() { echo .maestro/a.yaml; }\nqa_maestro_shards=(.maestro/a.yaml)\n"
        )
        self.executable(self.tools / "maestro", "exit 0\n")
        result = self.run_script("run-maestro.sh", "android")
        self.assertEqual(result.returncode, 1)
        self.assertIn("not both", result.stderr)

    def test_ios_bundle_id_comes_from_the_environment_or_the_hook(self):
        (self.state / "ios-udid").write_text("udid-1\n")
        (self.mobile / ".maestro").mkdir()
        (self.mobile / ".maestro/a.yaml").write_text("appId: test\n")
        self.executable(self.tools / "maestro", 'echo "maestro $*" >> "$EVENTS"\n')
        for bundle, hook in (
            ("dev.baukit.test", ""),
            ("com.example.env", ""),
            ("com.example.hook", "x"),
        ):
            with self.subTest(bundle=bundle):
                self.events.unlink(missing_ok=True)
                extra = {}
                if bundle == "com.example.env":
                    extra["BAUKIT_QA_IOS_BUNDLE_ID"] = bundle
                if hook:
                    (self.scripts / "local.sh").write_text(f"qa_ios_bundle_id={bundle}\n")
                result = self.run_script("run-maestro.sh", "ios", **extra)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(f"--env APP_ID={bundle}", self.events.read_text())

    def test_hooks_stop_extra_supervised_processes_through_the_pid_file_helper(self):
        binary = self.root / "backend/target/debug/worker"
        self.executable(
            binary,
            'exec -a "$0" python3 -c "print(\'ready\', flush=True); import time; time.sleep(60)"\n',
        )
        worker = start_process([str(binary)], stdout=subprocess.PIPE, text=True)
        self.addCleanup(worker.stdout.close)
        self.assertEqual(worker.stdout.readline().strip(), "ready")
        self.addCleanup(worker.wait)
        self.addCleanup(lambda: worker.kill() if worker.poll() is None else None)
        pid_file = self.state / "worker.pid"
        pid_file.write_text(f"{worker.pid}\n")
        (self.scripts / "local.sh").write_text(
            'qa_stop_services() { stop_pid_file "$state_dir/worker.pid" worker'
            ' "$qa_backend_bin_dir/worker"; }\n'
        )
        result = self.run_script("services.sh", "stop")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIsNotNone(worker.wait(timeout=5))
        self.assertFalse(pid_file.exists())

    def test_the_helper_keeps_a_pid_file_that_does_not_hold_a_pid(self):
        pid_file = self.state / "worker.pid"
        pid_file.write_text("invalid\n")
        (self.scripts / "local.sh").write_text(
            'qa_stop_services() { stop_pid_file "$state_dir/worker.pid" worker'
            ' "$qa_backend_bin_dir/worker"; }\n'
        )
        result = self.run_script("services.sh", "stop")
        self.assertEqual(result.returncode, 1)
        self.assertIn("invalid worker PID", result.stderr)
        self.assertTrue(pid_file.exists())

    def test_apk_variant_selects_the_default_apk_path(self):
        self.prepare_start()
        debug = self.mobile / "android/app/build/outputs/apk/debug"
        debug.mkdir(parents=True)
        (debug / "app-debug.apk").touch()
        result = self.run_script("android-env.sh", "start", BAUKIT_QA_APK_VARIANT="debug")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("install -r " + str(debug / "app-debug.apk"), self.events.read_text())
        invalid = self.run_script("android-env.sh", "start", BAUKIT_QA_APK_VARIANT="staging")
        self.assertEqual(invalid.returncode, 2)


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]])
