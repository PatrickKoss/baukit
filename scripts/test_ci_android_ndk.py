import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/ci-android-ndk.sh"
PINNED = "27.1.12297006"


class CiAndroidNdkTest(unittest.TestCase):
    def run_script(self, *arguments: str, **environment: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(SCRIPT), *arguments],
            env={**os.environ, **environment},
            text=True,
            capture_output=True,
            check=False,
        )

    def sdk(self, root: Path) -> Path:
        sdk = root / "sdk"
        tools = sdk / "cmdline-tools/latest/bin"
        tools.mkdir(parents=True)
        sdkmanager = tools / "sdkmanager"
        sdkmanager.write_text(
            '#!/bin/sh\nversion="${2#ndk;}"\nmkdir -p "$SDK/ndk/$version"\n'
            'printf "Pkg.Revision = %s\\n" "$version" > "$SDK/ndk/$version/source.properties"\n'
            'echo installed >> "$SDK/events"\n'
        )
        sdkmanager.chmod(0o755)
        bin_dir = root / "bin"
        bin_dir.mkdir()
        sudo = bin_dir / "sudo"
        sudo.write_text('#!/bin/sh\nexec "$@"\n')
        sudo.chmod(0o755)
        return sdk

    def test_version_reads_the_react_native_catalog(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            project = Path(directory)
            catalog = project / "node_modules/react-native/gradle/libs.versions.toml"
            catalog.parent.mkdir(parents=True)
            catalog.write_text(f'[versions]\nndkVersion = "{PINNED}"\n')
            result = self.run_script("version", str(project), ANDROID_HOME="/sdk", ANDROID_SDK_ROOT="/sdk")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.splitlines(), [f"version={PINNED}", f"path=/sdk/ndk/{PINNED}"])

    def test_install_keeps_only_the_pinned_ndk_and_skips_a_restored_one(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sdk = self.sdk(root)
            (sdk / "ndk/28.2.13676358").mkdir(parents=True)
            environment = {
                "ANDROID_SDK_ROOT": str(sdk),
                "SDK": str(sdk),
                "GITHUB_ACTIONS": "true",
                "PATH": f"{root / 'bin'}:{os.environ['PATH']}",
            }
            for _ in range(2):
                result = self.run_script("install", PINNED, **environment)
                self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(sorted(path.name for path in (sdk / "ndk").iterdir()), [PINNED])
            self.assertEqual((sdk / "events").read_text().splitlines(), ["installed"])

    def test_install_refuses_to_prune_outside_github_actions(self) -> None:
        result = self.run_script("install", PINNED, ANDROID_SDK_ROOT="/sdk", GITHUB_ACTIONS="")
        self.assertEqual(result.returncode, 2)


if __name__ == "__main__":
    unittest.main()
