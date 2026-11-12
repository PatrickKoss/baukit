import os
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
AUTH_ENABLED: bool = "{{ context.auth_enabled }}".lower() == "true"
AUTH_OIDC: bool = "{{ context.auth_oidc }}".lower() == "true"
HAS_BACKEND: bool = "{{ context.backend }}".lower() == "true"


class AndroidScriptsTest(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.scripts = self.root / "mobile/scripts/qa"
        shutil.copytree(SCRIPTS, self.scripts, ignore=shutil.ignore_patterns("local.sh"))
        (self.scripts / "local.sh").write_text("# shellcheck shell=bash\n")
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
        for name in ("BAUKIT_QA_AVD_CONFIG", "BAUKIT_QA_GRADLE_WORKERS", "BAUKIT_QA_NATIVE_JOBS"):
            self.environment.pop(name, None)

    def prepare_sdk(self) -> Path:
        tools = self.sdk / "cmdline-tools/16111833/bin"
        self.executable(tools / "sdkmanager", "exit 0\n")
        self.executable(
            tools / "avdmanager",
            r"""
printf '%s\n' "$*" >> "$EVENTS"
config="$ANDROID_AVD_HOME/test-qa.avd/config.ini"
case "$1" in
  delete) rm -f "$config" ;;
  list) if [[ -f "$config" ]]; then echo "Name: test-qa"; fi ;;
  create)
    while [[ "$1" != --package ]]; do shift; done
    image="${2//;/\/}"
    mkdir -p "$(dirname "$config")"
    printf 'image.sysdir.1=%s/\nhw.device.name=pixel_6\nhw.ramSize=1536\nhw.keyboard=no\nhw.cpu.ncore=2\n' "$image" > "$config"
    ;;
esac
""",
        )
        config = self.avds / "test-qa.avd/config.ini"
        config.parent.mkdir(parents=True)
        return config

    def executable(self, path: Path, content: str) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("#!/usr/bin/env bash\nset -eu\n" + content)
        path.chmod(0o755)

    def run_script(self, name: str, *arguments: str) -> None:
        result = subprocess.run(
            ["bash", str(self.scripts / name), *arguments],
            env=self.environment,
            capture_output=True,
            text=True,
            check=False,
            timeout=15,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_default_avd_config_is_untouched(self) -> None:
        config = self.prepare_sdk()
        original = (
            b"image.sysdir.1=system-images/android-36/google_apis/x86_64/\r\n"
            b"hw.device.name = pixel_6\r\nhw.ramSize = 1536\r\n"
            b"hw.keyboard=no\r\nhw.cpu.ncore=2"
        )
        config.write_bytes(original)
        for _ in range(2):
            self.run_script("android-sdk.sh")
            self.assertEqual(config.read_bytes(), original)
        self.assertEqual(self.events.read_text().splitlines(), ["list avd", "list avd"])

    def test_avd_overrides_replace_keys_with_whitespace_once(self) -> None:
        config = self.prepare_sdk()
        self.environment["BAUKIT_QA_AVD_CONFIG"] = (
            "hw.ramSize=4096\thw.keyboard=yes\nhw.cpu.ncore=4 hw.gpu.mode=swiftshader "
            "hw.ramSize=8192"
        )
        config.write_text(
            "image.sysdir.1=system-images/android-36/google_apis/x86_64/\n"
            "hw.device.name=pixel_6\n"
            "abi.type=x86_64\nhw.ramSize = 1024\n hw.ramSize=2048\n"
            "hw.keyboard = no\n\thw.keyboard\t= no\nhw.gpu.mode=auto\nhw.cpu.ncore = 1\n\thw.cpu.ncore\t= 2\n"
        )
        for _ in range(2):
            self.run_script("android-sdk.sh")
            self.assertEqual(
                config.read_text().splitlines(),
                [
                    "image.sysdir.1=system-images/android-36/google_apis/x86_64/",
                    "hw.device.name=pixel_6",
                    "abi.type=x86_64",
                    "hw.ramSize=8192",
                    "hw.keyboard=yes",
                    "hw.cpu.ncore=4",
                    "hw.gpu.mode=swiftshader",
                ],
            )
        self.assertEqual(self.events.read_text().splitlines(), ["list avd", "list avd"])

    def test_malformed_avd_overrides_fail_before_sdk_setup(self) -> None:
        for value in (
            "hw.ramSize",
            "=4096",
            "hw.ramSize=",
            "hw.ramSize==4096",
            "hw.ramSize=4096 bad/key=1",
        ):
            with self.subTest(value=value):
                result = subprocess.run(
                    ["bash", str(self.scripts / "android-sdk.sh")],
                    env={**self.environment, "BAUKIT_QA_AVD_CONFIG": value},
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=5,
                )
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertIn(
                    "BAUKIT_QA_AVD_CONFIG must be a whitespace-separated list of key=value entries",
                    result.stderr,
                )
                self.assertFalse(self.sdk.exists())

    def test_new_avd_uses_stock_pixel_6(self) -> None:
        config = self.prepare_sdk()
        self.run_script("android-sdk.sh")
        self.assertEqual(
            self.events.read_text().splitlines(),
            [
                "list avd",
                "create avd --force --name test-qa --package system-images;android-36;google_apis;x86_64 --device pixel_6",
            ],
        )
        self.assertIn("hw.ramSize=1536\nhw.keyboard=no\nhw.cpu.ncore=2\n", config.read_text())

    def test_legacy_avd_is_recreated_once(self) -> None:
        config = self.prepare_sdk()
        for device in ("pixel_7", "pixel_6"):
            with self.subTest(device=device):
                config.write_text(
                    "image.sysdir.1=system-images/android-36/google_apis/x86_64/\n"
                    f"hw.device.name = {device}\n"
                    "hw.ramSize = 4096\nhw.keyboard = yes\nhw.cpu.ncore = 4\n"
                )
                self.events.unlink(missing_ok=True)
                self.run_script("android-sdk.sh")
                self.assertEqual(
                    self.events.read_text().splitlines(),
                    [
                        "delete avd --name test-qa",
                        "list avd",
                        "create avd --force --name test-qa --package system-images;android-36;google_apis;x86_64 --device pixel_6",
                    ],
                )
                self.assertIn(
                    "hw.device.name=pixel_6\nhw.ramSize=1536\nhw.keyboard=no\nhw.cpu.ncore=2\n",
                    config.read_text(),
                )
                self.events.unlink()
                self.run_script("android-sdk.sh")
                self.assertEqual(self.events.read_text().splitlines(), ["list avd"])

    def test_opt_in_preserves_old_values_on_pixel_6(self) -> None:
        config = self.prepare_sdk()
        self.environment["BAUKIT_QA_AVD_CONFIG"] = "hw.ramSize=4096 hw.keyboard=yes hw.cpu.ncore=4"
        original = (
            "image.sysdir.1=system-images/android-36/google_apis/x86_64/\n"
            "hw.device.name=pixel_6\nhw.ramSize=4096\nhw.keyboard=yes\nhw.cpu.ncore=4\n"
        )
        config.write_text(original)
        self.run_script("android-sdk.sh")
        self.assertEqual(config.read_text(), original)
        self.assertEqual(self.events.read_text().splitlines(), ["list avd"])

    def test_sdk_uses_pinned_tools_even_when_latest_exists(self) -> None:
        tools = self.sdk / "cmdline-tools/16111833/bin"
        self.executable(tools / "sdkmanager", 'printf "pinned-sdk %s\\n" "$*" >> "$EVENTS"\n')
        self.executable(tools / "avdmanager", 'echo "Name: test-qa"\n')
        self.executable(self.sdk / "cmdline-tools/latest/bin/sdkmanager", "exit 99\n")
        self.run_script("android-sdk.sh")
        events = self.events.read_text().splitlines()
        self.assertEqual(
            events,
            [
                f"pinned-sdk --sdk_root={self.sdk} --licenses",
                f"pinned-sdk --sdk_root={self.sdk} platform-tools emulator platforms;android-36 "
                "build-tools;36.0.0 system-images;android-36;google_apis;x86_64",
            ],
        )

    def test_installed_packages_skip_sdkmanager(self) -> None:
        tools = self.sdk / "cmdline-tools/16111833/bin"
        self.executable(tools / "sdkmanager", 'printf "sdk %s\\n" "$*" >> "$EVENTS"\n')
        self.executable(tools / "avdmanager", 'echo "Name: test-qa"\n')
        installed = [
            "platform-tools",
            "emulator",
            "platforms/android-36",
            "build-tools/36.0.0",
        ]
        for directory in installed:
            (self.sdk / directory).mkdir(parents=True)
            (self.sdk / directory / "package.xml").write_text("<package/>")
        self.run_script("android-sdk.sh")
        self.assertEqual(
            self.events.read_text().splitlines(),
            [
                f"sdk --sdk_root={self.sdk} --licenses",
                f"sdk --sdk_root={self.sdk} system-images;android-36;google_apis;x86_64",
            ],
        )

        image = self.sdk / "system-images/android-36/google_apis/x86_64"
        image.mkdir(parents=True)
        (image / "package.xml").write_text("<package/>")
        self.events.unlink()
        self.run_script("android-sdk.sh")
        self.assertFalse(self.events.exists())

    def test_download_uses_the_host_platform_and_cpu(self) -> None:
        for system, machine, platform in (
            ("Linux", "x86_64", "linux"),
            ("Darwin", "x86_64", "mac_x86_64"),
            ("Darwin", "arm64", "mac_arm64"),
        ):
            with self.subTest(system=system, machine=machine):
                sdk = self.root / platform
                binaries = self.root / "bin"
                self.executable(
                    binaries / "uname",
                    f'if [[ "$1" == -s ]]; then echo {system}; else echo {machine}; fi\n',
                )
                self.executable(
                    binaries / "curl",
                    r"""
while [[ "$1" != --output ]]; do shift; done
zip=$2
printf '%s\n' "$3" > "$EVENTS"
touch "$zip"
""",
                )
                self.executable(
                    binaries / "unzip",
                    r"""
while [[ "$1" != -d ]]; do shift; done
tools="$2/cmdline-tools/bin"
mkdir -p "$tools"
printf '#!/bin/sh\nexit 0\n' > "$tools/sdkmanager"
printf '#!/bin/sh\necho "Name: test-qa"\n' > "$tools/avdmanager"
chmod +x "$tools/sdkmanager" "$tools/avdmanager"
""",
                )
                self.environment.update(
                    {
                        "PATH": str(binaries) + os.pathsep + os.environ["PATH"],
                        "ANDROID_HOME": str(sdk),
                        "ANDROID_SDK_ROOT": str(sdk),
                    }
                )
                self.run_script("android-sdk.sh")
                self.assertEqual(
                    self.events.read_text().strip(),
                    f"https://dl.google.com/android/repository/commandlinetools-{platform}-16111833_latest.zip",
                )
                self.assertTrue((sdk / "cmdline-tools/16111833/bin/sdkmanager").is_file())

    def test_release_build_sets_loopback_without_host_interface_discovery(self) -> None:
        bin_dir = self.root / "bin"
        self.executable(bin_dir / "corepack", 'printf "corepack %s\\n" "$*" >> "$EVENTS"\n')
        self.environment["PATH"] = str(bin_dir) + os.pathsep + self.environment["PATH"]
        self.environment["JAVA_TOOL_OPTIONS"] = "-Dqa.test=1"
        self.executable(
            self.root / "mobile/node_modules/.bin/expo", 'printf "expo %s\\n" "$*" >> "$EVENTS"\n'
        )
        self.executable(
            self.root / "mobile/android/gradlew",
            r"""
printf 'gradle %s\n' "$*" >> "$EVENTS"
printf 'java-options %s\n' "$JAVA_TOOL_OPTIONS" >> "$EVENTS"
apk="$(dirname "$0")/app/build/outputs/apk/release/app-release.apk"
mkdir -p "$(dirname "$apk")"
touch "$apk"
""",
        )
        self.run_script("build-android.sh")
        self.assertEqual(
            self.events.read_text().splitlines(),
            [
                f"corepack pnpm --dir {self.root}/mobile install --frozen-lockfile",
                f"corepack pnpm --dir {self.root}/mobile run tokens",
                "expo prebuild --clean --platform android --no-install",
                f"gradle -p {self.root}/mobile/android --no-daemon --build-cache "
                "-PreactNativeDevServerIp=127.0.0.1 -PreactNativeArchitectures=x86_64 assembleRelease",
                "java-options -Dqa.test=1 --enable-native-access=ALL-UNNAMED",
            ],
        )

    def test_builds_isolate_the_metro_cache_per_public_environment(self) -> None:
        self.fake_build_tools()
        self.executable(
            self.root / "mobile/android/gradlew",
            r"""
printf 'tmpdir %s\n' "$TMPDIR" >> "$EVENTS"
apk="$(dirname "$0")/app/build/outputs/apk/release/app-release.apk"
mkdir -p "$(dirname "$apk")"
touch "$apk"
""",
        )
        for port in ("18080", "18080", "18090"):
            self.environment["BAUKIT_QA_API_PORT"] = port
            self.run_script("build-android.sh")
        tmpdirs = [
            event.removeprefix("tmpdir ")
            for event in self.events.read_text().splitlines()
            if event.startswith("tmpdir ")
        ]
        self.assertEqual(tmpdirs[0], tmpdirs[1])
        self.assertNotEqual(tmpdirs[0], tmpdirs[2])
        for tmpdir in tmpdirs:
            self.assertTrue(tmpdir.startswith(f"{self.scripts.parent.parent}/.qa/metro-cache/"))

    def ios_build(self, workspace: str, **environment: str) -> str:
        bin_dir = self.root / "bin"
        self.executable(bin_dir / "uname", "echo Darwin\n")
        self.executable(bin_dir / "pod", "exit 0\n")
        self.executable(
            bin_dir / "xcodebuild",
            'printf "%s\\n" "$*" >> "$EVENTS"\n'
            + 'mkdir -p "$PWD/ios-build/Build/Products/Release-iphonesimulator/App.app"\n',
        )
        self.executable(bin_dir / "corepack", "exit 0\n")
        self.environment["PATH"] = str(bin_dir) + os.pathsep + self.environment["PATH"]
        self.environment.update(environment)
        self.executable(
            self.root / "mobile/node_modules/.bin/expo",
            f'mkdir -p "$PWD/ios/{workspace}.xcworkspace"\n',
        )
        self.run_script("build-ios.sh")
        return next(event for event in self.events.read_text().splitlines() if "-scheme" in event)

    def test_ios_scheme_defaults_to_the_app_json_name_then_the_workspace(self) -> None:
        self.assertIn("-scheme FromWorkspace ", self.ios_build("FromWorkspace"))
        self.events.unlink()
        (self.root / "mobile/app.json").write_text('{"expo": {"name": "FromAppJson"}}')
        self.assertIn("-scheme FromAppJson ", self.ios_build("FromWorkspace"))
        self.events.unlink()
        self.assertIn(
            "-scheme Explicit ",
            self.ios_build("FromWorkspace", BAUKIT_QA_IOS_SCHEME="Explicit"),
        )

    def test_gate_build_compiles_debug_and_release_from_one_prebuild(self) -> None:
        bin_dir = self.root / "bin"
        self.executable(bin_dir / "corepack", 'printf "corepack %s\\n" "$*" >> "$EVENTS"\n')
        self.environment["PATH"] = str(bin_dir) + os.pathsep + self.environment["PATH"]
        self.environment["BAUKIT_QA_ASSEMBLE_DEBUG"] = "1"
        self.environment["BAUKIT_QA_GRADLE_WORKERS"] = "3"
        self.executable(
            self.root / "mobile/node_modules/.bin/expo", 'printf "expo %s\\n" "$*" >> "$EVENTS"\n'
        )
        self.executable(
            self.root / "mobile/android/gradlew",
            r"""
printf 'gradle %s\n' "$*" >> "$EVENTS"
apk="$(dirname "$0")/app/build/outputs/apk/release/app-release.apk"
mkdir -p "$(dirname "$apk")"
touch "$apk"
""",
        )
        self.run_script("build-android.sh")
        events = self.events.read_text().splitlines()
        self.assertEqual(events.count("expo prebuild --clean --platform android --no-install"), 1)
        common = f"gradle -p {self.root}/mobile/android --no-daemon --build-cache -PreactNativeDevServerIp=127.0.0.1 --max-workers 3"
        self.assertEqual(
            [event for event in events if event.startswith("gradle")],
            [
                f"{common} --stacktrace -PreactNativeArchitectures=x86_64 assembleDebug",
                f"{common} -PreactNativeArchitectures=x86_64 assembleRelease",
            ],
        )

    def test_native_jobs_cap_cmake_through_the_init_script(self) -> None:
        bin_dir = self.root / "bin"
        self.executable(bin_dir / "corepack", "exit 0\n")
        self.environment["PATH"] = str(bin_dir) + os.pathsep + self.environment["PATH"]
        self.environment["BAUKIT_QA_NATIVE_JOBS"] = "2"
        self.executable(self.root / "mobile/node_modules/.bin/expo", "exit 0\n")
        self.executable(
            self.root / "mobile/android/gradlew",
            r"""
printf 'gradle %s jobs=%s\n' "$*" "$BAUKIT_QA_NATIVE_JOBS" >> "$EVENTS"
apk="$(dirname "$0")/app/build/outputs/apk/release/app-release.apk"
mkdir -p "$(dirname "$apk")"
touch "$apk"
""",
        )
        self.run_script("build-android.sh")
        init_script = self.scripts / "native-jobs.init.gradle"
        self.assertIn(f"--init-script {init_script} ", self.events.read_text())
        self.assertIn("jobs=2", self.events.read_text())
        self.assertIn("-DCMAKE_JOB_POOLS=native=${nativeJobs}", init_script.read_text())

    def fake_build_tools(self) -> None:
        bin_dir = self.root / "bin"
        self.executable(bin_dir / "corepack", 'printf "corepack %s\\n" "$*" >> "$EVENTS"\n')
        self.environment["PATH"] = str(bin_dir) + os.pathsep + self.environment["PATH"]
        self.executable(
            self.root / "mobile/node_modules/.bin/expo", 'printf "expo %s\\n" "$*" >> "$EVENTS"\n'
        )
        self.executable(
            self.root / "mobile/android/gradlew",
            r"""
printf 'gradle %s api=%s\n' "$*" "$EXPO_PUBLIC_API_URL" >> "$EVENTS"
apk="$(dirname "$0")/app/build/outputs/apk/release/app-release.apk"
mkdir -p "$(dirname "$apk")"
touch "$apk"
""",
        )

    def test_build_hooks_extend_environment_dependencies_and_gradle(self) -> None:
        self.fake_build_tools()
        (self.scripts / "local.sh").write_text(
            "api_port=8280\n"
            "qa_gradle_args=(-PreactNativeDevServerPort=8099)\n"
            'qa_build_env() { echo "hook env $EXPO_PUBLIC_API_URL" >> "$EVENTS"; }\n'
            'qa_install_dependencies() { echo "hook install" >> "$EVENTS"; }\n'
        )
        self.run_script("build-android.sh")
        events = self.events.read_text().splitlines()
        self.assertEqual(events[:2], ["hook env http://localhost:8280", "hook install"])
        self.assertFalse(any("install --frozen-lockfile" in event for event in events))
        gradle = next(event for event in events if event.startswith("gradle"))
        self.assertIn("-PreactNativeDevServerPort=8099", gradle)
        self.assertTrue(gradle.endswith("api=http://localhost:8280"))

    def test_build_runs_from_the_mobile_directory_and_exports_the_local_issuer(self) -> None:
        self.fake_build_tools()
        self.executable(
            self.root / "mobile/android/gradlew",
            r"""
printf 'cwd %s\n' "$PWD" >> "$EVENTS"
printf 'issuer %s\n' "${EXPO_PUBLIC_OIDC_ISSUER:-}${EXPO_PUBLIC_LOCAL_OIDC_ISSUER:-}" >> "$EVENTS"
apk="$(dirname "$0")/app/build/outputs/apk/release/app-release.apk"
mkdir -p "$(dirname "$apk")"
touch "$apk"
""",
        )
        self.environment["BAUKIT_QA_KEYCLOAK_PORT"] = "18099"
        subprocess.run(
            ["bash", str(self.scripts / "build-android.sh")],
            cwd=self.root,
            env=self.environment,
            capture_output=True,
            text=True,
            check=True,
            timeout=15,
        )
        events = self.events.read_text().splitlines()
        self.assertIn(f"cwd {(self.root / 'mobile').resolve()}", events)
        issuer = "http://localhost:18099/realms/{{ context.app_name }}" if AUTH_ENABLED else ""
        self.assertIn(f"issuer {issuer}", events)
        if AUTH_ENABLED and not AUTH_OIDC:
            self.assertEqual(
                (self.scripts / "build-android.sh")
                .read_text()
                .count("EXPO_PUBLIC_LOCAL_OIDC_ISSUER"),
                2,
            )

    def test_gradle_output_is_teed_to_the_opt_in_log(self) -> None:
        self.fake_build_tools()
        log = self.root / "logs/android-build.log"
        self.environment["BAUKIT_QA_GRADLE_LOG"] = str(log)
        self.executable(
            self.root / "mobile/android/gradlew",
            r"""
echo "gradle output"
apk="$(dirname "$0")/app/build/outputs/apk/release/app-release.apk"
mkdir -p "$(dirname "$apk")"
touch "$apk"
""",
        )
        self.run_script("build-android.sh")
        self.assertEqual(log.read_text(), "gradle output\n")

    def test_debug_variant_embeds_the_js_bundle_and_builds_only_debug(self) -> None:
        self.fake_build_tools()
        self.environment["BAUKIT_QA_APK_VARIANT"] = "debug"
        self.executable(
            self.root / "mobile/android/gradlew",
            r"""
printf 'gradle %s\n' "$*" >> "$EVENTS"
apk="$(dirname "$0")/app/build/outputs/apk/debug/app-debug.apk"
mkdir -p "$(dirname "$apk")"
touch "$apk"
""",
        )
        self.run_script("build-android.sh")
        events = self.events.read_text().splitlines()
        bundle = next(event for event in events if event.startswith("expo export:embed"))
        self.assertIn("--entry-file node_modules/expo-router/entry.js", bundle)
        self.assertIn(
            f"--bundle-output {self.root}/mobile/android/app/src/main/assets/index.android.bundle",
            bundle,
        )
        gradle = [event for event in events if event.startswith("gradle")]
        self.assertEqual(len(gradle), 1)
        self.assertTrue(gradle[0].endswith("assembleDebug"))
        self.assertLess(events.index(bundle), events.index(gradle[0]))

    def test_build_uses_the_environment_port_without_hooks(self) -> None:
        self.fake_build_tools()
        self.environment["BAUKIT_QA_API_PORT"] = "8281"
        self.run_script("build-android.sh")
        self.assertIn("api=http://localhost:8281", self.events.read_text())

    def test_sdk_platform_and_build_tools_are_set_independently(self) -> None:
        tools = self.sdk / "cmdline-tools/16111833/bin"
        self.executable(tools / "sdkmanager", 'printf "sdk %s\\n" "$*" >> "$EVENTS"\n')
        self.executable(tools / "avdmanager", 'echo "Name: test-qa"\n')
        self.environment["BAUKIT_QA_ANDROID_PLATFORM"] = "37.2"
        self.environment["BAUKIT_QA_ANDROID_BUILD_TOOLS"] = "37.0.0"
        self.run_script("android-sdk.sh")
        self.assertIn(
            "platforms;android-37.2 build-tools;37.0.0 system-images;android-36;google_apis;x86_64",
            self.events.read_text(),
        )

    def test_android_java_reports_usage_and_preserves_command_failure(self) -> None:
        for arguments, expected_status in (([], 2), (["bash", "-c", "exit 17"], 17)):
            with self.subTest(arguments=arguments):
                result = subprocess.run(
                    ["bash", str(self.scripts.parent / "android-java.sh"), *arguments],
                    env=self.environment,
                    capture_output=True,
                    text=True,
                    check=False,
                )
                self.assertEqual(result.returncode, expected_status, result.stdout + result.stderr)

    def test_avd_is_recreated_when_api_tag_or_architecture_changes(self) -> None:
        config = self.prepare_sdk()
        for old_image in (
            "system-images/android-35/google_apis/x86_64/",
            "system-images/android-36/google_apis_playstore/x86_64/",
            "system-images/android-36/google_apis/arm64-v8a/",
        ):
            with self.subTest(old_image=old_image):
                config.write_text(f"image.sysdir.1 = {old_image}\nhw.device.name=pixel_6\n")
                self.events.unlink(missing_ok=True)
                self.run_script("android-sdk.sh")
                self.assertIn("delete avd --name test-qa", self.events.read_text())
                self.assertIn("create avd", self.events.read_text())
                self.assertIn(
                    "image.sysdir.1=system-images/android-36/google_apis/x86_64/",
                    config.read_text(),
                )
                self.events.unlink()
                self.run_script("android-sdk.sh")
                self.assertNotIn("delete avd", self.events.read_text())
                self.assertNotIn("create avd", self.events.read_text())
        self.environment["BAUKIT_QA_ANDROID_IMAGE_TAG"] = "google_apis_playstore"
        self.run_script("android-sdk.sh")
        self.assertIn(
            "image.sysdir.1=system-images/android-36/google_apis_playstore/x86_64/",
            config.read_text(),
        )

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
                    env=self.environment,
                    capture_output=True,
                    text=True,
                    timeout=5,
                    check=False,
                )
                self.assertEqual(result.returncode, 124, result.stdout + result.stderr)
                self.assertIn("timed out", result.stderr)
                if arguments[-1] == "devices":
                    self.assertIn(" ".join(arguments), result.stderr)
                self.assertLess(time.monotonic() - started, 5)

    def health_probe(self, report: Path | None = None) -> subprocess.CompletedProcess[str]:
        arguments = [
            "python3",
            str(self.scripts / "android-adb.py"),
            "--check-health",
            str(self.sdk / "platform-tools/adb"),
            "emulator-5556",
        ]
        if report is not None:
            arguments.append(str(report))
        return subprocess.run(
            arguments,
            env=self.environment,
            capture_output=True,
            text=True,
            check=False,
            timeout=5,
        )

    def test_health_probe_appends_triggering_events_and_matching_anr_summary(self) -> None:
        self.executable(
            self.sdk / "platform-tools/adb",
            r"""
case "$*" in
  *logcat*)
    printf 'am_anr: [0,123,com.android.systemui,1,first reason]\nam_crash: [0,456,com.android.inputmethod.latin,1,second reason]\n' ;;
  *'dumpsys activity lastanr')
    printf 'ANR time: 2026-10-09 12:00:00\nPID: 123\nReason: executing service com.android.systemui/.SystemUIService\nErrorId: 1234\nFrozen: false\nprivate stack frame\n' ;;
esac
""",
        )
        report = self.root / "mobile/.qa/android-health.log"
        first = self.health_probe(report)
        self.assertEqual(first.returncode, 1, first.stderr)
        evidence = report.read_text()
        self.assertRegex(evidence, r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z emulator-5556 status=1\n")
        for line in (
            "am_anr: [0,123,com.android.systemui,1,first reason]",
            "am_crash: [0,456,com.android.inputmethod.latin,1,second reason]",
            "ANR time: 2026-10-09 12:00:00",
            "PID: 123",
            "Reason: executing service com.android.systemui/.SystemUIService",
            "ErrorId: 1234",
            "Frozen: false",
        ):
            self.assertIn(line, first.stderr)
            self.assertIn(line, evidence)
        self.assertNotIn("private stack frame", evidence)
        second = self.health_probe(report)
        self.assertEqual(second.returncode, 1, second.stderr)
        self.assertTrue(report.read_text().startswith(evidence))
        self.assertEqual(report.read_text().count("emulator-5556 status=1"), 2)

    def test_health_probe_rejects_anr_details_for_another_package(self) -> None:
        self.executable(
            self.sdk / "platform-tools/adb",
            r"""
case "$*" in
  *logcat*) echo 'am_anr: [0,123,com.android.systemui,1,reason]' ;;
  *'dumpsys activity lastanr')
    printf 'ANR in com.android.systemui.extra\nPID: 456\nReason: unrelated ANR\nstack: com.android.systemui\n' ;;
esac
""",
        )
        report = self.root / "mobile/.qa/android-health.log"
        result = self.health_probe(report)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn(
            "qa: logcat am_anr shows the ANR above; dumpsys lastanr has no record of it",
            result.stderr,
        )
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
                self.executable(
                    self.sdk / "platform-tools/adb",
                    f"""
case "$*" in
  *logcat*) echo 'am_anr: [0,123,com.android.systemui,1,reason]' ;;
  *'dumpsys activity lastanr') {command} ;;
esac
""",
                )
                result = self.health_probe(report)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn("am_anr: [0,123,com.android.systemui,1,reason]", result.stderr)
                self.assertIn(message, result.stderr)
                self.assertIn(message, report.read_text())

    def test_manual_health_probe_without_report_retains_event(self) -> None:
        self.executable(
            self.sdk / "platform-tools/adb",
            r"""
if [[ "$*" == *logcat* ]]; then
  echo 'am_crash: [0,123,com.android.systemui,1,reason]'
else
  exit 99
fi
""",
        )
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
                        [
                            "python3",
                            str(self.scripts / "android-adb.py"),
                            "--wait-for-boot",
                            str(adb),
                            "emulator-5556",
                        ],
                        env={**self.environment, setting: value},
                        capture_output=True,
                        text=True,
                        timeout=5,
                        check=False,
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(f"{setting} must be a positive finite number", result.stderr)
                    self.assertFalse(self.events.exists())

    def test_android_setup_is_idempotent_and_prepares_oidc_browser(self) -> None:
        emulator_args = self.root / "emulator-args"
        self.environment["EMULATOR_ARGS"] = str(emulator_args)
        self.executable(
            self.sdk / "platform-tools/adb",
            r"""
printf '%s\n' "$*" >> "$EVENTS"
if [[ "$*" == devices ]]; then printf 'List of devices attached\n'; fi
if [[ "$*" == *'getprop sys.boot_completed' && -f "$EMULATOR_ARGS" ]]; then echo 1; fi
if [[ "$*" == *'pidof com.android.systemui' ]]; then echo 321; fi
if [[ "$*" == *'dumpsys window displays' ]]; then echo '  mCurrentFocus=Window{1 u0 launcher}'; fi
if [[ "$*" == *'cat /proc/stat' ]]; then printf 'cpu  10 0 10 80 0 0 0 0 0 0\ncpu0 10 0 10 80 0 0 0 0 0 0\n'; fi
if [[ "$*" == *'emu avd snapshot save '* ]]; then mkdir -p "$ANDROID_AVD_HOME/test-qa.avd/snapshots/${@: -1}"; fi
if [[ "$*" == *'ip -4 route get 10.0.2.2' ]]; then echo '10.0.2.2 dev wlan0 src 10.0.2.16 uid 0'; fi
""",
        )
        self.environment["BAUKIT_QA_POLL_SECONDS"] = "0"
        self.executable(
            self.sdk / "emulator/emulator",
            '{ flock 9; printf "%s\\n" "$@" -- >> "$EMULATOR_ARGS"; } 9>"$EMULATOR_ARGS.lock"\n',
        )
        self.executable(self.scripts / "services.sh", "exit 0\n")
        (self.scripts / "local.sh").write_text(
            "qa_emulator_args=(-accel off)\nqa_reverse_ports=(18015)\n"
        )
        apk = self.root / "mobile/android/app/build/outputs/apk/release/app-release.apk"
        apk.parent.mkdir(parents=True)
        apk.touch()
        flags = '-s emulator-5556 shell echo "chrome --disable-fre --no-first-run --no-default-browser-check --force-renderer-accessibility=complete" > /data/local/tmp/chrome-command-line'
        debug = "-s emulator-5556 shell settings put global debug_app com.android.chrome"
        stop = "-s emulator-5556 shell am force-stop com.android.chrome"
        for headless, wifi in (("0", "1"), ("1", "1"), ("1", "0")):
            self.environment["BAUKIT_QA_HEADLESS"] = headless
            self.environment["BAUKIT_QA_ANDROID_WIFI"] = wifi
            self.events.unlink(missing_ok=True)
            emulator_args.unlink(missing_ok=True)
            shutil.rmtree(self.avds / "test-qa.avd/snapshots", ignore_errors=True)
            for marker in self.avds.glob("test-qa.avd/baukit-clean-*.ready"):
                marker.unlink()
            self.run_script("android-env.sh")
            features = "-Vulkan,-WiFiPacketStream" if wifi == "1" else "-Vulkan"
            snapshot = self.snapshot_name(features=features)
            expected_args = [
                "-avd",
                "test-qa",
                "-port",
                "5556",
                "-no-snapshot-save",
                "-no-audio",
                "-no-boot-anim",
                "-gpu",
                "swiftshader_indirect",
                "-feature",
                features,
                "-no-snapshot-load",
            ]
            expected_args.extend([*(["-no-window"] if headless == "1" else []), "-accel", "off"])
            cold, *shared, rest = emulator_args.read_text().split("--\n")
            self.assertEqual(cold.splitlines(), expected_args)
            self.assertEqual(rest, "")
            for port, launch in zip(
                ("5556", "5558"), sorted(shared, key=lambda args: args.split()[3]), strict=True
            ):
                quick = [*expected_args[:3], port, *expected_args[4:]]
                load = quick.index("-no-snapshot-load")
                quick[load : load + 1] = ["-snapshot", snapshot, "-read-only"]
                self.assertEqual(launch.splitlines(), quick)
            events = self.events.read_text().splitlines()
            activity = "dev.baukit.{{ context.app_crate }}/.MainActivity"
            command = f"-s emulator-5556 shell am start -n {activity} -W"
            self.assertEqual(events.count(command), 1)
            launch = events.index(command)
            self.assertEqual(events.count("-s emulator-5556 reverse tcp:18015 tcp:18015"), 1)
            self.assertFalse(any("monkey" in event for event in events))
            self.assertEqual(events.count(f"-s emulator-5556 emu avd snapshot save {snapshot}"), 1)
            if wifi == "0":
                self.assertFalse(
                    any("svc wifi" in event or "route get" in event for event in events)
                )
            for serial in ("emulator-5556", "emulator-5558") if wifi == "1" else ():
                enable = events.index(f"-s {serial} shell svc wifi enable")
                self.assertLess(
                    events.index(f"-s {serial} shell cmd connectivity airplane-mode disable"),
                    enable,
                )
                self.assertLess(enable, events.index(f"-s {serial} shell ip -4 route get 10.0.2.2"))
            if AUTH_ENABLED:
                for command in (flags, debug, stop):
                    self.assertEqual(events.count(command), 1)
                    self.assertLess(events.index(command), launch)
                self.assertLess(events.index(stop), events.index(flags))
            else:
                self.assertFalse(any("chrome" in event for event in events))

    def test_maestro_passes_local_hook_arguments_to_every_run(self) -> None:
        maestro = self.root / "bin/maestro"
        self.executable(maestro, 'printf "%s\\n" "$*" >> "$EVENTS"\n')
        self.environment["PATH"] = f"{maestro.parent}:{self.environment['PATH']}"
        self.environment["BAUKIT_QA_POLL_SECONDS"] = "0"
        mobile = self.root / "mobile"
        (mobile / ".maestro").mkdir(parents=True)
        (mobile / ".maestro/one.yaml").write_text("appId: x\n")
        (mobile / ".qa").mkdir()
        (mobile / ".qa/ios-udid").write_text("udid-1\n")
        (self.scripts / "local.sh").write_text("qa_maestro_args=(--exclude-tags kompass)\n")

        result = subprocess.run(
            ["bash", str(self.scripts / "run-maestro.sh"), "ios"],
            env=self.environment,
            capture_output=True,
            text=True,
            check=False,
            timeout=15,
        )

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("--exclude-tags kompass .maestro/one.yaml", self.events.read_text())

    def prepare_maestro_run(self, devices: list[str], local_hooks: str = "") -> None:
        self.executable(
            self.sdk / "platform-tools/adb",
            r"""
case "$*" in
  *get-state*) echo device ;;
  *"logcat -v threadtime"*)
    echo "$$" >> "$LOGCAT_PIDS"
    echo "logcat line for $2"
    exec sleep 60
    ;;
  *"dumpsys package com.android.chrome"*) echo "    versionName=130.0.6723.102" ;;
esac
""",
        )
        self.executable(
            self.root / "bin/maestro",
            r"""
previous=""
for argument in "$@"; do
  if [[ "$previous" == --device && "$argument" == "${FAIL_DEVICE:-}" ]]; then exit 1; fi
  previous="$argument"
done
""",
        )
        self.executable(self.scripts / "android-env.sh", "exit 0\n")
        self.environment["PATH"] = f"{self.root / 'bin'}:{self.environment['PATH']}"
        self.environment["BAUKIT_QA_POLL_SECONDS"] = "0"
        self.environment["LOGCAT_PIDS"] = str(self.root / "logcat-pids")
        mobile = self.root / "mobile"
        (mobile / ".maestro").mkdir(parents=True)
        (mobile / ".maestro/one.yaml").write_text("appId: x\n")
        (mobile / ".qa").mkdir()
        (mobile / ".qa/android-serial").write_text("".join(f"{device}\n" for device in devices))
        (self.scripts / "local.sh").write_text(local_hooks)

    def run_maestro_script(self, **environment: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["bash", str(self.scripts / "run-maestro.sh"), "android"],
            env={**self.environment, **environment},
            capture_output=True,
            text=True,
            check=False,
            timeout=30,
        )

    def assert_logcat_processes_stopped(self) -> None:
        for pid in (self.root / "logcat-pids").read_text().split():
            with self.assertRaises(ProcessLookupError):
                os.kill(int(pid), 0)

    def test_maestro_drops_device_evidence_when_the_run_passes(self) -> None:
        self.prepare_maestro_run(["emulator-5556"])
        result = self.run_maestro_script()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        debug = self.root / "mobile/maestro-debug"
        self.assertEqual(list(debug.glob("logcat-*")), [])
        self.assertEqual(list(debug.glob("chrome-version-*")), [])
        self.assertEqual(list((self.root / "mobile/.qa").glob("logcat.*")), [])
        self.assert_logcat_processes_stopped()

    def test_maestro_keeps_logcat_and_chrome_version_when_a_flow_fails(self) -> None:
        self.prepare_maestro_run(["emulator-5556"])
        result = self.run_maestro_script(FAIL_DEVICE="emulator-5556")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        debug = self.root / "mobile/maestro-debug"
        self.assertIn(
            "logcat line for emulator-5556",
            (debug / "logcat-emulator-5556.txt").read_text(),
        )
        self.assertIn(
            "versionName=130.0.6723.102",
            (debug / "chrome-version-emulator-5556.txt").read_text(),
        )
        self.assertEqual(list((self.root / "mobile/.qa").glob("logcat.*")), [])
        self.assert_logcat_processes_stopped()

    def test_maestro_keeps_evidence_only_for_the_failing_device(self) -> None:
        self.prepare_maestro_run(
            ["emulator-5556", "emulator-5558"],
            'qa_maestro_flows() { echo .maestro/one.yaml; }\n',
        )
        result = self.run_maestro_script(FAIL_DEVICE="emulator-5558")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        debug = self.root / "mobile/maestro-debug"
        self.assertEqual(
            sorted(path.name for path in debug.glob("logcat-*")), ["logcat-emulator-5558.txt"]
        )
        self.assert_logcat_processes_stopped()

    def start_foreign_process(self) -> subprocess.Popen:
        process = subprocess.Popen(["sleep", "30"])  # noqa: S607 -- fixed test helper
        self.addCleanup(process.kill)
        self.addCleanup(process.wait)
        return process

    @unittest.skipUnless(HAS_BACKEND, "services.sh hooks need a generated backend")
    def test_stop_refuses_a_pid_file_that_names_a_foreign_process(self) -> None:
        fake_bin = self.root / "bin"
        self.executable(fake_bin / "docker", "exit 0\n")
        self.environment["PATH"] = f"{fake_bin}:{self.environment['PATH']}"
        state = self.root / "mobile/.qa"
        state.mkdir(parents=True)
        foreign = self.start_foreign_process()
        pid_file = state / "backend.pid"
        pid_file.write_text(f"{foreign.pid}\n")
        result = subprocess.run(
            ["bash", str(self.scripts / "services.sh"), "stop"],
            env=self.environment,
            capture_output=True,
            text=True,
            check=False,
            timeout=15,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("refusing to stop it", result.stderr)
        self.assertIsNone(foreign.poll())
        self.assertTrue(pid_file.exists())

    @unittest.skipUnless(HAS_BACKEND, "services.sh hooks need a generated backend")
    def test_stop_pid_file_stops_the_expected_binary_for_workers(self) -> None:
        fake_bin = self.root / "bin"
        self.executable(fake_bin / "docker", "exit 0\n")
        self.environment["PATH"] = f"{fake_bin}:{self.environment['PATH']}"
        state = self.root / "mobile/.qa"
        state.mkdir(parents=True)
        worker = self.root / "backend/target/debug/worker"
        self.executable(
            worker,
            'exec -a "$0" python3 -c "print(\'ready\', flush=True); import time; time.sleep(30)"\n',
        )
        process = subprocess.Popen([str(worker)], stdout=subprocess.PIPE, text=True)
        self.addCleanup(process.stdout.close)
        self.addCleanup(process.kill)
        self.addCleanup(process.wait)
        self.assertEqual(process.stdout.readline().strip(), "ready")
        pid_file = state / "worker.pid"
        pid_file.write_text(f"{process.pid}\n")
        (self.scripts / "local.sh").write_text(
            'qa_stop_services() { stop_pid_file "$state_dir/worker.pid" worker'
            ' "$qa_backend_bin_dir/worker"; }\n'
        )
        self.run_script("services.sh", "stop")
        self.assertIsNotNone(process.wait(timeout=10))
        self.assertFalse(pid_file.exists())

    @unittest.skipUnless(HAS_BACKEND, "services.sh hooks need a generated backend")
    def test_services_hooks_can_replace_the_stack_and_move_ports_and_state(self) -> None:
        fake_bin = self.root / "bin"
        self.executable(fake_bin / "docker", 'echo "docker $*" >> "$EVENTS"\n')
        self.environment["PATH"] = f"{fake_bin}:{self.environment['PATH']}"
        override = self.root / "override.yml"
        override.touch()
        moved = self.root / "moved-state"
        self.environment["BAUKIT_QA_STATE_DIR"] = str(moved)
        self.environment["BAUKIT_QA_COMPOSE_OVERRIDE"] = str(override)
        (self.scripts / "local.sh").write_text(
            "api_port=8280\n"
            'qa_services_start() { echo "hook start $BAUKIT_QA_API_PORT" >> "$EVENTS"; }\n'
            'qa_services_stop() { echo "hook stop" >> "$EVENTS"; }\n'
        )
        self.run_script("services.sh")
        self.run_script("services.sh", "stop")
        self.assertEqual(
            self.events.read_text().splitlines(),
            ["hook stop", "hook start 8280", "hook stop"],
        )
        self.assertTrue(moved.is_dir())
        self.assertFalse((self.root / "mobile/.qa").exists())
        (self.scripts / "local.sh").unlink()
        self.events.unlink()
        self.run_script("services.sh", "stop")
        self.assertIn(f"-f {override} down --volumes", self.events.read_text())

    @unittest.skipUnless(HAS_BACKEND, "services.sh hooks need a generated backend")
    def test_backend_binaries_come_from_the_configured_directory(self) -> None:
        fake_bin = self.root / "bin"
        custom = self.root / "custom-bins"
        self.executable(fake_bin / "docker", 'echo "docker $*" >> "$EVENTS"\n')
        self.executable(fake_bin / "curl", "exit 0\n")
        self.executable(
            fake_bin / "cargo",
            f'mkdir -p "{custom}"\n'
            f'printf "#!/bin/sh\\necho migrate-ran >> \\"$EVENTS\\"\\n" > "{custom}/migrate"\n'
            f'printf "#!/bin/bash\\nexec -a \\"\\$0\\" sleep 30\\n" > "{custom}/api"\n'
            f'chmod +x "{custom}/migrate" "{custom}/api"\n',
        )
        self.environment["PATH"] = f"{fake_bin}:{self.environment['PATH']}"
        (self.scripts / "local.sh").write_text(f'qa_backend_bin_dir="{custom}"\n')
        self.run_script("services.sh")
        self.run_script("services.sh", "stop")
        self.assertIn("migrate-ran", self.events.read_text())

    @unittest.skipUnless(HAS_BACKEND, "services.sh hooks need a generated backend")
    def test_services_local_hooks_run_around_the_api(self) -> None:
        fake_bin = self.root / "bin"
        self.executable(fake_bin / "docker", 'echo "docker $*" >> "$EVENTS"\n')
        self.executable(fake_bin / "curl", "exit 0\n")
        self.executable(
            fake_bin / "cargo",
            'echo "cargo $*" >> "$EVENTS"\n'
            f'target="{self.root}/backend/target/debug"\n'
            'mkdir -p "$target"\n'
            'printf "#!/bin/sh\\nexit 0\\n" > "$target/migrate"\n'
            'printf "#!/bin/bash\\nexec -a \\"\\$0\\" sleep 30\\n" > "$target/api"\n'
            'chmod +x "$target/migrate" "$target/api"\n',
        )
        self.environment["PATH"] = f"{fake_bin}:{self.environment['PATH']}"
        (self.scripts / "local.sh").write_text(
            "qa_compose_services=(postgres redis)\n"
            "qa_cargo_bins=(seed-catalog)\n"
            'qa_services_env() { echo "hook env" >> "$EVENTS"; }\n'
            'qa_before_api() { echo "hook before-api" >> "$EVENTS"; }\n'
            'qa_after_api() { echo "hook after-api" >> "$EVENTS"; }\n'
            'qa_stop_services() { echo "hook stop" >> "$EVENTS"; }\n'
        )

        self.run_script("services.sh")
        started = self.events.read_text().splitlines()
        self.run_script("services.sh", "stop")
        stopped = self.events.read_text().splitlines()[len(started) :]

        up = next(index for index, event in enumerate(started) if "up -d --wait" in event)
        self.assertTrue(started[up].endswith("up -d --wait postgres redis"))
        build = next(
            index for index, event in enumerate(started) if event.startswith("cargo build")
        )
        self.assertTrue(started[build].endswith("--bin migrate --bin api --bin seed-catalog"))
        order = [
            started.index(event) for event in ("hook env", "hook before-api", "hook after-api")
        ]
        self.assertEqual(order, sorted(order))
        self.assertLess(up, order[0])
        self.assertLess(order[0], build)
        self.assertLess(build, order[1])
        self.assertEqual(stopped[0], "hook stop")

    def test_hanging_app_install_is_bounded(self) -> None:
        emulator_args = self.root / "emulator-args"
        self.environment["EMULATOR_ARGS"] = str(emulator_args)
        self.executable(
            self.sdk / "platform-tools/adb",
            r"""
if [[ "$*" == *'install -r '* ]]; then exec sleep 60; fi
if [[ "$*" == devices ]]; then printf 'List of devices attached\n'; fi
if [[ "$*" == *'getprop sys.boot_completed' && -f "$EMULATOR_ARGS" ]]; then echo 1; fi
if [[ "$*" == *'pidof com.android.systemui' ]]; then echo 321; fi
if [[ "$*" == *'dumpsys window displays' ]]; then echo '  mCurrentFocus=Window{1 u0 launcher}'; fi
if [[ "$*" == *'cat /proc/stat' ]]; then printf 'cpu  10 0 10 80 0 0 0 0 0 0\ncpu0 10 0 10 80 0 0 0 0 0 0\n'; fi
if [[ "$*" == *'emu avd snapshot save '* ]]; then mkdir -p "$ANDROID_AVD_HOME/test-qa.avd/snapshots/${@: -1}"; fi
""",
        )
        self.executable(
            self.sdk / "emulator/emulator", 'printf "%s\\n" "$@" -- >> "$EMULATOR_ARGS"\n'
        )
        self.executable(self.scripts / "services.sh", "exit 0\n")
        apk = self.root / "mobile/android/app/build/outputs/apk/release/app-release.apk"
        apk.parent.mkdir(parents=True)
        apk.touch()
        started = time.monotonic()
        result = subprocess.run(
            ["bash", str(self.scripts / "android-env.sh")],
            env={
                **self.environment,
                "BAUKIT_QA_POLL_SECONDS": "0",
                "BAUKIT_QA_HEADLESS": "1",
                "BAUKIT_QA_ANDROID_WIFI": "0",
                "BAUKIT_QA_ADB_TIMEOUT_SECONDS": "2",
            },
            capture_output=True,
            text=True,
            check=False,
            timeout=30,
        )
        self.assertEqual(result.returncode, 124, result.stdout + result.stderr)
        self.assertIn("ADB command timed out", result.stderr)
        self.assertIn("install -r", result.stderr)
        self.assertLess(time.monotonic() - started, 30)

    def snapshot_name(
        self,
        gpu: str = "swiftshader_indirect",
        version: str = "{{ context.template_version }}",
        features: str = "",
    ) -> str:
        result = subprocess.run(
            [
                "python3",
                str(self.scripts / "android-adb.py"),
                "--snapshot-name",
                str(self.avds / "test-qa.avd/config.ini"),
                str(self.sdk),
                gpu,
                features,
                version,
            ],
            capture_output=True,
            text=True,
            check=True,
            timeout=15,
        )
        return result.stdout.strip()

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
                self.executable(
                    self.sdk / "platform-tools/adb",
                    f"""
if [[ "$*" == devices ]]; then
  printf 'List of devices attached\\nemulator-5556\\tdevice\\n'
  {probe}
fi
if [[ "$*" == *'getprop sys.boot_completed' ]]; then echo 1; fi
""",
                )
                result = subprocess.run(
                    ["bash", str(self.scripts / "android-env.sh")],
                    env={**self.environment, "BAUKIT_QA_ADB_TIMEOUT_SECONDS": "0.2"},
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=5,
                )
                self.assertEqual(result.returncode, expected_status, result.stdout + result.stderr)
                self.assertEqual(self.events.read_text().splitlines(), ["services"])
                self.assertTrue(owned.exists())
                self.assertTrue(serial.exists())


if __name__ == "__main__":
    unittest.main()
