#!/usr/bin/env python3
"""Assert that every component belongs to the same baukit release train."""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from pathlib import Path

from release_packages import typescript_manifests

ROOT = Path(__file__).resolve().parents[1]
RELEASE_TRAIN_GROUP = "baukit-release-train"


def fail(message: str) -> None:
    print(f"version coherence error: {message}", file=sys.stderr)
    raise SystemExit(1)


def changeset_problems(config: dict, package_names: set[str]) -> list[str]:
    groups = config.get("fixed", [])
    if len(groups) != 1:
        return ["typescript/.changeset/config.json must define one fixed group"]
    group = groups[0]
    listed = set(group)
    problems = [
        f"TypeScript package {name} is missing from the Changesets fixed group"
        for name in sorted(package_names - listed)
    ]
    problems += [
        f"Changesets fixed group lists {name}, which is not a TypeScript package"
        for name in sorted(listed - package_names)
    ]
    if len(group) != len(listed):
        problems.append("Changesets fixed group contains duplicate packages")
    return problems


def release_plz_problems(release_plz: dict, crate_names: set[str]) -> list[str]:
    """Returns every mismatch between release-plz packages and workspace crates."""
    packages = release_plz.get("package", [])
    listed = {package.get("name") for package in packages}
    problems = [
        f"workspace crate {name} is missing from rust/release-plz.toml"
        for name in sorted(crate_names - listed)
    ]
    problems += [
        f"rust/release-plz.toml lists {name}, which is not a workspace crate"
        for name in sorted(listed - crate_names, key=str)
    ]
    for package in packages:
        if package.get("version_group") != RELEASE_TRAIN_GROUP:
            problems.append(
                f"rust/release-plz.toml package {package.get('name')} must set "
                f'version_group = "{RELEASE_TRAIN_GROUP}"'
            )
        if package.get("publish") is not False:
            problems.append(
                f"rust/release-plz.toml package {package.get('name')} must set publish = false"
            )
    return problems


def load_toml(relative_path: str) -> dict:
    with (ROOT / relative_path).open("rb") as manifest:
        return tomllib.load(manifest)


def check_rust_workspace() -> str:
    rust_workspace = load_toml("rust/Cargo.toml")["workspace"]
    rust_version = rust_workspace["package"]["version"]
    if rust_workspace["package"].get("license") != "MIT":
        fail("rust/Cargo.toml must declare the MIT license for the workspace")

    for name, dependency in rust_workspace["dependencies"].items():
        if not name.startswith("baukit-") or not isinstance(dependency, dict):
            continue
        dependency_version = dependency.get("version", "").removeprefix("=")
        if dependency_version != rust_version:
            fail(
                f"workspace dependency {name} requires {dependency.get('version')!r}, "
                f"expected ={rust_version}"
            )
    return rust_version


def crate_version(path: Path, rust_version: str) -> tuple[str, str]:
    with path.open("rb") as manifest:
        manifest_data = tomllib.load(manifest)
    package = manifest_data["package"]
    if package.get("publish") is False:
        fail(f"{package['name']} is published to crates.io; drop its publish flag")
    # A dev-dependency on a sibling crate must stay path-only. Carrying a
    # version makes cargo resolve it against the registry when packaging,
    # which the crates form a cycle through and no publish order can satisfy.
    for name, dependency in manifest_data.get("dev-dependencies", {}).items():
        if not name.startswith("baukit-") or not isinstance(dependency, dict):
            continue
        if "version" in dependency or dependency.get("workspace") is True:
            fail(
                f"{package['name']} dev-depends on {name} with a version; "
                "use a path-only dependency so cargo package can strip it"
            )
    version = package["version"]
    if isinstance(version, dict) and version.get("workspace") is True:
        version = rust_version
    if not isinstance(version, str):
        fail(f"cannot resolve the version in {path.relative_to(ROOT)}")
    return package["name"], version


def check_rust_crates(rust_version: str) -> int:
    crate_versions = dict(
        crate_version(path, rust_version)
        for path in sorted((ROOT / "rust/crates").glob("*/Cargo.toml"))
    )

    problems = release_plz_problems(load_toml("rust/release-plz.toml"), set(crate_versions))
    if problems:
        fail("; ".join(problems))

    bad_crates = {
        name: version for name, version in crate_versions.items() if version != rust_version
    }
    if bad_crates:
        fail(f"Rust crates differ from workspace version {rust_version}: {bad_crates}")

    cli_version = load_toml("cli/Cargo.toml")["package"]["version"]
    if cli_version != rust_version:
        fail(f"Rust is {rust_version}, but the CLI is {cli_version}")
    return len(crate_versions)


def links_local_package(path: Path, dependency: str, requirement: str) -> bool:
    linked_package = (path.parent / requirement.removeprefix("file:")).resolve()
    linked_manifest = linked_package / "package.json"
    if not linked_manifest.is_file():
        return False
    return json.loads(linked_manifest.read_text()).get("name") == dependency


def allowed_dev_requirement(path: Path, dependency: str, requirement: str) -> bool:
    if requirement == "workspace:*":
        return True
    return requirement.startswith("file:") and links_local_package(path, dependency, requirement)


def check_baukit_requirements(name: str, path: Path, package: dict, rust_version: str) -> None:
    for dependency_group in (
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ):
        for dependency, requirement in package.get(dependency_group, {}).items():
            if not dependency.startswith("@baukit/"):
                continue
            if dependency_group == "devDependencies" and allowed_dev_requirement(
                path, dependency, requirement
            ):
                continue
            if requirement != f"^{rust_version}":
                fail(
                    f"{name} {dependency_group} requires {dependency} "
                    f"at {requirement!r}, expected ^{rust_version}"
                )


def typescript_package_version(name: str, path: Path, rust_version: str) -> str:
    package = json.loads(path.read_text())
    if package.get("private") is not None:
        fail(f"{name} is published publicly; drop its private flag")
    if package.get("publishConfig", {}).get("access") != "public":
        fail(f'{name} must set publishConfig.access to "public"')
    if package.get("license") != "MIT":
        fail(f"{name} must declare the MIT license")
    check_baukit_requirements(name, path, package, rust_version)
    return package["version"]


def check_typescript_packages(rust_version: str) -> int:
    manifests = typescript_manifests(ROOT)
    changeset_config = json.loads((ROOT / "typescript/.changeset/config.json").read_text())
    problems = changeset_problems(changeset_config, set(manifests))
    if problems:
        fail("; ".join(problems))

    package_versions = {
        name: typescript_package_version(name, path, rust_version)
        for name, path in manifests.items()
    }
    if not package_versions:
        fail("no @baukit/* TypeScript packages were found")
    ts_versions = set(package_versions.values())
    if len(ts_versions) != 1:
        fail(f"TypeScript packages do not share one version: {package_versions}")
    ts_version = ts_versions.pop()
    if ts_version != rust_version:
        fail(f"Rust is {rust_version}, but TypeScript is {ts_version}")
    return len(package_versions)


def check_templates(rust_version: str) -> None:
    template_file = ROOT / "templates/VERSION"
    if not template_file.exists():
        return
    template_version = template_file.read_text().strip()
    if template_version != rust_version:
        fail(f"Rust is {rust_version}, but templates/VERSION is {template_version}")


def chart_values(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in path.read_text().splitlines():
        if line.startswith(("version:", "appVersion:")):
            key, value = line.split(":", 1)
            values[key] = value.strip().strip('"')
    return values


def check_charts(rust_version: str) -> int:
    chart_count = 0
    for path in sorted((ROOT / "deploy").glob("**/Chart.yaml")):
        if ".local-state" in path.parts:
            continue
        values = chart_values(path)
        chart_version = values.get("version", "")
        app_version = values.get("appVersion", "")
        chart_count += 1
        if chart_version != rust_version or app_version != rust_version:
            fail(
                f"Rust is {rust_version}, but {path.relative_to(ROOT)} has "
                f"version {chart_version!r} and appVersion {app_version!r}"
            )
    return chart_count


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--tag",
        help="also require a tag of the form vX.Y.Z to match the train",
    )
    args = parser.parse_args()

    rust_version = check_rust_workspace()
    crate_count = check_rust_crates(rust_version)
    package_count = check_typescript_packages(rust_version)
    check_templates(rust_version)
    chart_count = check_charts(rust_version)

    if args.tag:
        expected_tag = f"v{rust_version}"
        if args.tag != expected_tag:
            fail(f"tag is {args.tag}, expected {expected_tag}")

    print(
        f"baukit release train {rust_version} is coherent "
        f"({crate_count} crates, CLI, {package_count} packages, "
        f"{chart_count} charts)"
    )


if __name__ == "__main__":
    main()
