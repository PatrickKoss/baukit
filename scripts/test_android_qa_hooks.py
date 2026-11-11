import unittest

import test_android_qa
from process_runner import run_process

APP_ID = "dev.baukit.test"
OTHER_APP_ID = "dev.example.product"


class AndroidQaHookTests(unittest.TestCase):
    def setUp(self):
        test_android_qa.AndroidQaTests.setUp(self)
        self.environment["BAUKIT_QA_ANDROID_EMULATORS"] = "1"

    executable = test_android_qa.AndroidQaTests.executable
    owned = test_android_qa.AndroidQaTests.owned
    run_script = test_android_qa.AndroidQaTests.run_script
    prepare_start = test_android_qa.AndroidQaTests.prepare_start

    def hooks(self, body):
        (self.scripts / "local.sh").write_text(body)

    def events_list(self):
        return self.events.read_text().splitlines() if self.events.exists() else []

    def flows(self, *names):
        self.owned()
        flows = self.mobile / ".maestro"
        flows.mkdir()
        for name in names:
            (flows / name).write_text("appId: test\n")

    def starts(self):
        return [event for event in self.events_list() if "shell am start" in event]

    def test_app_id_defaults_to_the_template_id(self):
        self.prepare_start()
        self.assertEqual(self.run_script("android-env.sh", "start").returncode, 0)
        self.assertEqual(
            self.starts(), [f"adb -s emulator-5556 shell am start -n {APP_ID}/.MainActivity -W"]
        )

    def test_app_id_comes_from_the_environment_or_the_hook_in_both_scripts(self):
        self.prepare_start()
        self.flows("a.yaml")
        self.executable(self.tools / "maestro", 'echo "maestro $*" >> "$EVENTS"\n')
        for source in ("environment", "hook"):
            with self.subTest(source=source):
                self.events.unlink(missing_ok=True)
                environment = {}
                if source == "environment":
                    environment["BAUKIT_QA_APP_ID"] = OTHER_APP_ID
                else:
                    self.hooks(f"qa_app_id={OTHER_APP_ID}\n")
                self.assertEqual(
                    self.run_script("android-env.sh", "start", **environment).returncode, 0
                )
                self.attached.touch()
                maestro = self.run_script("run-maestro.sh", "android", **environment)
                self.assertEqual(maestro.returncode, 0, maestro.stderr)
                self.assertIn(f"-n {OTHER_APP_ID}/.MainActivity", self.starts()[0])
                self.assertIn(f"--env APP_ID={OTHER_APP_ID}", "\n".join(self.events_list()))

    def test_launch_error_reported_by_am_start_fails_the_start(self):
        self.prepare_start()
        result = self.run_script("android-env.sh", "start", FAILURE="launch-soft")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("Error: Activity class does not exist.", result.stderr)
        self.assertIn(f"could not start {APP_ID} on emulator-5556", result.stderr)

    def test_failed_install_prints_the_adb_output(self):
        self.prepare_start()
        result = self.run_script("android-env.sh", "start", FAILURE="install")
        self.assertEqual(result.returncode, 18, result.stderr)
        self.assertIn("INSTALL_FAILED_OLDER_SDK", result.stderr)
        self.assertEqual(self.starts(), [])

    def test_boot_leaves_services_and_the_app_alone(self):
        self.prepare_start()
        result = self.run_script("android-env.sh", "boot")
        self.assertEqual(result.returncode, 0, result.stderr)
        events = self.events_list()
        self.assertNotIn("services start", events)
        self.assertFalse(any("install -r" in event for event in events))
        self.assertTrue((self.state / "android-owned").exists())
        self.assertIn("emulators booted on emulator-5556", result.stdout)

    def test_boot_keeps_a_running_stack_when_it_replaces_owned_emulators(self):
        self.prepare_start()
        self.owned()
        self.assertEqual(self.run_script("android-env.sh", "boot").returncode, 0)
        self.assertNotIn("services stop", self.events_list())

    def test_prepare_builds_installs_and_launches_without_booting(self):
        self.prepare_start()
        self.owned()
        self.hooks('qa_build_apk() { echo "hook build" >> "$EVENTS"; }\n')
        result = self.run_script("android-env.sh", "prepare", BAUKIT_QA_SKIP_BUILD="0")
        self.assertEqual(result.returncode, 0, result.stderr)
        events = self.events_list()
        self.assertEqual(events[0], "hook build")
        self.assertFalse(any("emu kill" in event for event in events))
        self.assertFalse(any("services" in event for event in events))
        self.assertTrue(any("install -r" in event for event in events))
        self.assertEqual(len(self.starts()), 1)

    def test_prepare_fails_without_the_apk(self):
        self.prepare_start()
        self.hooks('qa_apk_path="$root/missing.apk"\n')
        result = self.run_script("android-env.sh", "prepare")
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing.apk", result.stderr)

    def test_hooks_replace_device_preparation_and_choose_the_apk(self):
        self.prepare_start()
        custom = self.root / "debug.apk"
        custom.touch()
        self.hooks(
            f'qa_apk_path="{custom}"\n'
            'qa_prepare_device() { echo "hook device $1 $2" >> "$EVENTS"; }\n'
        )
        result = self.run_script("android-env.sh", "start", BAUKIT_QA_ANDROID_EMULATORS="2")
        self.assertEqual(result.returncode, 0, result.stderr)
        events = self.events_list()
        for serial in ("emulator-5556", "emulator-5558"):
            self.assertIn(f"hook device {serial} {custom}", events)
        self.assertFalse(any("install -r" in event for event in events))

    def test_hook_can_extend_the_default_device_preparation(self):
        self.prepare_start()
        self.hooks('qa_prepare_device() { prepare_device "$@"; echo "after $1" >> "$EVENTS"; }\n')
        self.assertEqual(self.run_script("android-env.sh", "start").returncode, 0)
        events = self.events_list()
        self.assertGreater(events.index("after emulator-5556"), events.index(self.starts()[0]))

    def test_a_failing_device_hook_fails_the_start_with_its_status(self):
        self.prepare_start()
        self.hooks("qa_prepare_device() { return 23; }\n")
        result = self.run_script("android-env.sh", "start")
        self.assertEqual(result.returncode, 23, result.stderr)
        self.assertNotIn("Android app ready", result.stdout)

    def test_ports_set_in_the_hook_file_reach_adb_reverse(self):
        self.prepare_start()
        self.hooks("api_port=8280\n")
        self.assertEqual(self.run_script("android-env.sh", "start").returncode, 0)
        self.assertIn("adb -s emulator-5556 reverse tcp:8280 tcp:8280", self.events_list())

    def test_usage_lists_every_action(self):
        result = self.run_script("android-env.sh", "restart")
        self.assertEqual(result.returncode, 2)
        self.assertIn("[start|boot|prepare|stop]", result.stderr)

    def test_gpu_mode_is_selectable_and_validated(self):
        self.prepare_start()
        self.executable(
            self.sdk / "emulator/emulator",
            '{ flock 9; printf \'{"args": "%s"}\\n\' "$*" >> "$EVENTS"; } 9>"$EVENTS.lock"\n',
        )
        self.assertEqual(
            self.run_script("android-env.sh", "boot", BAUKIT_QA_ANDROID_GPU="auto").returncode, 0
        )
        self.assertTrue(any("-gpu auto" in event for event in self.events_list()))
        invalid = self.run_script("android-env.sh", "boot", BAUKIT_QA_ANDROID_GPU="a b")
        self.assertEqual(invalid.returncode, 2)

    def sharded_maestro(self, hooks, script='echo "maestro $*" >> "$EVENTS"\n', **environment):
        self.flows("a.yaml", "b.yaml")
        (self.state / "android-serial").write_text("emulator-5556\nemulator-5558\n")
        self.hooks(hooks)
        self.executable(self.tools / "maestro", script)
        return self.run_script("run-maestro.sh", "android", **environment)

    def test_declared_shards_run_one_per_device_with_per_device_environment(self):
        result = self.sharded_maestro(
            "qa_maestro_shards=(.maestro/a.yaml .maestro/b.yaml)\n"
            'qa_maestro_env() { export QA_USER="user-$1"; debug_dir="$debug_dir-$1"; }\n',
            'echo "maestro $QA_USER $*" >> "$EVENTS"\n',
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        runs = sorted(event for event in self.events_list() if event.startswith("maestro"))
        self.assertEqual(len(runs), 2)
        self.assertIn("maestro user-0 test --platform android --device emulator-5556", runs[0])
        self.assertTrue(runs[0].endswith(".maestro/a.yaml"))
        self.assertIn("maestro user-1 test --platform android --device emulator-5558", runs[1])
        self.assertTrue(runs[1].endswith(".maestro/b.yaml"))
        self.assertTrue((self.mobile / "maestro-debug/shard-1-1/maestro-output.log").exists())

    def test_declared_shards_report_a_failing_shard(self):
        result = self.sharded_maestro(
            "qa_maestro_shards=(.maestro/a.yaml .maestro/b.yaml)\n",
            '[[ "${*: -1}" != .maestro/b.yaml ]] || exit 9\n',
        )
        self.assertEqual(result.returncode, 9, result.stderr)

    def test_more_declared_shards_than_devices_fail_before_running_maestro(self):
        result = self.sharded_maestro(
            "qa_maestro_shards=(.maestro/a.yaml .maestro/b.yaml .maestro/c.yaml)\n"
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("3 Maestro shards but only 2 devices", result.stderr)

    def test_maestro_env_hook_runs_for_the_unsharded_path(self):
        self.flows("a.yaml")
        self.hooks('qa_maestro_env() { echo "env $1" >> "$EVENTS"; }\n')
        self.executable(self.tools / "maestro", "exit 0\n")
        self.assertEqual(self.run_script("run-maestro.sh", "android").returncode, 0)
        self.assertEqual(self.events_list().count("env 0"), 1)

    def test_a_run_past_the_deadline_is_stopped_with_exit_124(self):
        self.flows("a.yaml")
        self.executable(self.tools / "maestro", "while true; do echo tick; sleep 0.1; done\n")
        result = run_process(
            ["bash", str(self.scripts / "run-maestro.sh"), "android"],
            env={
                **self.environment,
                "BAUKIT_QA_MAESTRO_DEADLINE_SECONDS": "1",
                "BAUKIT_QA_POLL_SECONDS": "0.1",
            },
            capture_output=True,
            text=True,
            timeout=20,
            check=False,
        )
        self.assertEqual(result.returncode, 124, result.stderr)
        self.assertIn("exceeded 1 s", result.stderr)

    def test_deadline_must_be_a_positive_integer(self):
        self.flows("a.yaml")
        self.executable(self.tools / "maestro", "exit 0\n")
        result = self.run_script(
            "run-maestro.sh", "android", BAUKIT_QA_MAESTRO_DEADLINE_SECONDS="0"
        )
        self.assertEqual(result.returncode, 2)

    def test_state_dir_can_be_moved(self):
        self.prepare_start()
        moved = self.root / "moved"
        result = self.run_script("android-env.sh", "boot", BAUKIT_QA_STATE_DIR=str(moved))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((moved / "android-owned").exists())
        self.assertFalse((self.state / "android-owned").exists())


if __name__ == "__main__":
    unittest.main()
