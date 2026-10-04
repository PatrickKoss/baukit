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


if __name__ == "__main__":
    unittest.main()
