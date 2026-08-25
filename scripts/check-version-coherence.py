#!/usr/bin/env python3
"""Assert that every component belongs to the same baukit release train."""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

EXPECTED_TYPESCRIPT_PACKAGES = {
    "@baukit/a11y-core",
    "@baukit/analytics-core",
    "@baukit/analytics-posthog-native",
    "@baukit/analytics-posthog-web",
    "@baukit/api-runtime",
    "@baukit/auth-native",
    "@baukit/auth-web",
    "@baukit/data-contracts",
    "@baukit/data-contracts-dexie",
    "@baukit/data-contracts-expo-sqlite",
    "@baukit/events",
    "@baukit/localization-core",
    "@baukit/preferences-core",
    "@baukit/pwa-web",
    "@baukit/sync-client",
    "@baukit/ui-tokens",
}


def fail(message: str) -> None:
    print(f"version coherence error: {message}", file=sys.stderr)
    raise SystemExit(1)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--tag",
        help="also require a tag of the form vX.Y.Z to match the train",
    )
    args = parser.parse_args()

    with (ROOT / "rust/Cargo.toml").open("rb") as manifest:
        rust_workspace = tomllib.load(manifest)["workspace"]
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

    crate_versions: dict[str, str] = {}
    for path in sorted((ROOT / "rust/crates").glob("*/Cargo.toml")):
        with path.open("rb") as manifest:
            package = tomllib.load(manifest)["package"]
        if package.get("publish") is False:
            fail(f"{package['name']} is published to crates.io; drop its publish flag")
        version = package["version"]
        if isinstance(version, dict) and version.get("workspace") is True:
            version = rust_version
        if not isinstance(version, str):
            fail(f"cannot resolve the version in {path.relative_to(ROOT)}")
        crate_versions[package["name"]] = version

    bad_crates = {name: version for name, version in crate_versions.items() if version != rust_version}
    if bad_crates:
        fail(f"Rust crates differ from workspace version {rust_version}: {bad_crates}")

    with (ROOT / "cli/Cargo.toml").open("rb") as manifest:
        cli_version = tomllib.load(manifest)["package"]["version"]
    if cli_version != rust_version:
        fail(f"Rust is {rust_version}, but the CLI is {cli_version}")

    package_versions: dict[str, str] = {}
    for path in sorted((ROOT / "typescript/packages").glob("*/package.json")):
        package = json.loads(path.read_text())
        name = package.get("name", "")
        if name.startswith("@baukit/"):
            if package.get("private") is not None:
                fail(f"{name} is published publicly; drop its private flag")
            if package.get("publishConfig", {}).get("access") != "public":
                fail(f'{name} must set publishConfig.access to "public"')
            if package.get("license") != "MIT":
                fail(f"{name} must declare the MIT license")
            package_versions[name] = package["version"]
            for dependency_group in (
                "dependencies",
                "devDependencies",
                "peerDependencies",
                "optionalDependencies",
            ):
                for dependency, requirement in package.get(dependency_group, {}).items():
                    if not dependency.startswith("@baukit/") or requirement == "workspace:*":
                        continue
                    if dependency_group == "devDependencies" and requirement.startswith("file:"):
                        linked_package = (path.parent / requirement.removeprefix("file:")).resolve()
                        linked_manifest = linked_package / "package.json"
                        if linked_manifest.is_file():
                            linked_name = json.loads(linked_manifest.read_text()).get("name")
                            if linked_name == dependency:
                                continue
                    if requirement != f"^{rust_version}":
                        fail(
                            f"{name} {dependency_group} requires {dependency} "
                            f"at {requirement!r}, expected ^{rust_version}"
                        )

    if not package_versions:
        fail("no @baukit/* TypeScript packages were found")
    missing_packages = EXPECTED_TYPESCRIPT_PACKAGES - package_versions.keys()
    if missing_packages:
        fail(f"missing TypeScript packages: {sorted(missing_packages)}")
    ts_versions = set(package_versions.values())
    if len(ts_versions) != 1:
        fail(f"TypeScript packages do not share one version: {package_versions}")
    ts_version = ts_versions.pop()
    if ts_version != rust_version:
        fail(f"Rust is {rust_version}, but TypeScript is {ts_version}")

    template_file = ROOT / "templates/VERSION"
    if template_file.exists():
        template_version = template_file.read_text().strip()
        if template_version != rust_version:
            fail(f"Rust is {rust_version}, but templates/VERSION is {template_version}")

    chart_versions: dict[str, tuple[str, str]] = {}
    for path in sorted((ROOT / "deploy").glob("**/Chart.yaml")):
        if ".local-state" in path.parts:
            continue
        values: dict[str, str] = {}
        for line in path.read_text().splitlines():
            if line.startswith(("version:", "appVersion:")):
                key, value = line.split(":", 1)
                values[key] = value.strip().strip('"')
        chart_version = values.get("version", "")
        app_version = values.get("appVersion", "")
        chart_versions[str(path.relative_to(ROOT))] = (chart_version, app_version)
        if chart_version != rust_version or app_version != rust_version:
            fail(
                f"Rust is {rust_version}, but {path.relative_to(ROOT)} has "
                f"version {chart_version!r} and appVersion {app_version!r}"
            )

    if args.tag:
        expected_tag = f"v{rust_version}"
        if args.tag != expected_tag:
            fail(f"tag is {args.tag}, expected {expected_tag}")

    print(
        f"baukit release train {rust_version} is coherent "
        f"({len(crate_versions)} crates, CLI, {len(package_versions)} packages, "
        f"{len(chart_versions)} charts)"
    )


if __name__ == "__main__":
    main()
