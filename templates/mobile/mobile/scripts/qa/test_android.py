import os
import time
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parent


class AndroidScriptsTest(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.scripts = self.root / "mobile/scripts/qa"
        shutil.copytree(SCRIPTS, self.scripts)
        self.sdk = self.root / "sdk"
        self.avds = self.root / "avds"
        self.events = self.root / "events"
        self.environment = {
            **os.environ,
            "ANDROID_HOME": str(self.sdk),
            "ANDROID_SDK_ROOT": str(self.sdk),
            "ANDROID_AVD_HOME": str(self.avds),
            "BAUKIT_QA_ANDROID_AVD": "test-qa",
            "BAUKIT_QA_ANDROID_ARCHITECTURE": "x86_64",
            "BAUKIT_QA_SKIP_BUILD": "1",
            "EVENTS": str(self.events),
        }

    def executable(self, path: Path, content: str) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("#!/usr/bin/env bash\nset -eu\n" + content)
        path.chmod(0o755)

    def run_script(self, name: str) -> None:
        result = subprocess.run(
            ["bash", str(self.scripts / name)],
            env=self.environment, capture_output=True, text=True, check=False, timeout=15,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_avd_keys_with_whitespace_are_replaced_once(self) -> None:
        tools = self.sdk / "cmdline-tools/13114758/bin"
        self.executable(tools / "sdkmanager", "exit 0\n")
        self.executable(tools / "avdmanager", 'echo "Name: test-qa"\n')
        config = self.avds / "test-qa.avd/config.ini"
        config.parent.mkdir(parents=True)
        config.write_text(
            "image.sysdir.1=system-images/android-36/google_apis/x86_64/\n"
            "abi.type=x86_64\nhw.ramSize = 1024\n hw.ramSize=2048\n"
            "hw.keyboard = no\n\thw.keyboard\t= no\nhw.gpu.mode=auto\nhw.cpu.ncore = 1\n\thw.cpu.ncore\t= 2\n"
        )
        for _ in range(2):
            self.run_script("android-sdk.sh")
            self.assertEqual(config.read_text().splitlines(), [
                "image.sysdir.1=system-images/android-36/google_apis/x86_64/",
                "abi.type=x86_64", "hw.gpu.mode=auto",
                "hw.ramSize=4096", "hw.keyboard=yes", "hw.cpu.ncore=4",
            ])

    def test_sdk_uses_pinned_tools_even_when_latest_exists(self) -> None:
        tools = self.sdk / "cmdline-tools/13114758/bin"
        self.executable(tools / "sdkmanager", 'printf "pinned-sdk %s\\n" "$*" >> "$EVENTS"\n')
        self.executable(tools / "avdmanager", 'echo "Name: test-qa"\n')
        self.executable(self.sdk / "cmdline-tools/latest/bin/sdkmanager", "exit 99\n")
        self.run_script("android-sdk.sh")
        self.assertIn("pinned-sdk --sdk_root=", self.events.read_text())

    def test_release_build_sets_loopback_without_host_interface_discovery(self) -> None:
        bin_dir = self.root / "bin"
        self.executable(bin_dir / "corepack", 'printf "corepack %s\\n" "$*" >> "$EVENTS"\n')
        self.environment["PATH"] = str(bin_dir) + os.pathsep + self.environment["PATH"]
        self.executable(self.root / "mobile/node_modules/.bin/expo", 'printf "expo %s\\n" "$*" >> "$EVENTS"\n')
        self.executable(self.root / "mobile/android/gradlew", r'''
printf 'gradle %s\n' "$*" >> "$EVENTS"
apk="$(dirname "$0")/app/build/outputs/apk/release/app-release.apk"
mkdir -p "$(dirname "$apk")"
touch "$apk"
''')
        self.run_script("build-android.sh")
        self.assertEqual(self.events.read_text().splitlines(), [
            f"corepack pnpm@12.9.1 --dir {self.root}/mobile install --frozen-lockfile",
            f"corepack pnpm@12.9.1 --dir {self.root}/mobile run tokens",
            "expo prebuild --clean --platform android --no-install",
            f"gradle -p {self.root}/mobile/android --no-daemon "
            "-PreactNativeDevServerIp=127.0.0.1 -PreactNativeArchitectures=x86_64 assembleRelease",
        ])

    def test_avd_is_recreated_when_api_tag_or_architecture_changes(self) -> None:
        tools = self.sdk / "cmdline-tools/13114758/bin"
        self.executable(tools / "sdkmanager", "exit 0\n")
        self.executable(tools / "avdmanager", r'''
printf '%s\n' "$*" >> "$EVENTS"
config="$ANDROID_AVD_HOME/test-qa.avd/config.ini"
case "$1" in
  delete) rm -f "$config" ;;
  list) if [[ -f "$config" ]]; then echo "Name: test-qa"; fi ;;
  create)
    while [[ "$1" != --package ]]; do shift; done
    image="${2//;/\/}"
    mkdir -p "$(dirname "$config")"
    printf 'image.sysdir.1=%s/\n' "$image" > "$config"
    ;;
esac
''')
        config = self.avds / "test-qa.avd/config.ini"
        config.parent.mkdir(parents=True)
        for old_image in (
            "system-images/android-35/google_apis/x86_64/",
            "system-images/android-36/google_apis_playstore/x86_64/",
            "system-images/android-36/google_apis/arm64-v8a/",
        ):
            with self.subTest(old_image=old_image):
                config.write_text(f"image.sysdir.1 = {old_image}\n")
                self.events.unlink(missing_ok=True)
                self.run_script("android-sdk.sh")
                self.assertIn("delete avd --name test-qa", self.events.read_text())
                self.assertIn("create avd", self.events.read_text())
                self.assertIn("image.sysdir.1=system-images/android-36/google_apis/x86_64/", config.read_text())
                self.events.unlink()
                self.run_script("android-sdk.sh")
                self.assertNotIn("delete avd", self.events.read_text())
                self.assertNotIn("create avd", self.events.read_text())
        self.environment["BAUKIT_QA_ANDROID_IMAGE_TAG"] = "google_apis_playstore"
        self.run_script("android-sdk.sh")
        self.assertIn("image.sysdir.1=system-images/android-36/google_apis_playstore/x86_64/", config.read_text())

    def test_hanging_adb_commands_and_boot_probes_are_bounded(self) -> None:
        adb = self.sdk / "platform-tools/adb"
        self.executable(adb, "exec sleep 60\n")
        self.environment["BAUKIT_QA_ADB_TIMEOUT_SECONDS"] = "0.2"
        self.environment["BAUKIT_QA_BOOT_TIMEOUT_SECONDS"] = "0.5"
        for arguments in ([str(adb), "devices"], ["--wait-for-boot", str(adb), "emulator-5556"]):
            with self.subTest(arguments=arguments):
                started = time.monotonic()
                result = subprocess.run(
                    ["python3", str(self.scripts / "android-adb.py"), *arguments],
                    env=self.environment, capture_output=True, text=True, timeout=5, check=False,
                )
                self.assertEqual(result.returncode, 124, result.stdout + result.stderr)
                self.assertIn("timed out", result.stderr)
                if arguments[-1] == "devices":
                    self.assertIn(" ".join(arguments), result.stderr)
                self.assertLess(time.monotonic() - started, 5)

    def test_probe_timeouts_reject_nonpositive_or_nonfinite_values(self) -> None:
        adb = self.sdk / "platform-tools/adb"
        self.executable(adb, 'echo called >> "$EVENTS"\n')
        for setting in ("BAUKIT_QA_ADB_TIMEOUT_SECONDS", "BAUKIT_QA_BOOT_TIMEOUT_SECONDS"):
            for value in ("0", "-1", "nan", "inf"):
                with self.subTest(setting=setting, value=value):
                    result = subprocess.run(
                        ["python3", str(self.scripts / "android-adb.py"), "--wait-for-boot", str(adb), "emulator-5556"],
                        env={**self.environment, setting: value}, capture_output=True,
                        text=True, timeout=5, check=False,
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(f"{setting} must be a positive finite number", result.stderr)
                    self.assertFalse(self.events.exists())

    def test_android_setup_is_idempotent_and_prepares_oidc_browser(self) -> None:
        self.executable(self.sdk / "platform-tools/adb", r'''
printf '%s\n' "$*" >> "$EVENTS"
if [[ "$*" == devices ]]; then printf 'List of devices attached\n'; fi
if [[ "$*" == *'getprop sys.boot_completed' ]]; then echo 1; fi
''')
        self.executable(self.sdk / "emulator/emulator", "exit 0\n")
        self.executable(self.scripts / "services.sh", "exit 0\n")
        apk = self.root / "mobile/android/app/build/outputs/apk/release/app-release.apk"
        apk.parent.mkdir(parents=True)
        apk.touch()
{% if context.auth_oidc %}        flags = '-s emulator-5556 shell echo "chrome --disable-fre --no-first-run --no-default-browser-check" > /data/local/tmp/chrome-command-line'
        debug = '-s emulator-5556 shell settings put global debug_app com.android.chrome'
        stop = '-s emulator-5556 shell am force-stop com.android.chrome'
{% endif %}        for _ in range(2):
            self.events.unlink(missing_ok=True)
            self.run_script("android-env.sh")
            events = self.events.read_text().splitlines()
            launch = next(index for index, event in enumerate(events) if "shell monkey" in event)
{% if context.auth_oidc %}            for command in (flags, debug, stop):
                self.assertEqual(events.count(command), 1)
                self.assertLess(events.index(command), launch)
            self.assertLess(events.index(stop), events.index(flags))
{% else %}            self.assertFalse(any("chrome" in event for event in events))
{% endif %}

    def test_android_setup_stops_before_cleanup_when_device_probe_fails(self) -> None:
        self.executable(self.sdk / "emulator/emulator", 'echo emulator >> "$EVENTS"\n')
        self.executable(self.scripts / "services.sh", 'echo services >> "$EVENTS"\n')
        state = self.root / "mobile/.qa"
        state.mkdir()
        owned = state / "android-owned"
        owned.touch()
        apk = self.root / "mobile/android/app/build/outputs/apk/release/app-release.apk"
        apk.parent.mkdir(parents=True)
        apk.touch()
        for probe, expected_status in (("exit 7", 7), ("exec sleep 60", 124)):
            with self.subTest(probe=probe):
                owned.touch()
                self.events.unlink(missing_ok=True)
                self.executable(self.sdk / "platform-tools/adb", f'''
if [[ "$*" == devices ]]; then
  printf 'List of devices attached\\nemulator-5556\\tdevice\\n'
  {probe}
fi
if [[ "$*" == *'getprop sys.boot_completed' ]]; then echo 1; fi
''')
                result = subprocess.run(
                    ["bash", str(self.scripts / "android-env.sh")],
                    env={**self.environment, "BAUKIT_QA_ADB_TIMEOUT_SECONDS": "0.2"},
                    capture_output=True, text=True, check=False, timeout=5,
                )
                self.assertEqual(result.returncode, expected_status, result.stdout + result.stderr)
                self.assertFalse(self.events.exists())
                self.assertTrue(owned.exists())


if __name__ == "__main__":
    unittest.main()
