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
        shutil.copyfile(SCRIPTS.parent / "android-java.sh", self.scripts.parent / "android-java.sh")
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
        tools = self.sdk / "cmdline-tools/16111833/bin"
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
        tools = self.sdk / "cmdline-tools/16111833/bin"
        self.executable(tools / "sdkmanager", 'printf "pinned-sdk %s\\n" "$*" >> "$EVENTS"\n')
        self.executable(tools / "avdmanager", 'echo "Name: test-qa"\n')
        self.executable(self.sdk / "cmdline-tools/latest/bin/sdkmanager", "exit 99\n")
        self.run_script("android-sdk.sh")
        events = self.events.read_text().splitlines()
        self.assertEqual(events, [
            f"pinned-sdk --sdk_root={self.sdk} --licenses",
            f"pinned-sdk --sdk_root={self.sdk} platform-tools emulator platforms;android-36 "
            "build-tools;36.0.0 system-images;android-36;google_apis;x86_64",
        ])

    def test_download_uses_the_host_platform_and_cpu(self) -> None:
        for system, machine, platform in (
            ("Linux", "x86_64", "linux"),
            ("Darwin", "x86_64", "mac_x86_64"),
            ("Darwin", "arm64", "mac_arm64"),
        ):
            with self.subTest(system=system, machine=machine):
                sdk = self.root / platform
                binaries = self.root / "bin"
                self.executable(binaries / "uname", f'if [[ "$1" == -s ]]; then echo {system}; else echo {machine}; fi\n')
                self.executable(binaries / "curl", r'''
while [[ "$1" != --output ]]; do shift; done
zip=$2
printf '%s\n' "$3" > "$EVENTS"
touch "$zip"
''')
                self.executable(binaries / "unzip", r'''
while [[ "$1" != -d ]]; do shift; done
tools="$2/cmdline-tools/bin"
mkdir -p "$tools"
printf '#!/bin/sh\nexit 0\n' > "$tools/sdkmanager"
printf '#!/bin/sh\necho "Name: test-qa"\n' > "$tools/avdmanager"
chmod +x "$tools/sdkmanager" "$tools/avdmanager"
''')
                self.environment.update({
                    "PATH": str(binaries) + os.pathsep + os.environ["PATH"],
                    "ANDROID_HOME": str(sdk), "ANDROID_SDK_ROOT": str(sdk),
                })
                self.run_script("android-sdk.sh")
                self.assertEqual(self.events.read_text().strip(),
                    f"https://dl.google.com/android/repository/commandlinetools-{platform}-16111833_latest.zip")
                self.assertTrue((sdk / "cmdline-tools/16111833/bin/sdkmanager").is_file())

    def test_release_build_sets_loopback_without_host_interface_discovery(self) -> None:
        bin_dir = self.root / "bin"
        self.executable(bin_dir / "corepack", 'printf "corepack %s\\n" "$*" >> "$EVENTS"\n')
        self.environment["PATH"] = str(bin_dir) + os.pathsep + self.environment["PATH"]
        self.environment["JAVA_TOOL_OPTIONS"] = "-Dqa.test=1"
        self.executable(self.root / "mobile/node_modules/.bin/expo", 'printf "expo %s\\n" "$*" >> "$EVENTS"\n')
        self.executable(self.root / "mobile/android/gradlew", r'''
printf 'gradle %s\n' "$*" >> "$EVENTS"
printf 'java-options %s\n' "$JAVA_TOOL_OPTIONS" >> "$EVENTS"
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
            "java-options -Dqa.test=1 --enable-native-access=ALL-UNNAMED",
        ])

    def test_android_java_reports_usage_and_preserves_command_failure(self) -> None:
        for arguments, expected_status in (([], 2), (["bash", "-c", "exit 17"], 17)):
            with self.subTest(arguments=arguments):
                result = subprocess.run(
                    ["bash", str(self.scripts.parent / "android-java.sh"), *arguments],
                    env=self.environment, capture_output=True, text=True, check=False,
                )
                self.assertEqual(result.returncode, expected_status, result.stdout + result.stderr)

    def test_avd_is_recreated_when_api_tag_or_architecture_changes(self) -> None:
        tools = self.sdk / "cmdline-tools/16111833/bin"
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

    def health_probe(self, report: Path | None = None) -> subprocess.CompletedProcess[str]:
        arguments = [
            "python3", str(self.scripts / "android-adb.py"), "--check-health",
            str(self.sdk / "platform-tools/adb"), "emulator-5556",
        ]
        if report is not None:
            arguments.append(str(report))
        return subprocess.run(
            arguments, env=self.environment, capture_output=True, text=True,
            check=False, timeout=5,
        )

    def test_health_probe_appends_triggering_events_and_matching_anr_summary(self) -> None:
        self.executable(self.sdk / "platform-tools/adb", r'''
case "$*" in
  *logcat*)
    printf 'am_anr: [0,123,com.android.systemui,1,first reason]\nam_crash: [0,456,com.android.inputmethod.latin,1,second reason]\n' ;;
  *'dumpsys activity lastanr')
    printf 'ANR time: 2026-10-09 12:00:00\nPID: 123\nReason: executing service com.android.systemui/.SystemUIService\nErrorId: 1234\nFrozen: false\nprivate stack frame\n' ;;
esac
''')
        report = self.root / "mobile/.qa/android-health.log"
        first = self.health_probe(report)
        self.assertEqual(first.returncode, 1, first.stderr)
        evidence = report.read_text()
        self.assertRegex(evidence, r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z emulator-5556 status=1\n")
        for line in (
            "am_anr: [0,123,com.android.systemui,1,first reason]",
            "am_crash: [0,456,com.android.inputmethod.latin,1,second reason]",
            "ANR time: 2026-10-09 12:00:00", "PID: 123",
            "Reason: executing service com.android.systemui/.SystemUIService",
            "ErrorId: 1234", "Frozen: false",
        ):
            self.assertIn(line, first.stderr)
            self.assertIn(line, evidence)
        self.assertNotIn("private stack frame", evidence)
        second = self.health_probe(report)
        self.assertEqual(second.returncode, 1, second.stderr)
        self.assertTrue(report.read_text().startswith(evidence))
        self.assertEqual(report.read_text().count("emulator-5556 status=1"), 2)

    def test_health_probe_rejects_anr_details_for_another_package(self) -> None:
        self.executable(self.sdk / "platform-tools/adb", r'''
case "$*" in
  *logcat*) echo 'am_anr: [0,123,com.android.systemui,1,reason]' ;;
  *'dumpsys activity lastanr')
    printf 'ANR in com.android.systemui.extra\nPID: 456\nReason: unrelated ANR\nstack: com.android.systemui\n' ;;
esac
''')
        report = self.root / "mobile/.qa/android-health.log"
        result = self.health_probe(report)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("no matching critical-process ANR", result.stderr)
        self.assertNotIn("PID: 456", result.stderr)
        self.assertNotIn("com.android.systemui.extra", report.read_text())

    def test_health_probe_retains_evidence_when_anr_diagnostics_fail_or_time_out(self) -> None:
        self.environment["BAUKIT_QA_ADB_TIMEOUT_SECONDS"] = "0.2"
        report = self.root / "mobile/.qa/android-health.log"
        for command, message in (
            ("exit 7", "Android ANR diagnostics failed: exit 7"),
            ("exec sleep 60", "Android ANR diagnostics timed out"),
        ):
            with self.subTest(command=command):
                self.executable(self.sdk / "platform-tools/adb", f'''
case "$*" in
  *logcat*) echo 'am_anr: [0,123,com.android.systemui,1,reason]' ;;
  *'dumpsys activity lastanr') {command} ;;
esac
''')
                result = self.health_probe(report)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn("am_anr: [0,123,com.android.systemui,1,reason]", result.stderr)
                self.assertIn(message, result.stderr)
                self.assertIn(message, report.read_text())

    def test_manual_health_probe_without_report_retains_event(self) -> None:
        self.executable(self.sdk / "platform-tools/adb", r'''
if [[ "$*" == *logcat* ]]; then
  echo 'am_crash: [0,123,com.android.systemui,1,reason]'
else
  exit 99
fi
''')
        result = self.health_probe()
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("am_crash: [0,123,com.android.systemui,1,reason]", result.stderr)
        self.assertFalse((self.root / "mobile/.qa").exists())

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
            command = "-s emulator-5556 shell am start -n dev.baukit.{{ context.app_crate }}/.MainActivity -W"
            self.assertEqual(events.count(command), 1)
            launch = events.index(command)
            self.assertFalse(any("monkey" in event for event in events))
{% if context.auth_oidc %}            for command in (flags, debug, stop):
                self.assertEqual(events.count(command), 1)
                self.assertLess(events.index(command), launch)
            self.assertLess(events.index(stop), events.index(flags))
{% else %}            self.assertFalse(any("chrome" in event for event in events))
{% endif %}

    def test_android_setup_keeps_ownership_when_device_probe_fails(self) -> None:
        self.executable(self.sdk / "emulator/emulator", 'echo emulator >> "$EVENTS"\n')
        self.executable(self.scripts / "services.sh", 'echo services >> "$EVENTS"\n')
        state = self.root / "mobile/.qa"
        state.mkdir()
        owned = state / "android-owned"
        owned.touch()
        serial = state / "android-serial"
        serial.write_text("emulator-5556\n")
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
                self.assertEqual(self.events.read_text().splitlines(), ["services"])
                self.assertTrue(owned.exists())
                self.assertTrue(serial.exists())


if __name__ == "__main__":
    unittest.main()
