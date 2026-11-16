from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

CHECK_PATH = Path(__file__).resolve().parent / "check-version-coherence.py"
SPEC = importlib.util.spec_from_file_location("check_version_coherence", CHECK_PATH)
assert SPEC is not None and SPEC.loader is not None
check = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = check
SPEC.loader.exec_module(check)

CRATES = {"baukit-core", "baukit-http"}


def package(name: str, **overrides: object) -> dict:
    entry = {"name": name, "version_group": check.RELEASE_TRAIN_GROUP, "publish": False}
    entry.update(overrides)
    return entry


def config(*packages: dict) -> dict:
    return {"workspace": {}, "package": list(packages)}


class ChangesetCoverageTest(unittest.TestCase):
    def test_one_fixed_group_covering_all_packages_passes(self) -> None:
        self.assertEqual(
            check.changeset_problems({"fixed": [["@baukit/navigation"]]}, {"@baukit/navigation"}),
            [],
        )

    def test_a_package_missing_from_the_fixed_group_is_reported(self) -> None:
        self.assertEqual(
            check.changeset_problems({"fixed": [[]]}, {"@baukit/navigation"}),
            ["TypeScript package @baukit/navigation is missing from the Changesets fixed group"],
        )

    def test_a_missing_package_directory_is_reported(self) -> None:
        self.assertEqual(
            check.changeset_problems({"fixed": [["@baukit/gone"]]}, set()),
            ["Changesets fixed group lists @baukit/gone, which is not a TypeScript package"],
        )

    def test_duplicate_packages_are_reported(self) -> None:
        self.assertEqual(
            check.changeset_problems(
                {"fixed": [["@baukit/navigation", "@baukit/navigation"]]}, {"@baukit/navigation"}
            ),
            ["Changesets fixed group contains duplicate packages"],
        )

    def test_zero_or_multiple_groups_are_reported(self) -> None:
        for groups in ([], [["@baukit/a11y-core"], ["@baukit/navigation"]]):
            with self.subTest(groups=groups):
                self.assertEqual(
                    check.changeset_problems({"fixed": groups}, {"@baukit/navigation"}),
                    ["typescript/.changeset/config.json must define one fixed group"],
                )


class ReleasePackageDiscoveryTest(unittest.TestCase):
    def test_every_package_directory_requires_a_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "typescript/packages/navigation").mkdir(parents=True)
            with self.assertRaises(FileNotFoundError):
                check.typescript_manifests(root)

    def test_duplicate_or_unscoped_names_are_rejected(self) -> None:
        for names, error in (
            (["@baukit/navigation", "@baukit/navigation"], "duplicate TypeScript package name"),
            (["navigation"], "must name an @baukit/"),
        ):
            with self.subTest(names=names), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                for index, name in enumerate(names):
                    path = root / "typescript/packages" / str(index)
                    path.mkdir(parents=True)
                    (path / "package.json").write_text(json.dumps({"name": name}))
                with self.assertRaisesRegex(ValueError, error):
                    check.typescript_manifests(root)


class ReleasePlzCoverageTest(unittest.TestCase):
    def test_every_crate_listed_in_the_train_passes(self) -> None:
        release_plz = config(package("baukit-core"), package("baukit-http"))
        self.assertEqual(check.release_plz_problems(release_plz, CRATES), [])

    def test_a_missing_crate_is_reported(self) -> None:
        problems = check.release_plz_problems(config(package("baukit-core")), CRATES)
        self.assertEqual(
            problems, ["workspace crate baukit-http is missing from rust/release-plz.toml"]
        )

    def test_a_listed_crate_that_is_not_in_the_workspace_is_reported(self) -> None:
        release_plz = config(package("baukit-core"), package("baukit-http"), package("baukit-gone"))
        self.assertEqual(
            check.release_plz_problems(release_plz, CRATES),
            ["rust/release-plz.toml lists baukit-gone, which is not a workspace crate"],
        )

    def test_a_crate_outside_the_train_or_published_is_reported(self) -> None:
        release_plz = config(
            package("baukit-core", version_group="other"),
            package("baukit-http", publish=True),
        )
        problems = check.release_plz_problems(release_plz, CRATES)
        self.assertEqual(len(problems), 2)
        self.assertIn("baukit-core must set version_group", problems[0])
        self.assertIn("baukit-http must set publish = false", problems[1])

    def test_the_repository_config_covers_every_workspace_crate(self) -> None:
        crates = set()
        for manifest in (check.ROOT / "rust/crates").glob("*/Cargo.toml"):
            with manifest.open("rb") as handle:
                crates.add(tomllib.load(handle)["package"]["name"])
        with (check.ROOT / "rust/release-plz.toml").open("rb") as handle:
            release_plz = tomllib.load(handle)
        self.assertEqual(check.release_plz_problems(release_plz, crates), [])


class ChartTest(unittest.TestCase):
    TRAIN = (
        "for chart in deploy/chart/app/Chart.yaml\n  deploy/observability-platform/Chart.yaml; do\n"
    )

    def repo(self, charts: dict[str, str], train: str = TRAIN) -> Path:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        (root / "scripts").mkdir()
        (root / "scripts/release-train.sh").write_text(train)
        for relative, version in charts.items():
            path = root / relative / "Chart.yaml"
            path.parent.mkdir(parents=True)
            path.write_text(f'version: {version}\nappVersion: "{version}"\n')
        return root

    def test_charts_on_the_train_pass(self) -> None:
        root = self.repo({"deploy/chart/app": "1.2.3", "deploy/observability-platform": "1.2.3"})
        self.assertEqual(check.chart_problems(root, "1.2.3"), (2, []))

    def test_a_chart_off_the_train_is_reported(self) -> None:
        root = self.repo({"deploy/chart/app": "1.2.3", "deploy/observability-platform": "1.2.2"})
        _, problems = check.chart_problems(root, "1.2.3")
        self.assertEqual(len(problems), 1)
        self.assertIn("deploy/observability-platform/Chart.yaml has version '1.2.2'", problems[0])

    def test_a_chart_the_release_train_does_not_bump_is_reported(self) -> None:
        root = self.repo(
            {"deploy/chart/app": "1.2.3", "deploy/observability-platform": "1.2.3"},
            train="for chart in deploy/chart/app/Chart.yaml; do\n",
        )
        self.assertEqual(
            check.chart_problems(root, "1.2.3"),
            (
                2,
                ["scripts/release-train.sh does not bump deploy/observability-platform/Chart.yaml"],
            ),
        )

    def test_the_repository_charts_pass(self) -> None:
        version = check.check_rust_workspace()
        _, problems = check.chart_problems(check.ROOT, version)
        self.assertEqual(problems, [])


if __name__ == "__main__":
    unittest.main()
