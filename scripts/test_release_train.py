from __future__ import annotations

import json
import os
import re
import shlex
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/release-train.sh"


def shell_order(script: str) -> list[str]:
    match = re.search(
        r"^ORDER=\((.*?)^\)", (ROOT / script).read_text(), re.MULTILINE | re.DOTALL
    )
    if match is None:
        raise AssertionError(f"{script} must declare ORDER")
    return shlex.split(match[1], comments=True)


def crate_dependencies(manifest: dict, workspace: dict) -> set[str]:
    dependencies = set()
    for group in ("dependencies", "build-dependencies"):
        for alias, requirement in manifest.get(group, {}).items():
            if isinstance(requirement, dict) and requirement.get("workspace") is True:
                requirement = workspace["dependencies"][alias]
            name = requirement.get("package", alias) if isinstance(requirement, dict) else alias
            if name.startswith("baukit-"):
                dependencies.add(name)
    for target in manifest.get("target", {}).values():
        dependencies.update(crate_dependencies(target, workspace))
    return dependencies


class ReleaseInventoryTest(unittest.TestCase):
    def test_fixed_group_and_publish_order_cover_every_package_directory(self) -> None:
        directories = sorted(
            path for path in (ROOT / "typescript/packages").iterdir() if path.is_dir()
        )
        packages = [
            json.loads((path / "package.json").read_text())["name"] for path in directories
        ]
        config = json.loads((ROOT / "typescript/.changeset/config.json").read_text())
        self.assertEqual(len(config["fixed"]), 1)
        self.assertCountEqual(config["fixed"][0], packages)
        self.assertCountEqual(
            shell_order("scripts/publish-packages.sh"), [path.name for path in directories]
        )

    def test_package_publish_order_follows_dependencies_and_peers(self) -> None:
        order = shell_order("scripts/publish-packages.sh")
        manifests = [ROOT / "typescript/packages" / name / "package.json" for name in order]
        packages = [json.loads(path.read_text()) for path in manifests]
        positions = {package["name"]: index for index, package in enumerate(packages)}
        for package in packages:
            dependencies = {
                name
                for group in ("dependencies", "peerDependencies", "optionalDependencies")
                for name in package.get(group, {})
                if name.startswith("@baukit/")
            }
            for dependency in sorted(dependencies):
                with self.subTest(package=package["name"], dependency=dependency):
                    self.assertIn(dependency, positions)
                    self.assertLess(positions[dependency], positions[package["name"]])

    def test_release_plz_and_publish_order_cover_every_crate_directory(self) -> None:
        directories = sorted(
            path for path in (ROOT / "rust/crates").iterdir() if path.is_dir()
        )
        crates = [
            tomllib.loads((path / "Cargo.toml").read_text())["package"]["name"]
            for path in directories
        ]
        config = tomllib.loads((ROOT / "rust/release-plz.toml").read_text())
        self.assertCountEqual([package["name"] for package in config["package"]], crates)
        self.assertCountEqual(
            shell_order("scripts/publish-crates.sh"), [path.name for path in directories]
        )

    def test_crate_publish_order_follows_dependencies(self) -> None:
        order = shell_order("scripts/publish-crates.sh")
        positions = {name: index for index, name in enumerate(order)}
        workspace = tomllib.loads((ROOT / "rust/Cargo.toml").read_text())["workspace"]
        for crate in order:
            manifest = tomllib.loads(
                (ROOT / "rust/crates" / crate / "Cargo.toml").read_text()
            )
            for dependency in sorted(crate_dependencies(manifest, workspace)):
                with self.subTest(crate=crate, dependency=dependency):
                    self.assertIn(dependency, positions)
                    self.assertLess(positions[dependency], positions[crate])


class ReleaseTrainCorepackTest(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        for path in (ROOT / "typescript/packages").glob("*/package.json"):
            destination = self.root / path.relative_to(ROOT)
            destination.parent.mkdir(parents=True)
            shutil.copyfile(path, destination)
        (self.root / "scripts").mkdir()
        shutil.copyfile(
            ROOT / "scripts/release_packages.py", self.root / "scripts/release_packages.py"
        )

    def test_release_train_uses_corepack_in_the_typescript_workspace(self) -> None:
        self.check_train("minor")

    def test_train_changeset_includes_a_new_package_without_a_script_edit(self) -> None:
        directory = self.root / "typescript/packages/release-test"
        directory.mkdir()
        (directory / "package.json").write_text(json.dumps({"name": "@baukit/release-test"}))
        self.check_train("patch")

    def check_train(self, bump: str) -> None:
        root = self.root
        (root / "typescript/.changeset").mkdir()
        (root / "rust").mkdir()
        (root / "rust/Cargo.toml").write_text(
            '[workspace.package]\nversion = "0.6.0"\n'
        )
        self.executable(root / "scripts/check-version-coherence.py", "#!/bin/sh\nexit 0\n")
        binaries = root / "bin"
        binaries.mkdir()
        self.executable(
            binaries / "git",
            '#!/bin/sh\ncase "$1" in\n'
            'rev-parse) printf "%s\\n" "$TEST_REPO_ROOT";;\n'
            'status) exit 0;;\nesac\n',
        )
        self.executable(
            binaries / "corepack",
            '#!/bin/sh\nprintf "%s\\n" "$PWD" "$@" > "$TEST_COREPACK_LOG"\nexit 17\n',
        )
        self.executable(binaries / "pnpm", "#!/bin/sh\nexit 99\n")
        log = root / "corepack.log"
        environment = {
            **os.environ,
            "PATH": f"{binaries}:{os.environ['PATH']}",
            "TEST_REPO_ROOT": str(root),
            "TEST_COREPACK_LOG": str(log),
        }
        result = subprocess.run(
            ["bash", str(SCRIPT), bump],
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertEqual(
            log.read_text().splitlines(),
            [str(root / "typescript"), "pnpm", "version-packages"],
        )
        changeset = (root / "typescript/.changeset/release-train.md").read_text()
        self.assertCountEqual(
            changeset.split("---")[1].strip().splitlines(),
            [
                f"'{json.loads(path.read_text())['name']}': {bump}"
                for path in (root / "typescript/packages").glob("*/package.json")
            ],
        )
        next_version = "0.7.0" if bump == "minor" else "0.6.1"
        self.assertIn(f"Release the coordinated baukit {next_version} train.", changeset)

    @staticmethod
    def executable(path: Path, contents: str) -> None:
        path.write_text(contents)
        path.chmod(0o755)


class ReleaseTrainFilesTest(unittest.TestCase):
    def test_common_template_records_shipped_entries_and_pending_fixes(self):
        source = (ROOT / "templates/common/CHANGELOG.md").read_text()
        pending, history = source.split("## [0.7.0] - 2026-10-04", 1)
        shipped, previous = history.split("## [0.6.0] - 2026-10-02", 1)
        self.assertIn("React and the react-dom override", pending)
        self.assertIn("Update pnpm to 12.9.1", shipped)
        self.assertIn("Added `dateTimeInput`", shipped)
        self.assertNotIn("Changed generated API DTOs", shipped)
        self.assertIn("Changed generated API DTOs", previous)
        self.assertIn("Added the opt-in MCP stdio package", previous)
        self.assertNotIn("pinned pnpm to 12.7.0", source)

    def test_patch_train_cuts_template_changelog_and_updates_cli_tags(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for relative in ["scripts/release-train.sh", "scripts/release_packages.py", "scripts/cli_install.py", "templates/common/CHANGELOG.md", "README.md"]:
                destination = root / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / relative, destination)
            files = {
                "rust/Cargo.toml": '[workspace.package]\nversion = "0.7.0"\n',
                "cli/Cargo.toml": '[package]\nname = "baukit-cli"\nversion = "0.7.0"\n',
                "rust/crates/baukit-core/CHANGELOG.md": "## [Unreleased]\n\n- Pending crate fix.\n",
                "typescript/packages/analytics-core/package.json": json.dumps({"name": "@baukit/analytics-core", "version": "0.7.1"}),
                "deploy/chart/baukit-app/Chart.yaml": 'version: 0.7.0\nappVersion: "0.7.0"\n',
                "deploy/observability/Chart.yaml": 'version: 0.7.0\nappVersion: "0.7.0"\n',
                "deploy/chart/baukit-app/README.md": '  - name: baukit-app\n    version: 0.7.0\n',
            }
            for relative, source in files.items():
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(source)
            (root / "typescript/.changeset").mkdir()
            before = (root / "templates/common/CHANGELOG.md").read_text()
            binaries = root / "bin"
            binaries.mkdir()
            executable = ReleaseTrainCorepackTest.executable
            real_git = shutil.which("git")
            self.assertIsNotNone(real_git)
            subprocess.run([real_git, "init", "--quiet", str(root)], check=True)
            subprocess.run([real_git, "add", "."], cwd=root, check=True)
            executable(binaries / "git", '#!/bin/sh\ncase "$1" in\nrev-parse) printf "%s\\n" "$TEST_REPO_ROOT";;\nstatus) exit 0;;\n*) exec ' + shlex.quote(real_git) + ' "$@";;\nesac\n')
            executable(binaries / "corepack", "#!/bin/sh\nexit 0\n")
            executable(binaries / "cargo", "#!/bin/sh\nexit 0\n")
            executable(binaries / "node", "#!/bin/sh\nprintf '0.7.1\\n'\n")
            executable(root / "scripts/check-version-coherence.py", "#!/bin/sh\nexit 0\n")
            environment = {**os.environ, "PATH": f"{binaries}:{os.environ['PATH']}", "TEST_REPO_ROOT": str(root), "RELEASE_DATE": "2026-10-05"}
            result = subprocess.run(["bash", str(root / "scripts/release-train.sh"), "patch"], cwd=root, env=environment, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            after = (root / "templates/common/CHANGELOG.md").read_text()
            self.assertEqual(after, before.replace("## [Unreleased]\n\n", "## [Unreleased]\n\n## [0.7.1] - 2026-10-05\n\n", 1))
            self.assertEqual(after.count("## [0.7.0]"), 1)
            self.assertEqual(after.count("## [0.6.0]"), 1)
            self.assertIn("--tag v0.7.1 --locked baukit-cli", (root / "README.md").read_text())
            self.assertEqual((root / "templates/VERSION").read_text(), "0.7.1\n")


if __name__ == "__main__":
    unittest.main()
