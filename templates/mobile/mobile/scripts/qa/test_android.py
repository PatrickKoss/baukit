import os
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
            env=self.environment, capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_avd_keys_with_whitespace_are_replaced_once(self) -> None:
        tools = self.sdk / "cmdline-tools/latest/bin"
        self.executable(tools / "sdkmanager", "exit 0\n")
        self.executable(tools / "avdmanager", 'echo "Name: test-qa"\n')
        config = self.avds / "test-qa.avd/config.ini"
        config.parent.mkdir(parents=True)
        config.write_text(
            "abi.type=x86_64\nhw.ramSize = 1024\n hw.ramSize=2048\n"
            "hw.keyboard = no\n\thw.keyboard\t= no\nhw.gpu.mode=auto\n"
        )
        for _ in range(2):
            self.run_script("android-sdk.sh")
            self.assertEqual(config.read_text().splitlines(), [
                "abi.type=x86_64", "hw.gpu.mode=auto",
                "hw.ramSize=4096", "hw.keyboard=yes",
            ])

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
{% if context.auth_oidc %}        flags = '-s emulator-5556 shell echo "chrome --no-first-run --no-default-browser-check" > /data/local/tmp/chrome-command-line'
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


if __name__ == "__main__":
    unittest.main()
