#!/usr/bin/env python3
"""Assert that every component belongs to the same baukit release train."""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def fail(message: str) -> None:
    print(f"version coherence error: {message}", file=sys.stderr)
    raise SystemExit(1)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--tag",
        help="also require a tag of the form baukit-vX.Y.Z to match the train",
    )
    args = parser.parse_args()

    with (ROOT / "rust/Cargo.toml").open("rb") as manifest:
        rust_workspace = tomllib.load(manifest)["workspace"]
    rust_version = rust_workspace["package"]["version"]

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
        version = package["version"]
        if isinstance(version, dict) and version.get("workspace") is True:
            version = rust_version
        if not isinstance(version, str):
            fail(f"cannot resolve the version in {path.relative_to(ROOT)}")
        crate_versions[package["name"]] = version

    bad_crates = {name: version for name, version in crate_versions.items() if version != rust_version}
    if bad_crates:
        fail(f"Rust crates differ from workspace version {rust_version}: {bad_crates}")

    package_versions: dict[str, str] = {}
    for path in sorted((ROOT / "typescript/packages").glob("*/package.json")):
        package = json.loads(path.read_text())
        name = package.get("name", "")
        if name.startswith("@baukit/"):
            if package.get("private") is not True:
                fail(f"{name} must remain private until the go-public decision")
            package_versions[name] = package["version"]

    if not package_versions:
        fail("no @baukit/* TypeScript packages were found")
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

    if args.tag:
        expected_tag = f"baukit-v{rust_version}"
        if args.tag != expected_tag:
            fail(f"tag is {args.tag}, expected {expected_tag}")

    print(
        f"baukit release train {rust_version} is coherent "
        f"({len(crate_versions)} crates, {len(package_versions)} packages)"
    )


if __name__ == "__main__":
    main()
