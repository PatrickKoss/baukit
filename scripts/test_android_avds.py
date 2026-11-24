import json
import unittest

import test_android_qa as qa
from test_android_qa import free_console_ports


class AndroidAvdTests(unittest.TestCase):
    setUp = qa.AndroidQaTests.setUp
    executable = qa.AndroidQaTests.executable
    run_script = qa.AndroidQaTests.run_script
    prepare_start = qa.AndroidQaTests.prepare_start

    def test_each_emulator_boots_its_own_writable_avd(self):
        self.prepare_start()
        port = free_console_ports(3)
        serials = [f"emulator-{port + offset}" for offset in (0, 2, 4)]
        (self.root / "baukit.toml").write_text(
            f"[quality]\nandroid_emulator_port = {port}\nandroid_emulators = 3\n"
        )
        self.executable(
            self.sdk / "emulator/emulator",
            '{ flock 9; printf \'{"args": "%s"}\\n\' "$*" >> "$EVENTS"; } 9>"$EVENTS.lock"\n',
        )
        result = self.run_script("android-env.sh", "start")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            (self.state / "android-serial").read_text(),
            "".join(f"{serial}\n" for serial in serials),
        )
        events = self.events.read_text().splitlines()
        launches = [json.loads(event)["args"] for event in events if event.startswith("{")]
        self.assertEqual(
            sorted(launch.split("-avd ")[1].split()[0] for launch in launches),
            ["test-qa", "test-qa-1", "test-qa-2"],
        )
        for launch in launches:
            self.assertNotIn("-read-only", launch)
        for serial in serials:
            self.assertIn(f"adb -s {serial} install -r", self.events.read_text())
            self.assertTrue(any(serial in event and "lastanr" in event for event in events), serial)
        self.assertEqual(
            self.run_script("android-env.sh", "start", BAUKIT_QA_ANDROID_EMULATORS="0").returncode,
            2,
        )

    def test_extra_avd_is_cloned_once_and_recreated_on_config_change(self):
        self.prepare_start()
        port = free_console_ports(2)
        (self.root / "baukit.toml").write_text(
            f"[quality]\nandroid_emulator_port = {port}\nandroid_emulators = 2\n"
        )
        clone = self.root / "avd/test-qa-1.avd"
        self.assertEqual(self.run_script("android-env.sh", "start").returncode, 0)
        self.run_script("android-env.sh", "stop")
        self.assertEqual(
            (clone / "config.ini").read_text(),
            (self.root / "avd/test-qa.avd/config.ini").read_text(),
        )
        self.assertIn("path.rel=avd/test-qa-1.avd", (self.root / "avd/test-qa-1.ini").read_text())
        marker = clone / "userdata-qemu.img"
        marker.write_text("kept")
        self.assertEqual(self.run_script("android-env.sh", "start").returncode, 0)
        self.run_script("android-env.sh", "stop")
        self.assertTrue(marker.exists())
        (self.root / "avd/test-qa.avd/config.ini").write_text("hw.ramSize=8192\n")
        self.assertEqual(self.run_script("android-env.sh", "start").returncode, 0)
        self.run_script("android-env.sh", "stop")
        self.assertFalse(marker.exists())
        self.assertEqual((clone / "config.ini").read_text(), "hw.ramSize=8192\n")

    def test_each_avd_prunes_to_its_own_single_snapshot(self):
        self.prepare_start()
        port = free_console_ports(2)
        (self.root / "baukit.toml").write_text(
            f"[quality]\nandroid_emulator_port = {port}\nandroid_emulators = 2\n"
        )
        self.executable(
            self.sdk / "emulator/emulator",
            'while (($#)); do case "$1" in -avd) avd="$2" ;; -port) port="$2" ;; esac\n'
            "shift; done\n"
            'echo "$avd" > "$EVENTS.avd-$port"; echo "{}" >> "$EVENTS"\n',
        )
        first = self.run_script("android-env.sh", "start")
        self.assertEqual(first.returncode, 0, first.stderr)
        self.run_script("android-env.sh", "stop")
        avds = [self.root / "avd/test-qa.avd", self.root / "avd/test-qa-1.avd"]
        for avd in avds:
            for ready in avd.glob("*.ready"):
                ready.unlink()
            (avd / "snapshots/baukit-clean-000000000000").mkdir(parents=True)
            (avd / "baukit-clean-000000000000.ready").touch()
        result = self.run_script("android-env.sh", "start")
        self.assertEqual(result.returncode, 0, result.stderr)
        names = []
        for avd in avds:
            snapshots = [path.name for path in (avd / "snapshots").iterdir()]
            self.assertEqual(len(snapshots), 1, avd)
            self.assertNotEqual(snapshots, ["baukit-clean-000000000000"])
            self.assertEqual([path.name for path in avd.glob("*.ready")], [f"{snapshots[0]}.ready"])
            names.append(snapshots[0])
        self.assertEqual(names[0], names[1])
        self.assertEqual(self.run_script("android-env.sh", "stop").returncode, 0)


if __name__ == "__main__":
    unittest.main()
