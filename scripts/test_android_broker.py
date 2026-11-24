import unittest

import test_android_qa as qa
from process_runner import run_process

FAKE_PRUEFSTAND = r"""
printf 'pruefstand %s url=%s\n' "$*" "$PRUEFSTAND_URL" >> "$EVENTS"
[[ "$1" == lease ]] || exit "${RELEASE_STATUS:-0}"
[[ -z "${LEASE_STATUS:-}" ]] || exit "$LEASE_STATUS"
for ((n = 1; n <= $5; n++)); do
  echo "ANDROID_SERIAL=10.0.0.$n:5555 LEASE_ID=lease-$n EXPIRES_AT=2026-11-24T12:00:00Z"
done
"""
BROKER = "https://pruefstand.lab.example"


class AndroidBrokerTests(unittest.TestCase):
    setUp = qa.AndroidQaTests.setUp
    executable = qa.AndroidQaTests.executable
    run_script = qa.AndroidQaTests.run_script
    prepare_start = qa.AndroidQaTests.prepare_start

    def start(self, **environment):
        self.prepare_start()
        self.executable(self.tools / "pruefstand", FAKE_PRUEFSTAND)
        self.environment.pop("ANDROID_AVD_HOME")
        self.events.write_text("{}\n")
        self.attached.touch()
        return self.run_script(
            "android-env.sh",
            "start",
            BAUKIT_QA_DEVICE_BROKER=BROKER,
            BAUKIT_QA_DEVICE_BROKER_ARGS="--pool lab --label ci",
            BAUKIT_QA_ANDROID_EMULATORS="2",
            **environment,
        )

    def broker_calls(self, command):
        return [
            event
            for event in self.events.read_text().splitlines()
            if event.startswith(f"pruefstand {command} ")
        ]

    def stop(self):
        return self.run_script("android-env.sh", "stop", BAUKIT_QA_DEVICE_BROKER=BROKER)

    def test_start_leases_devices_and_stop_releases_them(self):
        result = self.start()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            self.broker_calls("lease"),
            [
                "pruefstand lease --api 36 --count 2 --ttl 30m --wait 10m --pool lab --label ci "
                f"url={BROKER}"
            ],
        )
        self.assertEqual(
            (self.state / "android-serial").read_text(), "10.0.0.1:5555\n10.0.0.2:5555\n"
        )
        events = self.events.read_text()
        self.assertNotIn("emulator", events.replace("android-emulator", ""))
        for serial in ("10.0.0.1:5555", "10.0.0.2:5555"):
            self.assertIn(f"adb connect {serial}", events)
            self.assertIn(f"adb -s {serial} install -r", events)
        self.assertEqual(self.broker_calls("release"), [])
        stopped = self.stop()
        self.assertEqual(stopped.returncode, 0, stopped.stderr)
        self.assertEqual(
            self.broker_calls("release"), [f"pruefstand release lease-1 lease-2 url={BROKER}"]
        )
        self.assertIn("adb disconnect 10.0.0.1:5555", self.events.read_text())
        self.assertFalse((self.state / "android-leases").exists())

    def test_a_failed_start_releases_its_leases_on_exit(self):
        result = self.start(FAILURE="install")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            self.broker_calls("release"), [f"pruefstand release lease-1 lease-2 url={BROKER}"]
        )
        self.assertFalse((self.state / "android-leases").exists())
        self.assertFalse((self.state / "android-serial").exists())

    def test_no_capacity_exits_3_without_releasing(self):
        result = self.start(LEASE_STATUS="3")
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertIn("the device broker had no 2 free emulators within 10m", result.stderr)
        self.assertEqual(self.broker_calls("release"), [])
        self.assertNotIn("install -r", self.events.read_text())

    def test_a_failed_release_keeps_the_stop_failing(self):
        self.assertEqual(self.start().returncode, 0)
        stopped = self.run_script(
            "android-env.sh", "stop", BAUKIT_QA_DEVICE_BROKER=BROKER, RELEASE_STATUS="1"
        )
        self.assertNotEqual(stopped.returncode, 0)
        self.assertIn("the broker reclaims them after their TTL", stopped.stderr)

    def test_reverse_accepts_a_leased_serial(self):
        result = run_process(
            ["bash", str(self.scripts / "android-env.sh"), "reverse", "10.0.0.1:5555"],
            env={**self.environment, "BAUKIT_QA_DEVICE_BROKER": BROKER},
            capture_output=True,
            text=True,
            timeout=qa.HANG_WATCHDOG_SECONDS,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("adb -s 10.0.0.1:5555 reverse tcp:18080 tcp:18080", self.events.read_text())


if __name__ == "__main__":
    unittest.main()
