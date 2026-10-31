import io
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/check-gradle-deprecations.py"
TEMPLATE_GRADLE = 'apply plugin: "com.android.application"\nandroid {\n    namespace "com.helloworld"\n}\n'
PROPERTY = (
    "Properties should be assigned using the 'propName = value' syntax. Setting a property via the "
    "Gradle-generated 'propName value' or 'propName(value)' syntax in Groovy DSL has been deprecated. "
    "Use assignment ('{name} = <value>') instead."
)


def write_template(project: Path) -> None:
    archive = project / "node_modules/expo/template.tgz"
    archive.parent.mkdir(parents=True)
    payload = TEMPLATE_GRADLE.encode()
    with tarfile.open(archive, "w:gz") as bundle:
        member = tarfile.TarInfo("package/android/app/build.gradle")
        member.size = len(payload)
        bundle.addfile(member, io.BytesIO(payload))


def deprecation(message: str, *frames: str) -> str:
    return "\n".join([message, *(f"\tat {frame}" for frame in frames)])


class CheckGradleDeprecationsTest(unittest.TestCase):
    def run_check(self, project: Path, log: str) -> subprocess.CompletedProcess[str]:
        log_path = project / "assemble.log"
        log_path.write_text(log)
        return subprocess.run(
            [sys.executable, str(SCRIPT), "--project", str(project), str(log_path)],
            text=True,
            capture_output=True,
            check=False,
        )

    def project(self, directory: str) -> Path:
        project = Path(directory)
        write_template(project)
        gradle = project / "android/app/build.gradle"
        gradle.parent.mkdir(parents=True)
        gradle.write_text(
            'apply plugin: "com.android.application"\nandroid {\n    namespace "dev.baukit.fixture"\n'
            "    compileSdk 36\n}\n"
        )
        library = project / "node_modules/.pnpm/lib/node_modules/lib/android/build.gradle"
        library.parent.mkdir(parents=True)
        library.write_text("android {\n    namespace 'lib'\n}\n")
        return project

    def test_template_package_and_toolchain_deprecations_pass(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            project = self.project(directory)
            log = "\n".join(
                [
                    deprecation(
                        PROPERTY.format(name="namespace"),
                        "com.android.build.gradle.internal.dsl.BaseAppModuleExtension_Decorated.namespace(Unknown Source)",
                        f"build_x.doCall({project}/android/app/build.gradle:3)",
                    ),
                    deprecation(
                        PROPERTY.format(name="namespace"),
                        f"build_y.doCall({project}/node_modules/.pnpm/lib/node_modules/lib/android/build.gradle:2)",
                    ),
                    deprecation(
                        "Declaring dependencies using multi-string notation has been deprecated. Use single-string.",
                        "org.gradle.api.Dependencies.create(Dependencies.java:1)",
                        "com.android.build.gradle.internal.res.Aapt2FromMaven$Companion.create(Aapt2FromMaven.kt:139)",
                        f"build_z.doCall({project}/android/app/build.gradle:1)",
                    ),
                    f"{project}/node_modules/lib/Module.java:70: warning: [removal] x() has been deprecated",
                ]
            )
            result = self.run_check(project, log)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("expo template\tandroid/app/build.gradle:3", result.stdout)
            self.assertIn("third-party package\tnode_modules/lib/android/build.gradle:2", result.stdout)
            self.assertIn("third-party toolchain\tcom.android.build.gradle.internal.res.Aapt2FromMaven", result.stdout)
            self.assertIn("3 Gradle deprecations, 0 from Baukit-owned files", result.stdout)

    def test_deprecation_from_a_line_outside_the_expo_template_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            project = self.project(directory)
            log = deprecation(
                PROPERTY.format(name="compileSdk"),
                f"build_x.doCall({project}/android/app/build.gradle:4)",
            )
            result = self.run_check(project, log)
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertIn("baukit\tandroid/app/build.gradle:4", result.stdout)

    def test_deprecation_from_a_script_outside_android_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            project = self.project(directory)
            script = project / "plugins/extra.gradle"
            script.parent.mkdir()
            script.write_text("ext.x = 1\n")
            log = deprecation(PROPERTY.format(name="url"), f"build_x.doCall({script}:1)")
            result = self.run_check(project, log)
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertIn("baukit\tplugins/extra.gradle:1", result.stdout)


if __name__ == "__main__":
    unittest.main()
