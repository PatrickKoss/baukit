import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent


class AndroidSdkSetupTest(unittest.TestCase):
    def test_new_and_existing_avds_use_four_cores_without_duplicate_keys(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sdk = root / "sdk"
            avds = root / "avds"
            tools = sdk / "cmdline-tools/13114758/bin"
            tools.mkdir(parents=True)
            (tools / "sdkmanager").write_text("#!/bin/sh\nexit 0\n")
            (tools / "avdmanager").write_text('''#!/bin/sh
mkdir -p "$ANDROID_AVD_HOME/test.avd"
touch "$ANDROID_AVD_HOME/test.ini"
printf 'hw.cpu.ncore = 1\nhw.cpu.ncore=2\nhw.gpu.mode=auto\n' > "$ANDROID_AVD_HOME/test.avd/config.ini"
''')
            for tool in tools.iterdir():
                tool.chmod(0o755)
            environment = {
                **os.environ,
                "ANDROID_HOME": str(sdk),
                "ANDROID_SDK_ROOT": str(sdk),
                "ANDROID_AVD_HOME": str(avds),
                "BAUKIT_ANDROID_AVD": "test",
            }
            for _ in range(2):
                result = subprocess.run(
                    ["bash", str(ROOT / "scripts/android-sdk-setup.sh")],
                    env=environment,
                    text=True,
                    capture_output=True,
                    check=False,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    (avds / "test.avd/config.ini").read_text().splitlines(),
                    ["hw.gpu.mode=auto", "hw.cpu.ncore=4"],
                )

    def test_native_gate_and_ci_builds_set_the_loopback_gradle_property(self) -> None:
        result = subprocess.run(
            ["make", "-n", "native-android-gate"],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = [result.stdout]
        commands.extend(
            (ROOT / path).read_text()
            for path in (
                ".github/workflows/ci.yml",
                "templates/common/.github/workflows/native.yml",
            )
        )
        for command in commands:
            gradle_lines = [
                line
                for line in command.splitlines()
                if "gradlew" in line and "assembleDebug" in line
            ]
            self.assertTrue(gradle_lines, command)
            for line in gradle_lines:
                self.assertIn("-PreactNativeDevServerIp=127.0.0.1", line)


if __name__ == "__main__":
    unittest.main()
